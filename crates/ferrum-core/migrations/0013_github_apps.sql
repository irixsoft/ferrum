CREATE TABLE github_apps (
    app_id INTEGER PRIMARY KEY,
    app_slug TEXT NOT NULL,
    app_name TEXT NOT NULL,
    account TEXT NOT NULL UNIQUE COLLATE NOCASE,
    account_type TEXT NOT NULL CHECK (account_type IN ('user', 'organization')),
    private_key TEXT NOT NULL,
    webhook_secret TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret TEXT NOT NULL,
    installation_id INTEGER,
    connected_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO github_apps (app_id, app_slug, app_name, account, account_type, private_key,
                         webhook_secret, client_id, client_secret, installation_id, connected_at)
SELECT app_id, app_slug, app_name, account, 'user', private_key,
       webhook_secret, client_id, client_secret, installation_id, connected_at
FROM github_app;

DROP TABLE github_app;
