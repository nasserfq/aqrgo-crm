use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use jobs::{JobState, Jobs};

async fn queue() -> Jobs {
    let pool = testkit::fresh_pool().await;
    common::db::migrate_module(&pool, jobs::SCHEMA, &jobs::MIGRATOR)
        .await
        .unwrap();
    Jobs::new(pool)
}

#[tokio::test]
async fn a_queued_job_runs_exactly_once_with_its_payload() {
    let q = queue().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let worker = q.worker().register("greet", move |p| {
        let s = s.clone();
        async move {
            s.lock()
                .unwrap()
                .push(p["name"].as_str().unwrap().to_string());
            Ok(())
        }
    });
    let id = q
        .enqueue("greet", &serde_json::json!({ "name": "ناصر" }))
        .await
        .unwrap();

    assert!(worker.run_one().await.unwrap());
    assert!(
        !worker.run_one().await.unwrap(),
        "queue should now be empty"
    );
    assert_eq!(*seen.lock().unwrap(), vec!["ناصر".to_string()]);
    assert_eq!(q.state(id).await.unwrap(), Some(JobState::Done));
}

#[tokio::test]
async fn a_failed_job_is_retried_later_not_immediately() {
    let q = queue().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let worker = q.worker().register("flaky", move |_| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err("SES down".to_string())
        }
    });
    let id = q.enqueue("flaky", &serde_json::json!({})).await.unwrap();

    assert!(worker.run_one().await.unwrap());
    assert!(
        !worker.run_one().await.unwrap(),
        "retry must wait for its backoff"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(q.state(id).await.unwrap(), Some(JobState::Queued));
}

#[tokio::test]
async fn a_job_gives_up_after_its_last_attempt() {
    let q = queue().await;
    let worker = q
        .worker()
        .register("doomed", |_| async { Err("nope".to_string()) });
    let id = q
        .enqueue_with_attempts("doomed", &serde_json::json!({}), 1)
        .await
        .unwrap();

    assert!(worker.run_one().await.unwrap());
    assert_eq!(q.state(id).await.unwrap(), Some(JobState::Failed));
}

#[tokio::test]
async fn unknown_job_kinds_fail_instead_of_vanishing() {
    let q = queue().await;
    let worker = q.worker();
    let id = q
        .enqueue_with_attempts("mystery", &serde_json::json!({}), 1)
        .await
        .unwrap();
    assert!(worker.run_one().await.unwrap());
    assert_eq!(q.state(id).await.unwrap(), Some(JobState::Failed));
}
