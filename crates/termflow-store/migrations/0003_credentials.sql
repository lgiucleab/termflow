-- 0003: provider credentials (secrets live in the OS keychain, never here) and agent session model choice.
CREATE TABLE credentials (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    label       TEXT NOT NULL,
    base_url    TEXT NOT NULL,
    wire        TEXT NOT NULL,
    has_secret  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
ALTER TABLE sessions ADD COLUMN credential_id TEXT REFERENCES credentials(id) ON DELETE SET NULL;
ALTER TABLE sessions ADD COLUMN reasoning INTEGER NOT NULL DEFAULT 1;
CREATE INDEX turns_session ON turns(session_id, seq);
