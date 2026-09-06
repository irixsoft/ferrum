CREATE TABLE github_app (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    app_id INTEGER NOT NULL,
    app_slug TEXT NOT NULL,
    app_name TEXT NOT NULL,
    account TEXT NOT NULL,
    private_key TEXT NOT NULL,
    webhook_secret TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret TEXT NOT NULL,
    installation_id INTEGER,
    connected_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE github_state (
    hash TEXT PRIMARY KEY NOT NULL,
    used_at TEXT,
    expires_at TEXT NOT NULL
);

CREATE TABLE github_deliveries (
    id TEXT PRIMARY KEY NOT NULL,
    event TEXT NOT NULL,
    repository TEXT NOT NULL,
    git_ref TEXT,
    commit_sha TEXT,
    payload TEXT NOT NULL,
    received_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX github_deliveries_repo ON github_deliveries(repository, received_at);
