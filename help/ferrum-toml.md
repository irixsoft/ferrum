# ferrum.toml

A file at the root of your repository (or of the root directory you set for the app) that tells Ferrum what the app is: its processes, the paths they answer, its build and migration commands, the Ubuntu packages it needs, the variables it reads, and the names it reads its database and Redis addresses under. It is the only file Ferrum reads for this; there is no Aptfile, Procfile or example-file scanning.

Ferrum reads it when you create the app, and again on **every deploy** while "Follow the repo's file" is on, right after the clone, so the tag's packages are installed and its commands used for the very deploy that brings them. A tag that changes the file changes the app; rolling back to an older tag brings that tag's file back with it.

```toml
runtime = "bun"                       # node, bun or dotnet
version = "1.2.3"                     # a full version; a channel like "10.0" for .NET
install = "bun install --frozen-lockfile"
build = "bun run --filter web build"
migrate = "bun run db:migrate"        # runs once per deploy, before any process restarts
packages = ["ffmpeg", "libvips42"]    # Ubuntu packages, installed before the build

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

[env]
required = ["SESSION_SECRET", "SMTP_HOST"]   # names your code reads; the panel holds the values

[env.UPLOADS_DIR]                     # a table when there is something to say about a key
about = "Where uploaded files are kept; must survive a deploy"
default = "{{shared}}/uploads"        # written once if you set nothing; {{shared}} is the app's shared directory

[env.SMTP_USER]                       # the app runs without it: shown, never refuses a deploy
about = "Leave empty for a relay without a login"
optional = true
```

## What each key does

- `runtime`, `version`: the toolchain the app builds and runs with. Toolchains are private to Ferrum, never on the system PATH.
- `install`, `build`, `migrate`: run as the app's own user, through `sh -c`, in the release directory. A key you leave out keeps what the panel has.
- `packages`: Ubuntu packages, one name each (`^[a-z0-9][a-z0-9+._-]*$`), installed before the build on every deploy. A package dropped from the list is kept and the app's page says so, with an Uninstall button; a deploy never removes anything. A key left out keeps the panel's list.
- `[env]`: the variables your code reads. Each key goes in one place: a bare name in `required`, or a table `[env.NAME]` when it has a sentence (`about`, shown on the Environment tab), a non-secret `default`, or `optional = true`. A table alone already makes its key required; you do not list it in `required` as well.
  - **A required key with no value and no default refuses the deploy before anything is built**, naming the key.
  - `optional = true` is for a key the app runs without: it is shown with its sentence and never holds a deploy back.
  - A default is written the first time nothing is set, never over a value you typed; `{{shared}}` in it becomes `/var/lib/ferrum/apps/<slug>/shared`.
  - Names Ferrum sets itself cannot be declared: `PORT`, `HOST`, `<NAME>_PORT` of a process this file gives a port, and the labels this file names. Any other name is yours, `SMTP_PORT` included.
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

Domains, environment values, memory and CPU limits, the startup budget and which database is linked belong to the server and live in the panel. A process's memory limit is kept when the file changes.

## When a tag drops a process

If a served name or a path still points at a process the new file no longer names, the deploy is refused before anything is switched, the old version keeps running, and you are notified. Remove or repoint the name first, then push the tag again.

## Turning it off

On the app's Configuration tab, switch "Follow the repo's file" off. The panel becomes the source and the file is ignored, which is the right move when you deploy a repository you do not own.
