-- Runs with search_path = whatsapp.
-- Meta may deliver the same message more than once; each is handled once.
CREATE TABLE seen_messages (
    id      TEXT PRIMARY KEY,           -- WhatsApp message id (wamid...)
    seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
