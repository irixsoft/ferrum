# Environment variables

Every process of an app reads the same environment, written by Ferrum to `shared/.env` as the app's own user, readable by nobody else.

## What you set

Values under the app's Environment tab. Ferrum never reads values from the repository: a real `.env` is not in git, and should not be.

## What the repository tells Ferrum

- `.env.example` (also `.env.sample`, `.env.template`) and code that reads `process.env.X`, `Bun.env.X` or `Environment.GetEnvironmentVariable("X")` tell Ferrum which names your app needs. Missing ones are listed on the Environment tab and in the deploy log.
- A build or start that fails naming a missing variable is reported with that name.

## What Ferrum sets itself

- `PORT`: set per process, for the process that owns the port.
- `<NAME>_PORT`: every process with a port, for its siblings. `WEB_PORT`, `REALTIME_PORT`.
- `HOST`: always `127.0.0.1`. Apps listen on loopback; nginx faces the world.
- The database and Redis addresses, under the labels your `ferrum.toml` names, or `DATABASE_URL` and `REDIS_URL` when it names none. See *Databases and roles*.
- `NODE_ENV=production` for Node and Bun; `ASPNETCORE_URLS` and `DOTNET_ROOT` for .NET.

A value you set under one of these names is ignored; Ferrum's wins.

## Build-time public keys

`NEXT_PUBLIC_*` and `VITE_*` values are baked in at build time. Set them before the deploy that needs them; changing them later needs another deploy.
