-- 0002: block index within its terminal (matches UI-side marks) + lookup index.
ALTER TABLE blocks ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;
CREATE INDEX blocks_finished ON blocks(finished_at);
