//! Sites module: each user's public one-page website (company profile and
//! their available listings).
//!
//! Public API: [`SitesService`], [`SiteInput`], [`Site`], [`PublicSite`],
//! [`PublicListing`] and [`web`]. Owns the `sites` Postgres schema. Reads
//! listings only through `listings::ListingsService`.

mod model;
mod service;
pub mod web;

pub use model::{PublicListing, PublicSite, Site, SiteInput, DEFAULT_ACCENT};
pub use service::{SitesError, SitesService};

pub const SCHEMA: &str = "sites";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
