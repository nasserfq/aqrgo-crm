-- Runs with search_path = listings (see common::db::migrate_module).
CREATE TABLE listings (
    id               BIGSERIAL PRIMARY KEY,
    owner_id         BIGINT NOT NULL,           -- auth user id; no cross-module FK on purpose
    ref_no           INT NOT NULL,              -- م: the owner's own running number 1, 2, 3...
    offer_type       TEXT NOT NULL DEFAULT '',  -- نوع العرض
    plan_no          TEXT NOT NULL DEFAULT '',  -- رقم المخطط
    plot_no          TEXT NOT NULL DEFAULT '',  -- رقم القطعة
    area             TEXT NOT NULL DEFAULT '',  -- المساحة
    frontages        TEXT NOT NULL DEFAULT '',  -- الواجهات والشوارع
    location         TEXT NOT NULL DEFAULT '',  -- الموقع
    price            TEXT NOT NULL DEFAULT '',  -- السعر
    buyer            TEXT NOT NULL DEFAULT '',  -- بيانات المشتري
    seller           TEXT NOT NULL DEFAULT '',  -- بيانات البائع
    broker           TEXT NOT NULL DEFAULT '',  -- بيانات الوسيط
    listed_on        DATE NOT NULL,             -- اليوم والتاريخ
    status           TEXT NOT NULL DEFAULT 'available'
                     CHECK (status IN ('available', 'reserved', 'sold')),
    source           TEXT NOT NULL DEFAULT 'web' CHECK (source IN ('web', 'telegram')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_id, ref_no)
);

CREATE INDEX listings_owner_idx ON listings (owner_id, listed_on DESC, id DESC);
