CREATE TABLE app_processes (
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (name GLOB '[a-z]*' AND name NOT GLOB '*[^a-z0-9_]*' AND length(name) <= 16),
    start TEXT,
    dir TEXT NOT NULL DEFAULT '',
    has_port INTEGER NOT NULL DEFAULT 1,
    health_path TEXT,
    static_dir TEXT,
    memory_mb INTEGER NOT NULL DEFAULT 512,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (app_id, name),
    CHECK ((start IS NULL) <> (static_dir IS NULL))
);

ALTER TABLE apps ADD COLUMN follow_repo_file INTEGER NOT NULL DEFAULT 0;
ALTER TABLE app_routes ADD COLUMN process TEXT NOT NULL DEFAULT 'web';

INSERT INTO app_processes (app_id, name, start, dir, has_port, health_path, static_dir, memory_mb)
SELECT id,
       'web',
       CASE WHEN runtime = 'static' THEN NULL ELSE COALESCE(start_cmd, '') END,
       '',
       CASE WHEN runtime = 'static' THEN 0 ELSE 1 END,
       CASE WHEN runtime = 'static' THEN NULL ELSE health_path END,
       CASE WHEN runtime = 'static' THEN COALESCE(output_dir, 'dist') ELSE NULL END,
       memory_mb
FROM apps;

UPDATE app_ports SET name = 'web' WHERE name = 'main';
UPDATE app_routes SET process = 'web' WHERE port_name = 'main';

INSERT INTO events (id, kind, app_id, subject, sentence, link)
SELECT lower(hex(randomblob(16))),
       'port_removed',
       r.app_id,
       r.path,
       'The route ' || r.path || ' to ' || upper(r.port_name) || '_PORT was removed: routes now point at processes. Run that server as its own process, or on the same port as the rest.',
       '/apps/' || a.slug || '?tab=configuration'
FROM app_routes r
JOIN apps a ON a.id = r.app_id
WHERE r.port_name <> 'main';

DELETE FROM app_routes WHERE port_name <> 'main';
DELETE FROM app_ports WHERE name NOT IN ('web', 'redis');

UPDATE apps SET runtime = toolchain WHERE runtime = 'static';
