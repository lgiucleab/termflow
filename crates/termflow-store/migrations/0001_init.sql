-- 0001: initial schema (BUILD_PROMPT §13). Later milestones add health, rate_limits, budgets, snapshots, catalog_models, permissions_log.

CREATE TABLE workspaces (
    id          TEXT PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    trusted     INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

CREATE TABLE sessions (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    title          TEXT,
    kind           TEXT NOT NULL CHECK (kind IN ('agent', 'terminal', 'external')),
    profile_id     TEXT,
    pinned_model   TEXT,
    mode           TEXT,
    worktree_path  TEXT,
    status         TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE TABLE turns (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role             TEXT NOT NULL,
    status           TEXT NOT NULL,
    served_provider  TEXT,
    served_model     TEXT,
    credential_id    TEXT,
    attempt_id       TEXT,
    created_at       TEXT NOT NULL,
    seq              INTEGER NOT NULL
);
CREATE UNIQUE INDEX turns_session_seq ON turns(session_id, seq);

CREATE TABLE parts (
    id       TEXT PRIMARY KEY,
    turn_id  TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    idx      INTEGER NOT NULL,
    kind     TEXT NOT NULL,
    json     TEXT NOT NULL
);
CREATE UNIQUE INDEX parts_turn_idx ON parts(turn_id, idx);

CREATE TABLE blobs (
    id          TEXT PRIMARY KEY,
    sha256      TEXT NOT NULL UNIQUE,
    media_type  TEXT NOT NULL,
    bytes       BLOB NOT NULL
);

CREATE TABLE tool_calls (
    call_id      TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id      TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    args_json    TEXT NOT NULL,
    status       TEXT NOT NULL,
    started_at   TEXT,
    finished_at  TEXT,
    exec_count   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE id_map (
    call_id    TEXT NOT NULL REFERENCES tool_calls(call_id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    native_id  TEXT NOT NULL,
    PRIMARY KEY (call_id, provider)
);

CREATE TABLE attempts (
    id                  TEXT PRIMARY KEY,
    session_id          TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id             TEXT,
    provider            TEXT NOT NULL,
    model               TEXT NOT NULL,
    credential_id       TEXT,
    started_at          TEXT NOT NULL,
    ended_at            TEXT,
    http_status         INTEGER,
    error_class         TEXT,
    retry_no            INTEGER NOT NULL DEFAULT 0,
    ttft_ms             INTEGER,
    input_tokens        INTEGER,
    output_tokens       INTEGER,
    cache_write_tokens  INTEGER,
    cache_read_tokens   INTEGER,
    reasoning_tokens    INTEGER,
    cost_usd            REAL
);
CREATE INDEX attempts_session ON attempts(session_id);

CREATE TABLE costs (
    id             TEXT PRIMARY KEY,
    session_id     TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    attempt_id     TEXT REFERENCES attempts(id) ON DELETE SET NULL,
    provider       TEXT NOT NULL,
    model          TEXT NOT NULL,
    credential_id  TEXT,
    cost_usd       REAL NOT NULL,
    at             TEXT NOT NULL
);

CREATE TABLE blocks (
    id                   TEXT PRIMARY KEY,
    terminal_session_id  TEXT NOT NULL,
    command              TEXT,
    cwd                  TEXT,
    git_branch           TEXT,
    shell                TEXT,
    exit_code            INTEGER,
    started_at           TEXT,
    finished_at          TEXT,
    output_excerpt       TEXT
);

CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value_json  TEXT NOT NULL
);
