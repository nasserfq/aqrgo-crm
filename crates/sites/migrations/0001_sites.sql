-- Runs with search_path = sites (see common::db::migrate_module).
-- One public website per user.
CREATE TABLE sites (
    owner_id      BIGINT PRIMARY KEY,         -- auth user id; no cross-module FK on purpose
    slug          TEXT NOT NULL UNIQUE,       -- the address: <slug>.aqrgo.com
    published     BOOLEAN NOT NULL DEFAULT false,
    company_name  TEXT NOT NULL DEFAULT '',
    tagline       TEXT NOT NULL DEFAULT '',
    about         TEXT NOT NULL DEFAULT '',
    services      TEXT NOT NULL DEFAULT '',   -- one per line
    license_no    TEXT NOT NULL DEFAULT '',   -- رخصة فال
    phone         TEXT NOT NULL DEFAULT '',
    whatsapp      TEXT NOT NULL DEFAULT '',
    email         TEXT NOT NULL DEFAULT '',
    address       TEXT NOT NULL DEFAULT '',
    map_url       TEXT NOT NULL DEFAULT '',
    accent        TEXT NOT NULL DEFAULT '#6c4fd8',
    logo          BYTEA,
    logo_type     TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Listings the owner chose to keep off their site.
CREATE TABLE hidden_listings (
    owner_id    BIGINT NOT NULL,
    listing_id  BIGINT NOT NULL,
    PRIMARY KEY (owner_id, listing_id)
);
