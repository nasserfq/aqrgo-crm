use sqlx::PgPool;

use crate::model::{Listing, ListingInput, Source, Status};

#[derive(Debug, thiserror::Error)]
pub enum ListingsError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub q: Option<String>,
    pub status: Option<Status>,
}

const COLUMNS: &str = "id, ref_no, offer_type, plan_no, plot_no, area, frontages, location, price, buyer, seller, broker, \
                       listed_on, status, source";

/// The only way other modules read or write listings.
#[derive(Clone)]
pub struct ListingsService {
    pool: PgPool,
}

impl ListingsService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, owner: i64, filter: &Filter) -> Result<Vec<Listing>, ListingsError> {
        let q = filter
            .q
            .as_deref()
            .map(|s| common::digits::to_ascii_digits(s.trim()))
            .filter(|s| !s.is_empty())
            .map(|s| {
                format!(
                    "%{}%",
                    s.replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                )
            });
        let sql = format!(
            "SELECT {COLUMNS} FROM listings.listings
             WHERE owner_id = $3
               AND ($1::text IS NULL OR concat_ws(' ', ref_no::text, offer_type, plan_no, plot_no, area, frontages,
                    location, price, buyer, seller, broker) ILIKE $1)
               AND ($2::text IS NULL OR status = $2)
             ORDER BY listed_on DESC, id DESC
             LIMIT 2000"
        );
        Ok(sqlx::query_as::<_, Listing>(&sql)
            .bind(q)
            .bind(filter.status.map(Status::as_str))
            .bind(owner)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn recent(&self, owner: i64, limit: i64) -> Result<Vec<Listing>, ListingsError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM listings.listings WHERE owner_id = $2 ORDER BY id DESC LIMIT $1"
        );
        Ok(sqlx::query_as::<_, Listing>(&sql)
            .bind(limit)
            .bind(owner)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get(&self, owner: i64, id: i64) -> Result<Option<Listing>, ListingsError> {
        let sql =
            format!("SELECT {COLUMNS} FROM listings.listings WHERE id = $1 AND owner_id = $2");
        Ok(sqlx::query_as::<_, Listing>(&sql)
            .bind(id)
            .bind(owner)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn create(
        &self,
        owner: i64,
        input: ListingInput,
        source: Source,
    ) -> Result<Listing, ListingsError> {
        let i = input.normalized().map_err(ListingsError::Invalid)?;
        let sql = format!(
            "INSERT INTO listings.listings (offer_type, plan_no, plot_no, area, frontages, location, price, buyer,
                seller, broker, listed_on, status, owner_id, source, ref_no)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,
                     (SELECT COALESCE(MAX(ref_no), 0) + 1 FROM listings.listings WHERE owner_id = $13))
             RETURNING {COLUMNS}"
        );
        // Serialise inserts per owner so two saves at once (web + Telegram)
        // never pick the same running number.
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('listings.ref_no'), $1::int)")
            .bind((owner % i32::MAX as i64) as i32)
            .execute(&mut *tx)
            .await?;
        let listing = sqlx::query_as::<_, Listing>(&sql)
            .bind(&i.offer_type)
            .bind(&i.plan_no)
            .bind(&i.plot_no)
            .bind(&i.area)
            .bind(&i.frontages)
            .bind(&i.location)
            .bind(&i.price)
            .bind(&i.buyer)
            .bind(&i.seller)
            .bind(&i.broker)
            .bind(i.listed_on.unwrap_or_else(common::dates::today_riyadh))
            .bind(i.status.as_str())
            .bind(owner)
            .bind(source.as_str())
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(listing)
    }

    pub async fn update(
        &self,
        owner: i64,
        id: i64,
        input: ListingInput,
    ) -> Result<Option<Listing>, ListingsError> {
        let i = input.normalized().map_err(ListingsError::Invalid)?;
        let sql = format!(
            "UPDATE listings.listings SET offer_type=$2, plan_no=$3, plot_no=$4, area=$5, frontages=$6,
                location=$7, price=$8, buyer=$9, seller=$10, broker=$11, listed_on=$12, status=$13, updated_at=now()
             WHERE id = $1 AND owner_id = $14
             RETURNING {COLUMNS}"
        );
        Ok(sqlx::query_as::<_, Listing>(&sql)
            .bind(id)
            .bind(&i.offer_type)
            .bind(&i.plan_no)
            .bind(&i.plot_no)
            .bind(&i.area)
            .bind(&i.frontages)
            .bind(&i.location)
            .bind(&i.price)
            .bind(&i.buyer)
            .bind(&i.seller)
            .bind(&i.broker)
            .bind(i.listed_on.unwrap_or_else(common::dates::today_riyadh))
            .bind(i.status.as_str())
            .bind(owner)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn delete(&self, owner: i64, id: i64) -> Result<bool, ListingsError> {
        let r = sqlx::query("DELETE FROM listings.listings WHERE id = $1 AND owner_id = $2")
            .bind(id)
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
