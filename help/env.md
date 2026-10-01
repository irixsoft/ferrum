# Environment variables

Every process of an app reads the same environment, written by Ferrum to `shared/.env` as the app's own user, readable by nobody else.

## What you set

Values under the app's Environment tab. Ferrum never reads values from the repository: a real `.env` is not in git, and should not be.

## What the repository tells Ferrum

The `[env]` section of `ferrum.toml` names the variables your code reads, so the Environment tab can ask for exactly those; Ferrum does not guess from example files or from the code.

```toml
[env]
required = ["SESSION_SECRET", "SMTP_HOST"]

[env.UPLOADS_DIR]
about = "Where uploaded files are kept; must survive a deploy"
default = "{{shared}}/uploads"
```

- A bare name in `required` is shown as missing until you set it. A deploy of a tag whose file requires a name that has no value and no default is refused before anything is built.
- `[env.NAME]` adds a sentence (`about`) the tab shows beside the field, and a non-secret `default` Ferrum writes the first time nothing is set. A value you typed is never overwritten.
- `{{shared}}` in a default stands for the app's shared directory, `/var/lib/ferrum/apps/<slug>/shared`, the place for uploads and anything that must survive a deploy; the app's Overview shows that path.
- A build or start that fails naming a missing variable is reported with that name, and the name is listed as missing.

## What Ferrum sets itself

- `PORT`: set per process, for the process that owns the port.
- `<NAME>_PORT`: every process with a port, for its siblings. `WEB_PORT`, `REALTIME_PORT`.
- `HOST`: always `127.0.0.1`. Apps listen on loopback; nginx faces the world.
- The database and Redis addresses, under the labels your `ferrum.toml` names, or `DATABASE_URL` and `REDIS_URL` when it names none. See *Databases and roles*.
- `NODE_ENV=production` for Node and Bun; `ASPNETCORE_URLS` and `DOTNET_ROOT` for .NET.

A value you set under one of these names is ignored; Ferrum's wins.

## Build-time public keys

`NEXT_PUBLIC_*` and `VITE_*` values are baked in at build time. Set them before the deploy that needs them; changing them later needs another deploy.
