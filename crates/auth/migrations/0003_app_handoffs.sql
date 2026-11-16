-- Runs with search_path = auth.
-- One-time codes that hand a Google sign-in from the browser to the phone app
-- (see src/handoff.rs). Kept in the database so a restart or deploy between
-- Google and the app's redeem doesn't lose them. Rows live two minutes.
CREATE TABLE app_handoffs (
    code_hash  TEXT PRIMARY KEY,
    session    TEXT NOT NULL,
    challenge  TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
