//! Listings module: the property table (one row per property).
//!
//! Public API used by other modules: [`ListingsService`], [`ListingInput`],
//! [`Listing`], [`Status`], [`Source`], [`web::router`] and [`api::router`] (JSON for the mobile apps).
//! Owns the `listings` Postgres schema. Every row belongs to one user
//! (`owner_id`), and every query is scoped to that owner.

pub mod api;
mod model;
mod service;
pub mod web;

pub use model::{
    Listing, ListingInput, Source, Status, BUILDING_FIELDS, FIELDS, OFFER_TYPE_SUGGESTIONS,
};
pub use service::{Filter, ListingsError, ListingsService};

pub const SCHEMA: &str = "listings";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
