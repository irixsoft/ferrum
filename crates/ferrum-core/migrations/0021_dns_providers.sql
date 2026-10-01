CREATE TABLE dns_providers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('cloudflare', 'route53')),
    credentials TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
