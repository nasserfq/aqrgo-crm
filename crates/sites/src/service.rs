use listings::{Filter, ListingsService, Status};
use sqlx::PgPool;

use crate::model::{logo_type, PublicListing, PublicSite, Site, SiteInput, MAX_LOGO_BYTES};

#[derive(Debug, thiserror::Error)]
pub enum SitesError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Listings(#[from] listings::ListingsError),
}

const COLUMNS: &str =
    "owner_id, slug, published, company_name, tagline, about, services, license_no, phone, \
                       whatsapp, email, address, map_url, accent, logo IS NOT NULL AS has_logo";

/// The only way other modules read or write sites.
#[derive(Clone)]
pub struct SitesService {
    pool: PgPool,
    listings: ListingsService,
}

impl SitesService {
    pub fn new(pool: PgPool, listings: ListingsService) -> Self {
        Self { pool, listings }
    }

    pub async fn save(&self, owner: i64, input: SiteInput) -> Result<Site, SitesError> {
        let sql = format!(
            "INSERT INTO sites.sites (owner_id, slug, published, company_name, tagline, about, services,
                license_no, phone, whatsapp, email, address, map_url, accent)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
             ON CONFLICT (owner_id) DO UPDATE SET slug=$2, published=$3, company_name=$4, tagline=$5,
                about=$6, services=$7, license_no=$8, phone=$9, whatsapp=$10, email=$11, address=$12,
                map_url=$13, accent=$14, updated_at=now()
             RETURNING {COLUMNS}"
        );
        let i = input.normalized().map_err(SitesError::Invalid)?;
        let mut tx = self.pool.begin().await?;
        let site = sqlx::query_as::<_, Site>(&sql)
            .bind(owner)
            .bind(&i.slug)
            .bind(i.published)
            .bind(&i.company_name)
            .bind(&i.tagline)
            .bind(&i.about)
            .bind(&i.services)
            .bind(&i.license_no)
            .bind(&i.phone)
            .bind(&i.whatsapp)
            .bind(&i.email)
            .bind(&i.address)
            .bind(&i.map_url)
            .bind(&i.accent)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match &e {
                sqlx::Error::Database(d) if d.is_unique_violation() => {
                    SitesError::Invalid("هذا العنوان مستخدم لموقع آخر، اختر عنواناً غيره")
                }
                _ => e.into(),
            })?;
        sqlx::query("DELETE FROM sites.hidden_listings WHERE owner_id = $1")
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO sites.hidden_listings (owner_id, listing_id)
             SELECT $1, unnest($2::bigint[]) ON CONFLICT DO NOTHING",
        )
        .bind(owner)
        .bind(&i.hidden_listings)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(site)
    }

    /// The owner's site settings, if they started one.
    pub async fn get(&self, owner: i64) -> Result<Option<Site>, SitesError> {
        let sql = format!("SELECT {COLUMNS} FROM sites.sites WHERE owner_id = $1");
        Ok(sqlx::query_as::<_, Site>(&sql)
            .bind(owner)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Listing ids the owner keeps off their site, in ascending order.
    pub async fn hidden_listings(&self, owner: i64) -> Result<Vec<i64>, SitesError> {
        Ok(sqlx::query_scalar(
            "SELECT listing_id FROM sites.hidden_listings WHERE owner_id = $1 ORDER BY listing_id",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A published site and the listings it shows, or `None`.
    pub async fn public(&self, slug: &str) -> Result<Option<PublicSite>, SitesError> {
        let sql = format!("SELECT {COLUMNS} FROM sites.sites WHERE slug = $1 AND published");
        let Some(site) = sqlx::query_as::<_, Site>(&sql)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };
        let filter = Filter {
            q: None,
            status: Some(Status::Available),
        };
        let hidden = self.hidden_listings(site.owner_id).await?;
        let listings = self
            .listings
            .list(site.owner_id, &filter)
            .await?
            .iter()
            .filter(|l| !hidden.contains(&l.id))
            .map(PublicListing::from)
            .collect();
        Ok(Some(PublicSite { site, listings }))
    }

    /// Replaces the logo of the owner's site (PNG, JPEG or WebP, at most 512 KB).
    pub async fn set_logo(&self, owner: i64, bytes: Vec<u8>) -> Result<(), SitesError> {
        if bytes.len() > MAX_LOGO_BYTES {
            return Err(SitesError::Invalid("الشعار أكبر من ٥١٢ كيلوبايت"));
        }
        let content_type = logo_type(&bytes).ok_or(SitesError::Invalid(
            "الشعار يجب أن يكون صورة PNG أو JPG أو WebP",
        ))?;
        let r = sqlx::query(
            "UPDATE sites.sites SET logo = $2, logo_type = $3, updated_at = now() WHERE owner_id = $1",
        )
        .bind(owner)
        .bind(bytes)
        .bind(content_type)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(SitesError::Invalid("احفظ بيانات الموقع أولاً ثم ارفع الشعار"));
        }
        Ok(())
    }

    /// The logo of a published site and its content type.
    pub async fn logo(&self, slug: &str) -> Result<Option<(Vec<u8>, String)>, SitesError> {
        let row: Option<(Option<Vec<u8>>, Option<String>)> =
            sqlx::query_as("SELECT logo, logo_type FROM sites.sites WHERE slug = $1 AND published")
                .bind(slug)
                .fetch_optional(&self.pool)
                .await?;
        Ok(match row {
            Some((Some(bytes), Some(t))) => Some((bytes, t)),
            _ => None,
        })
    }

    /// The published site served at `host` (`<slug>.<base_domain>`, any port).
    pub async fn slug_for_host(
        &self,
        host: &str,
        base_domain: &str,
    ) -> Result<Option<String>, SitesError> {
        let Some(slug) = crate::model::slug_in_host(host, base_domain) else {
            return Ok(None);
        };
        let found: Option<String> =
            sqlx::query_scalar("SELECT slug FROM sites.sites WHERE slug = $1 AND published")
                .bind(&slug)
                .fetch_optional(&self.pool)
                .await?;
        Ok(found)
    }
}
