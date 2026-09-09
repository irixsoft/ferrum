CREATE TABLE command_runs (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    command TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT,
    exit TEXT
);

CREATE TABLE command_logs (
    run_id TEXT NOT NULL REFERENCES command_runs(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    at TEXT NOT NULL DEFAULT (datetime('now')),
    stream TEXT NOT NULL,
    line TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);
