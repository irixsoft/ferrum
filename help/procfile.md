# Procfile

A plain-text file many repositories already ship, one process per line. Ferrum reads it when there is no `ferrum.toml`.

```
web: bun run start
worker: bun run worker
release: bun run db:migrate
```

- `web` gets its own port as `PORT` and answers `/`.
- Every other name is a worker: no port, checked by staying up for ten seconds after it starts.
- `release` is not a process; it becomes the migration command, run once per deploy before any process restarts.

A Procfile cannot name a start folder, a health path, a folder to serve or a URL path. When you need any of those, add a `ferrum.toml`; it takes precedence when both exist.
