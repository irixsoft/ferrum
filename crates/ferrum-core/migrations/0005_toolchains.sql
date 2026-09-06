CREATE TABLE toolchains (
    kind TEXT NOT NULL CHECK (kind IN ('node', 'bun', 'dotnet')),
    version TEXT NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    installed_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (kind, version)
);
