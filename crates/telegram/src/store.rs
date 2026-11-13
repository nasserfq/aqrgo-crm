use listings::ListingInput;
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct Store {
    pool: PgPool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Guided,
    Edit,
}

#[derive(Debug, Clone)]
pub(crate) struct ChatState {
    pub draft_id: i64,
    pub mode: Mode,
    pub field: String,
}

impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_draft(&self, owner: i64, input: &ListingInput) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO telegram.drafts (owner_id, data) VALUES ($1, $2) RETURNING id",
        )
        .bind(owner)
        .bind(sqlx::types::Json(input))
        .fetch_one(&self.pool)
        .await
    }

    pub async fn draft(&self, owner: i64, id: i64) -> Result<Option<ListingInput>, sqlx::Error> {
        let row: Option<(sqlx::types::Json<ListingInput>,)> =
            sqlx::query_as("SELECT data FROM telegram.drafts WHERE id = $1 AND owner_id = $2")
                .bind(id)
                .bind(owner)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0 .0))
    }

    pub async fn update_draft(
        &self,
        owner: i64,
        id: i64,
        input: &ListingInput,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE telegram.drafts SET data = $3 WHERE id = $1 AND owner_id = $2")
            .bind(id)
            .bind(owner)
            .bind(sqlx::types::Json(input))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes the draft and returns it, so only one caller can save it.
    pub async fn take_draft(
        &self,
        owner: i64,
        id: i64,
    ) -> Result<Option<ListingInput>, sqlx::Error> {
        let row: Option<(sqlx::types::Json<ListingInput>,)> = sqlx::query_as(
            "DELETE FROM telegram.drafts WHERE id = $1 AND owner_id = $2 RETURNING data",
        )
        .bind(id)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0 .0))
    }

    pub async fn state(&self, chat_id: i64) -> Result<Option<ChatState>, sqlx::Error> {
        let row: Option<(i64, String, String)> = sqlx::query_as(
            "SELECT draft_id, mode, field FROM telegram.chat_state
             WHERE chat_id = $1 AND updated_at > now() - interval '1 day'",
        )
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(draft_id, mode, field)| ChatState {
            draft_id,
            mode: if mode == "guided" {
                Mode::Guided
            } else {
                Mode::Edit
            },
            field,
        }))
    }

    pub async fn set_state(
        &self,
        chat_id: i64,
        draft_id: i64,
        mode: Mode,
        field: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO telegram.chat_state (chat_id, draft_id, mode, field) VALUES ($1, $2, $3, $4)
             ON CONFLICT (chat_id) DO UPDATE SET draft_id = $2, mode = $3, field = $4, updated_at = now()",
        )
        .bind(chat_id)
        .bind(draft_id)
        .bind(if mode == Mode::Guided { "guided" } else { "edit" })
        .bind(field)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear_state(&self, chat_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM telegram.chat_state WHERE chat_id = $1")
            .bind(chat_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Drafts nobody confirmed within a week are dropped.
    pub async fn cleanup(&self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM telegram.drafts WHERE created_at < now() - interval '7 days'")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
