CREATE TABLE database_extensions_any (
    database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    PRIMARY KEY (database_id, name)
);
INSERT INTO database_extensions_any (database_id, name)
    SELECT database_id, CASE name WHEN 'pgvector' THEN 'vector' ELSE name END
    FROM database_extensions;
DROP TABLE database_extensions;
ALTER TABLE database_extensions_any RENAME TO database_extensions;
