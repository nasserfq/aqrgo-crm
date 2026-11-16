-- Runs with search_path = auth.
-- One WhatsApp number per user, stored as international digits (e.g. 9665XXXXXXXX).
ALTER TABLE users ADD COLUMN whatsapp_id BIGINT UNIQUE;
