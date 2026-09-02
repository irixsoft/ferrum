CREATE TABLE metrics (
    at INTEGER NOT NULL,
    scope TEXT NOT NULL,
    cpu_pct REAL NOT NULL,
    memory_bytes INTEGER NOT NULL,
    memory_peak_bytes INTEGER,
    disk_used_bytes INTEGER,
    net_rx_bytes INTEGER,
    net_tx_bytes INTEGER
);

CREATE INDEX metrics_scope_at ON metrics (scope, at);
