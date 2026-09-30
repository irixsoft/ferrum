CREATE TABLE events (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    app_id TEXT REFERENCES apps(id) ON DELETE SET NULL,
    subject TEXT NOT NULL,
    sentence TEXT NOT NULL,
    link TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    read_at TEXT
);
CREATE INDEX events_unread ON events(read_at, created_at);
