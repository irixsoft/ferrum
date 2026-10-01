CREATE TABLE app_env_required (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    key TEXT NOT NULL CHECK (key GLOB '[A-Za-z_]*' AND key NOT GLOB '*[^A-Za-z0-9_]*'),
    source TEXT NOT NULL,
    about TEXT,
    default_value TEXT,
    PRIMARY KEY (app_id, key)
);

DROP TABLE app_env_hints;
