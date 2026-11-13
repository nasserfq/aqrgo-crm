-- Runs with search_path = telegram.
-- Listing drafts waiting for the user to confirm, edit or cancel.
CREATE TABLE drafts (
    id         BIGSERIAL PRIMARY KEY,
    owner_id   BIGINT NOT NULL,
    data       JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- What the bot is waiting for in each chat (step-by-step flow, or a field edit).
CREATE TABLE chat_state (
    chat_id    BIGINT PRIMARY KEY,
    draft_id   BIGINT NOT NULL REFERENCES drafts (id) ON DELETE CASCADE,
    mode       TEXT NOT NULL CHECK (mode IN ('guided', 'edit')),
    field      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
