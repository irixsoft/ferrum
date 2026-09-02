CREATE TABLE databases (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL,
    connection_limit INTEGER NOT NULL DEFAULT 20,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE database_extensions (
    database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (name IN ('pgvector', 'pg_trgm', 'pgcrypto', 'uuid-ossp')),
    PRIMARY KEY (database_id, name)
);

CREATE TABLE app_databases (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (app_id, database_id)
);

CREATE TABLE redis_instances (
    app_id TEXT PRIMARY KEY NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    password TEXT NOT NULL,
    maxmemory_mb INTEGER NOT NULL DEFAULT 64,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
