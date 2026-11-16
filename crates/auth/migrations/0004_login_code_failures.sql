-- Runs with search_path = auth.
-- Wrong guesses per code, so the daily cap on wrong tries per email can be
-- counted across codes (a new code resets `attempts` but not the day's total).
ALTER TABLE login_codes ADD COLUMN failures INT NOT NULL DEFAULT 0;
