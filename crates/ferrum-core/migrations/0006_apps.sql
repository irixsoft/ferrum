CREATE TABLE apps (
    id TEXT PRIMARY KEY NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    repository TEXT NOT NULL,
    git_ref TEXT NOT NULL,
    tracking TEXT NOT NULL CHECK (tracking IN ('branch', 'releases')),
    root TEXT NOT NULL DEFAULT '',
    runtime TEXT NOT NULL CHECK (runtime IN ('node', 'bun', 'static', 'dotnet')),
    toolchain TEXT NOT NULL CHECK (toolchain IN ('node', 'bun', 'dotnet')),
    runtime_version TEXT NOT NULL,
    install_cmd TEXT,
    build_cmd TEXT,
    start_cmd TEXT,
    migrate_cmd TEXT,
    output_dir TEXT,
    health_path TEXT NOT NULL DEFAULT '/',
    startup_budget_secs INTEGER NOT NULL DEFAULT 60,
    memory_mb INTEGER NOT NULL DEFAULT 512,
    cpu_percent INTEGER NOT NULL DEFAULT 100,
    pause_for_migrations INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE app_ports (
    port INTEGER PRIMARY KEY NOT NULL CHECK (port BETWEEN 20000 AND 29999),
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    UNIQUE (app_id, name)
);

CREATE TABLE app_routes (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    port_name TEXT NOT NULL,
    websocket INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (app_id, path)
);

CREATE TABLE app_env (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    key TEXT NOT NULL CHECK (key GLOB '[A-Za-z_]*' AND key NOT GLOB '*[^A-Za-z0-9_]*'),
    value TEXT NOT NULL,
    PRIMARY KEY (app_id, key)
);

CREATE TABLE app_packages (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    family TEXT NOT NULL DEFAULT 'debian',
    PRIMARY KEY (app_id, name)
);

CREATE TABLE app_domains (
    domain TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0
);
