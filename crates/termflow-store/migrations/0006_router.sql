-- M3 router: credential blocks (quota/auth) survive restarts, sessions can follow a profile,
-- and the model catalog is cached for offline use.
CREATE TABLE route_blocks (
    credential_id TEXT PRIMARY KEY,
    until         TEXT,
    reason        TEXT NOT NULL
);
ALTER TABLE sessions ADD COLUMN profile TEXT;
CREATE TABLE catalog_cache (
    source     TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);
CREATE INDEX attempts_started ON attempts(started_at);
