# ferrum.toml

A file at the root of your repository (or of the root directory you set for the app) that tells Ferrum what the app is: its processes, the paths they answer, its build and migration commands, and the names it reads its database and Redis addresses under.

Ferrum reads it when you create the app, and again on **every deploy** while "Follow the repo's file" is on. A tag that changes the file changes the app; rolling back to an older tag brings that tag's file back with it.

```toml
runtime = "bun"                       # node, bun or dotnet
version = "1.2.3"                     # a full version; a channel like "10.0" for .NET
install = "bun install --frozen-lockfile"
build = "bun run --filter web build"
migrate = "bun run db:migrate"        # runs once per deploy, before any process restarts
packages = ["ffmpeg"]                 # suggested at creation; the Aptfile is the source on deploys

[processes.web]
start = "bun run start"
dir = "apps/web"                      # the folder the command starts in, relative to the app
port = true                           # gets its own port as PORT
health = "/api/healthz"               # what a deploy checks before calling the process healthy

[processes.realtime]
start = "bun run start"
dir = "apps/realtime"
port = true
path = "/live"                        # nginx sends this path to this process on every served name
websocket = true

[processes.jobs]
start = "bun run start"               # no port: a worker
dir = "apps/jobs"

[processes.admin]
static = "apps/admin/dist"            # a folder nginx serves; no program runs
path = "/admin"

[database]
url = "DATABASE_ADMIN_URL"            # the label the owner's address is written under

[database.roles.app]
url = "DATABASE_URL"                  # a restricted role Ferrum creates, and its label

[redis]
url = "CACHE_URL"
```

## What each key does

- `runtime`, `version`: the toolchain the app builds and runs with. Toolchains are private to Ferrum, never on the system PATH.
- `install`, `build`, `migrate`: run as the app's own user, through `sh -c`, in the release directory. A key you leave out keeps what the panel has.
- `[processes.<name>]`: one table per process. A name is lowercase letters, digits and underscores. `web` is the usual name for the process that answers `/`.
  - `start`: the command. Anything the process needs that its siblings don't goes in front of it: `WORKER_MODE=1 bun run start`.
  - `dir`: where the command starts, relative to the app's root. Empty means the root.
  - `port`: `true` for a process visitors reach. `web` and any process with a `health` path listen by default; everything else is a worker.
  - `health`: the path a deploy calls on that port until it answers 2xx or 3xx.
  - `static`: instead of `start`, a folder of built files for nginx to serve.
  - `path`, `websocket`: the URL path nginx routes to this process, on every served name.
- `[database]`, `[database.roles.<name>]`, `[redis]`: the labels your code reads. See *Databases and roles*.
- `start`, `output_dir`, `health_path` at the top level still work and mean one process called `web`.

## What the file never decides

Domains, environment values, memory limits and which database is linked belong to the server and live in the panel. A process's memory limit is kept when the file changes.

## When a tag drops a process

If a served name or a path still points at a process the new file no longer names, the deploy is refused before anything is switched, the old version keeps running, and you are notified. Remove or repoint the name first, then push the tag again.

## Turning it off

On the app's Configuration tab, switch "Follow the repo's file" off. The panel becomes the source and the file is ignored, which is the right move when you deploy a repository you do not own.
