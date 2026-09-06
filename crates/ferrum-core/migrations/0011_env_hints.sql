CREATE TABLE app_env_hints (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    key TEXT NOT NULL CHECK (key GLOB '[A-Za-z_]*' AND key NOT GLOB '*[^A-Za-z0-9_]*'),
    source TEXT NOT NULL,
    optional INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (app_id, key)
);
