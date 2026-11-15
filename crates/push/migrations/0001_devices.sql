-- Runs with search_path = push (see common::db::migrate_module).
-- Phones that asked for notifications. One Expo push token per phone install;
-- a token moves to whoever signed in on that phone last.
CREATE TABLE devices (
    token       TEXT PRIMARY KEY,
    owner_id    BIGINT NOT NULL,              -- auth user id; no cross-module FK on purpose
    platform    TEXT NOT NULL CHECK (platform IN ('ios', 'android')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX devices_owner ON devices (owner_id);
