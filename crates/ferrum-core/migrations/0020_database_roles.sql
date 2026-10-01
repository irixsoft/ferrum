CREATE TABLE database_roles (
    id TEXT PRIMARY KEY NOT NULL,
    database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
    name TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL,
    connection_limit INTEGER NOT NULL DEFAULT 20,
    env_label TEXT NOT NULL,
    owner INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (database_id, env_label)
);

INSERT INTO database_roles (id, database_id, name, password, connection_limit, env_label, owner, created_at)
SELECT lower(hex(randomblob(16))), id, role, password, connection_limit, 'DATABASE_URL', 1, created_at
FROM databases;

ALTER TABLE redis_instances ADD COLUMN env_label TEXT NOT NULL DEFAULT 'REDIS_URL';
ALTER TABLE app_databases ADD COLUMN env_label TEXT;
