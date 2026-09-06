CREATE TABLE cert_attempts (
    domain TEXT PRIMARY KEY NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    next_at TEXT
);
