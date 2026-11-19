-- Runs with search_path = auth.
-- Extra emails that sign in to an account, besides its own. Apple's "Hide My
-- Email" gives each app a relay address that matches no account; linking it
-- here makes Apple sign-in open the user's real account. Checked before
-- users.email, so a linked address wins over an account it once created.
CREATE TABLE sign_in_emails (
    email      TEXT PRIMARY KEY,               -- always lowercase
    user_id    BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sign_in_emails_user_idx ON sign_in_emails (user_id);
