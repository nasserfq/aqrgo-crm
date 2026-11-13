//! Jobs module: a small background job queue stored in Postgres.
//!
//! Other modules call [`Jobs::enqueue`] with a job kind and a JSON payload,
//! and register a handler for that kind on the [`Worker`] at startup.
//! Workers claim jobs with `FOR UPDATE SKIP LOCKED`, so several app
//! instances can run safely. Failed jobs retry with exponential backoff.
//! Payloads are wiped once a job finishes, because they may hold secrets
//! such as login codes. Owns the `jobs` Postgres schema.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::Notify;

pub const SCHEMA: &str = "jobs";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
pub type Handler = Arc<dyn Fn(Value) -> HandlerFuture + Send + Sync>;

/// Handle used to put work on the queue. Cheap to clone.
#[derive(Clone)]
pub struct Jobs {
    pool: PgPool,
    wake: Arc<Notify>,
}

impl Jobs {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            wake: Arc::new(Notify::new()),
        }
    }

    /// Queues a job with the default 8 attempts.
    pub async fn enqueue<P: Serialize>(&self, kind: &str, payload: &P) -> Result<i64, sqlx::Error> {
        self.enqueue_with_attempts(kind, payload, 8).await
    }

    pub async fn enqueue_with_attempts<P: Serialize>(
        &self,
        kind: &str,
        payload: &P,
        max_attempts: i32,
    ) -> Result<i64, sqlx::Error> {
        let payload =
            serde_json::to_value(payload).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO jobs.jobs (kind, payload, max_attempts) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(kind)
        .bind(payload)
        .bind(max_attempts.max(1))
        .fetch_one(&self.pool)
        .await?;
        self.wake.notify_one();
        Ok(id)
    }

    pub async fn state(&self, id: i64) -> Result<Option<JobState>, sqlx::Error> {
        let s: Option<String> = sqlx::query_scalar("SELECT status FROM jobs.jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(s.and_then(|s| JobState::parse(&s)))
    }

    /// Builds a worker that shares this queue's wake-up signal.
    pub fn worker(&self) -> Worker {
        Worker {
            jobs: self.clone(),
            handlers: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

impl JobState {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(sqlx::FromRow)]
struct Claimed {
    id: i64,
    kind: String,
    payload: Value,
    attempts: i32,
    max_attempts: i32,
}

pub struct Worker {
    jobs: Jobs,
    handlers: HashMap<String, Handler>,
}

/// Seconds to wait before retry number `attempt` (1-based): 10s, 20s, 40s… capped at 1h.
pub fn backoff_secs(attempt: i32) -> i64 {
    let exp = attempt.clamp(1, 20) as u32 - 1;
    (10_i64.saturating_mul(2_i64.saturating_pow(exp))).min(3600)
}

impl Worker {
    pub fn register<F, Fut>(mut self, kind: &str, f: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.handlers
            .insert(kind.to_string(), Arc::new(move |v| Box::pin(f(v))));
        self
    }

    /// Runs until the process exits.
    pub async fn run(self) {
        let mut last_housekeeping = tokio::time::Instant::now() - Duration::from_secs(3600);
        loop {
            if last_housekeeping.elapsed() > Duration::from_secs(600) {
                if let Err(e) = self.housekeeping().await {
                    tracing::warn!(error = %e, "jobs housekeeping failed");
                }
                last_housekeeping = tokio::time::Instant::now();
            }
            match self.run_one().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "jobs worker error"),
            }
            tokio::select! {
                _ = self.jobs.wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
        }
    }

    /// Claims and runs one ready job. Returns false when the queue is empty.
    pub async fn run_one(&self) -> Result<bool, sqlx::Error> {
        let claimed: Option<Claimed> = sqlx::query_as(
            "UPDATE jobs.jobs SET status = 'running', locked_at = now(), attempts = attempts + 1, updated_at = now()
             WHERE id = (SELECT id FROM jobs.jobs WHERE status = 'queued' AND run_at <= now()
                         ORDER BY run_at, id FOR UPDATE SKIP LOCKED LIMIT 1)
             RETURNING id, kind, payload, attempts, max_attempts",
        )
        .fetch_optional(&self.jobs.pool)
        .await?;
        let Some(job) = claimed else { return Ok(false) };

        let result = match self.handlers.get(&job.kind) {
            Some(h) => h(job.payload).await,
            None => Err(format!("no handler for job kind '{}'", job.kind)),
        };

        match result {
            Ok(()) => {
                sqlx::query(
                    "UPDATE jobs.jobs SET status = 'done', payload = '{}'::jsonb, last_error = NULL,
                     locked_at = NULL, updated_at = now() WHERE id = $1",
                )
                .bind(job.id)
                .execute(&self.jobs.pool)
                .await?;
            }
            Err(err) => {
                let give_up = job.attempts >= job.max_attempts;
                tracing::warn!(job = job.id, kind = %job.kind, attempt = job.attempts, give_up, error = %err, "job failed");
                sqlx::query(
                    "UPDATE jobs.jobs SET
                        status = CASE WHEN $2 THEN 'failed' ELSE 'queued' END,
                        payload = CASE WHEN $2 THEN '{}'::jsonb ELSE payload END,
                        run_at = now() + make_interval(secs => $3),
                        last_error = $4, locked_at = NULL, updated_at = now()
                     WHERE id = $1",
                )
                .bind(job.id)
                .bind(give_up)
                .bind(backoff_secs(job.attempts) as f64)
                .bind(err)
                .execute(&self.jobs.pool)
                .await?;
            }
        }
        Ok(true)
    }

    /// Re-queues jobs stuck in `running` (e.g. the app crashed mid-job) and
    /// deletes finished jobs older than a week.
    async fn housekeeping(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs.jobs SET status = 'queued', locked_at = NULL, updated_at = now()
             WHERE status = 'running' AND locked_at < now() - interval '15 minutes'",
        )
        .execute(&self.jobs.pool)
        .await?;
        sqlx::query("DELETE FROM jobs.jobs WHERE status IN ('done', 'failed') AND updated_at < now() - interval '7 days'")
            .execute(&self.jobs.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_secs(1), 10);
        assert_eq!(backoff_secs(2), 20);
        assert_eq!(backoff_secs(4), 80);
        assert_eq!(backoff_secs(30), 3600);
    }
}
