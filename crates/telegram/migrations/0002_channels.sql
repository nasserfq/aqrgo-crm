-- Runs with search_path = telegram.
-- The same conversation now also runs on WhatsApp, so chat ids are per channel.
ALTER TABLE chat_state ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram'
    CHECK (channel IN ('telegram', 'whatsapp'));
ALTER TABLE chat_state DROP CONSTRAINT chat_state_pkey;
ALTER TABLE chat_state ADD PRIMARY KEY (channel, chat_id);
