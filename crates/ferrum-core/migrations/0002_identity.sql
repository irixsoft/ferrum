CREATE TABLE users (
    id         TEXT PRIMARY KEY NOT NULL,
    handle     TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE credentials (
    id         TEXT PRIMARY KEY NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label      TEXT,
    credential TEXT NOT NULL,
    counter    INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_used  TEXT
);
CREATE INDEX credentials_user ON credentials(user_id);

CREATE TABLE sessions (
    id         TEXT PRIMARY KEY NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_agent TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    last_seen  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE api_tokens (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    hash       TEXT NOT NULL,
    read_only  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_used  TEXT,
    revoked_at TEXT
);

CREATE TABLE enrollments (
    id         TEXT PRIMARY KEY NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    used_at    TEXT
);
