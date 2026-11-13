-- Runs with search_path = listings.
ALTER TABLE listings DROP CONSTRAINT listings_source_check;
ALTER TABLE listings ADD CONSTRAINT listings_source_check
    CHECK (source IN ('web', 'telegram', 'whatsapp'));
