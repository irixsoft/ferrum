CREATE TABLE releases (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    dir TEXT NOT NULL,
    git_ref TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    commit_message TEXT,
    built_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (app_id, dir)
);

CREATE TABLE deploys (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    trigger TEXT NOT NULL CHECK (trigger IN ('webhook', 'manual', 'cli', 'rollback')),
    git_ref TEXT NOT NULL,
    commit_sha TEXT,
    commit_message TEXT,
    author TEXT,
    state TEXT,
    outcome TEXT CHECK (outcome IN ('live', 'failed', 'rolledback')),
    failure_reason TEXT,
    release_id TEXT REFERENCES releases(id) ON DELETE SET NULL,
    restore_deploy_id TEXT REFERENCES deploys(id) ON DELETE SET NULL,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT
);

CREATE TABLE deploy_steps (
    deploy_id TEXT NOT NULL REFERENCES deploys(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    position INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('done', 'active', 'pending', 'skipped', 'failed')),
    started_at TEXT,
    finished_at TEXT,
    note TEXT,
    PRIMARY KEY (deploy_id, state)
);

CREATE TABLE deploy_logs (
    deploy_id TEXT NOT NULL REFERENCES deploys(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    at TEXT NOT NULL DEFAULT (datetime('now')),
    stream TEXT NOT NULL,
    line TEXT NOT NULL,
    PRIMARY KEY (deploy_id, seq)
);

CREATE TABLE snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
    deploy_id TEXT REFERENCES deploys(id) ON DELETE SET NULL,
    path TEXT NOT NULL,
    taken_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE apps ADD COLUMN current_release_id TEXT REFERENCES releases(id) ON DELETE SET NULL;
