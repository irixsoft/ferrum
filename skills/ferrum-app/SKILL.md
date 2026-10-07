---
name: ferrum-app
description: >
  Make a Bun, Node or .NET service run on Ferrum, the single-binary deploy manager for one
  Ubuntu server: write or change its ferrum.toml, lay out its processes, required variables,
  packages, database labels and health path, or convert an existing app to these conventions.
  Use when an app is created for Ferrum, prepared for its first deploy, gains a process,
  worker, second server, extra hostname, package or database role, or when a deploy on Ferrum
  was refused or failed. Triggers: ferrum, ferrum.toml, "deploy to the server", PORT/HOST,
  systemd unit, worker, realtime process, wildcard subdomains, DATABASE_URL label, restricted
  role, required variables, UPLOADS_DIR. Does not apply to libraries, CLIs, desktop or mobile
  work.
---

# Running an app on Ferrum

Written for Ferrum 0.2.3. Everything an agent needs is in this file.

## How Ferrum runs an app

One Ubuntu server. For each app Ferrum keeps a Linux user, a directory
`/var/lib/ferrum/apps/<slug>/` with `releases/<id>/` (one per deploy), `current` (a symlink
to the live release), `env` (the environment, readable by root only), and `shared/` (survives
deploys and is the only place the app may write: `cache/`, `storage/`, uploads). Every
process is a systemd unit `ferrum-app-<slug>-<process>` started in `current/<root>/<dir>` as the
app's user with the shared environment. nginx faces the network and proxies each name and
path to the right process on `127.0.0.1`; the app itself never listens publicly.

A deploy is a pushed git tag: clone → read `ferrum.toml` → install the packages it lists →
install → build → snapshot the database and run `migrate` once → write units → switch
`current` → start or restart every process → check each one → done, or roll every process
back together. A tag whose file requires a variable that has no value, or drops a process a
domain still points at, is refused before anything is built; the old release keeps running.

Toolchains (Bun, Node, .NET) are private to Ferrum, never on the system PATH; the app runs
on the toolchain and version it was created with unless `ferrum.toml` names one.

## ferrum.toml

The one file Ferrum reads about the app: at the repository root (or at the root directory
set for the app). There is no Aptfile, no Procfile, and no scanning of `.env.example` or the
code. Ferrum reads it when the app is created and again on **every deploy** while the app's
"Follow the repo's file" switch is on. A tag that changes the file changes the app; rolling
back to an older tag brings that tag's file back.

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
bypass_rls = true                     # only when migrations use FORCE ROW LEVEL SECURITY

[database.roles.app]
url = "DATABASE_URL"                  # a restricted role Ferrum creates, and its label

[redis]
url = "CACHE_URL"

[env]
required = ["SESSION_SECRET", "SMTP_HOST"]   # bare names the code must have; values live in the panel

[env.UPLOADS_DIR]                     # a table instead, when there is something to say about a key
about = "Where uploaded files are kept; must survive a deploy"
default = "{{shared}}/uploads"        # written once if nothing is set; {{shared}} is the app's shared directory

[env.SMTP_USER]                       # the app runs without it: shown in the panel, never refuses a deploy
about = "Leave empty for a relay without a login"
optional = true
```

Keys:

- `runtime`, `version`: the toolchain. Leave them out to keep what the panel has. A tag that
  names one not installed on the server is refused; it is installed from the panel's Runtimes
  page first. Node toolchains carry `pnpm` and `yarn` through corepack, which follows
  `packageManager` in `package.json`.
- `install`, `build`, `migrate`: run as the app's own user through `sh -c`, in the release
  directory. A key left out keeps the panel's value. Without a file, Ferrum prefills them at
  creation from `package.json` scripts (`build`, `start`, `db:migrate`/`migrate`) or the
  `.csproj`; the prefill never runs on deploys.
- `packages`: Ubuntu packages, one name each (`^[a-z0-9][a-z0-9+._-]*$`), installed before
  the build on every deploy. A package dropped from the list is kept and the app's page says
  so, with an Uninstall button; a deploy never removes anything. Left out, the panel's list
  stands. Packages are system-wide.
- `[processes.<name>]`: one table per process. A name is lowercase letters, digits and
  underscores. `web` is the usual name for the process that answers `/`.
  - `start`: the command, bare, no port flag. Anything one process needs that its siblings
    don't goes in front of it: `WORKER_MODE=1 bun run start`.
  - `dir`: where the command starts, relative to the app's root. Empty means the root.
  - `port`: `true` for a process visitors reach. `web` and any process with a `health`
    listen by default; everything else is a worker.
  - `health`: any path you choose that the deploy GETs on that port until it answers 2xx or
    3xx, within the app's startup budget (panel, default 60 s). Answer it with a cheap
    handler that does no database work: `"/healthz": () => new Response("ok")` in
    `Bun.serve` routes, or an `app/api/healthz/route.ts` returning `Response.json({ ok:
    true })` in Next. Left out, the deploy asks for `/`, and a full page render (or a redirect
    to a login page) decides whether the process is healthy.
  - `static`: instead of `start`, a folder of built files nginx serves straight from the
    release. A folder process has no unit and no log.
  - `path`, `websocket`: the URL path nginx routes to this process on every served name.
    Only one process can own `/`, and it is the one a served name points at.
- `[database]`, `[database.roles.<name>]`, `[redis]`: the env names the code reads its
  addresses under. See *Databases*.
- `[env]`: the variables the code reads. See *Declaring variables* below; it has rules an
  agent gets wrong.
- Top-level `start`, `output_dir`, `health_path` still work and mean one process `web`.

What the file never decides: domains, environment values, memory and CPU limits, the startup
budget, whether traffic pauses for migrations, and which database is linked. Those belong to
the server and live in the panel.

### Declaring variables

Every variable goes in **exactly one** of two places. Pick per key:

| The key is… | Write it as |
|---|---|
| needed, and its name says enough | a bare name in `required = [...]` |
| needed, and wants a sentence or a non-secret default | a table `[env.KEY]` with `about` and/or `default` — **not** also in `required` |
| something the app runs without | a table `[env.KEY]` with `optional = true` — **not** in `required` |

- **A table alone already makes the key required.** The `required` list is only the short
  form for keys with nothing to say. Never write a key in both; the list is not a "this is
  required" flag that tables need.
- **Required** means: a deploy is refused before the build while the key has no value in the
  panel and no `default` in the file. So only require what the app cannot start or build
  without.
- **Optional** means: the Environment tab shows the key and its sentence, and a deploy never
  waits for it. Use it for anything described as "leave empty unless…", legacy keys, feature
  switches, and anything used only in local development. Never give such a key a fake value
  or a fake default to get past the check, and never leave it out of the file.
- `default` is for non-secret values only. It is written the first time nothing is set and
  never over a value typed in the panel. `{{shared}}` in it becomes
  `/var/lib/ferrum/apps/<slug>/shared`. Secrets never get a default.
- Do not declare what Ferrum sets itself: `PORT`, `HOST`, `<NAME>_PORT` for a process of
  this app that has a port (`WEB_PORT`, `REALTIME_PORT`), and the labels named under
  `[database]`, `[database.roles.*]` and `[redis]`. Any other name is the app's own, including
  ones that end in `_PORT`: `SMTP_PORT` and `DB_PORT` are fine.

Wrong, and how it is fixed:

```toml
# WRONG: ADMIN_EMAIL is in the list and in a table.
[env]
required = ["SESSION_SECRET", "ADMIN_EMAIL"]

[env.ADMIN_EMAIL]
about = "The first admin's login"

# WRONG: the app works without it, so requiring it blocks every deploy.
[env.STRIPE_LEGACY_KEY]
about = "Optional; only for accounts created before 2024"
```

```toml
# RIGHT
[env]
required = ["SESSION_SECRET"]

[env.ADMIN_EMAIL]
about = "The first admin's login"

[env.STRIPE_LEGACY_KEY]
about = "Only for accounts created before 2024"
optional = true
```

Ferrum 0.2.2 tolerates the first mistake (the table wins), but 0.2.1 refuses the deploy, so
write it the right way. A key in `required` whose table says `optional = true` is always
refused.

## Processes

- All processes share the repository, the build, the release directory, the environment, the
  Linux user and `shared/`. One deploy updates all of them; one rollback puts all back.
- Each has its own command, start folder, port (if any), health path, memory limit, unit,
  journal and Restart button.
- A process with a port passes when its health path answers; a worker passes when it is
  still running ten seconds after it started. Any process failing fails the deploy, and
  every process goes back to the previous release.
- With "Pause traffic while migrations run" on (the default when a migrate command exists),
  the maintenance page goes up and every running process is stopped before `migrate`, so no
  worker touches the database mid-change.
- Reaching a sibling: every process with a port is in the shared env as `<NAME>_PORT`
  (`WEB_PORT`, `REALTIME_PORT`); `HOST` is always `127.0.0.1`. Most apps never need this
  because nginx routes paths and names.
- systemd restarts a process that dies between deploys; one that stays down is reported
  once as "broke on its own". SIGTERM is the stop signal; exit cleanly on it.
- Logs: stdout and stderr go to the journal, one per process. No log files in the app.

## Environment

Every process reads the same environment, which Ferrum writes to a root-only file the unit
loads at start. Read variables from the process environment; Ferrum's file is not readable by
the app.

Ferrum sets: `PORT` (per process, for the one that owns it), `<NAME>_PORT` for every port
process, `HOST=127.0.0.1`, the database and Redis addresses under the labels the file names
(`DATABASE_URL` and `REDIS_URL` when it names none), `NODE_ENV=production` for Node and Bun,
`ASPNETCORE_URLS` and `DOTNET_ROOT` for .NET. A value set under one of these names in the
panel is ignored; Ferrum's wins.

Everything else is set in the panel, which asks for exactly the names `[env]` declares.
Ferrum never reads values from the repository and never guesses names from example files or
the code. `NEXT_PUBLIC_*` and `VITE_*` values are baked in at build time: they must be set
before the deploy that needs them.

## Domains and paths

Names are rows in the panel, never in the file. A row either **serves** a process or
**redirects** to another served name of the same app; one row is primary. Several names can
serve the same process — `example.org`, `admin.example.org`, `partner.example.org` all on
`web`, the app choosing what to show from the `Host` header — or different ones:
`example.com` → `site`, `admin.example.com` → `admin`.

A path (`/live` → `realtime`) applies on every served name, on top of the process that
answers `/` there. `*.example.com` is one more row covering every subdomain without its own
row; it needs a DNS provider (Cloudflare or Route 53) set up in the panel, and `example.com`
is a separate row.

nginx passes `Host`, `X-Forwarded-Host`, `X-Forwarded-Proto`, `X-Forwarded-For`. A name
serving a websocket process, and any websocket path, gets a 24-hour read timeout; plain
paths one hour. HSTS is set per exact name. Until a name's certificate is issued it is served
over plain HTTP.

## Databases

PostgreSQL on the same server, loopback only. Each database has an **owner** role, created
with it: it owns every table, runs migrations, and is what `DATABASE_URL` points at unless
the file says otherwise. **Restricted roles** (`[database.roles.<name>]`) are logins that
can connect and nothing else; what they may read or write comes from the app's own
migrations (`GRANT`, row-level security policies). Ferrum names them `<database>_<name>`, so
migrations that grant by name must use that form. They never get superuser or bypass-RLS.

Row-level security never applies to a table's owner unless the migrations say `FORCE ROW
LEVEL SECURITY`; then the owner needs `bypass_rls = true` under `[database]` to see every
row. Only the owner can have it.

Labels are the env names the code reads: one for the owner, one per role, one for Redis
(`REDIS_URL` by default). A second linked database is `<NAME>_DATABASE_URL` by default.
Ferrum rewrites every address on each rotate, so a label never points at a stale password.
A label must be unique across everything the app's env carries.

Roles declared in the file are created when the app is linked to a database or deployed
from a tag that adds one. A role dropped from the file is kept, with a notice.

Migrations run once per deploy via `migrate`, after a snapshot of the database, as the
owner. Schema and policy changes belong there, not in app startup code.

## Converting an existing app

Work through this in order; each line is a grep or a file.

1. **Processes.** List what actually runs in production: web server(s), realtime, workers,
   bots, built folders. Each becomes `[processes.<name>]`. A single-server app is just `web`.
2. **Ports.** `grep -rn "3000\|:300\|listen(\|port:" src` — every listener takes its port
   from `PORT` and binds `HOST`. `Bun.serve` without `port` already reads `PORT`; `next
   start` reads `PORT`; ASP.NET reads `ASPNETCORE_URLS`. Delete `-p 3001` flags from start
   scripts. Internal calls between processes use `127.0.0.1:$<NAME>_PORT`, or better, a path
   routed by nginx.
3. **Dead files.** Remove `ecosystem.config.js`, `nginx.conf`, `Aptfile`, `Procfile`,
   production Dockerfiles and compose files, log-file settings. Keep a dev compose if the
   team uses it locally.
4. **Writable state.** `grep -rn "uploads\|storage\|writeFile\|mkdir" src` — every path
   comes from an env var (`UPLOADS_DIR`, `STORAGE_DIR`) declared in `[env]` with a default
   under `{{shared}}`. The app serves its own uploads (`/uploads/*` route); nginx will not
   serve them from disk.
5. **Health.** Each port process answers a cheap GET (`/healthz`, `/api/healthz`) with 200
   and no database work. Name it in `health`.
6. **Workers.** A worker's main loop runs for the life of the process; it must not exit when
   idle. It stops cleanly on SIGTERM.
7. **Hosts.** If one process answers several hostnames, routing reads `Host` or
   `X-Forwarded-Host`, and absolute URLs come from env (`SITE_URL`, `NEXT_PUBLIC_*`), not
   from the request.
8. **Database.** Migrations in a `migrate` script. If the code uses two connections (admin
   and app), name both labels in `[database]` and `[database.roles.app]`; add
   `bypass_rls = true` only if a migration says `FORCE ROW LEVEL SECURITY`. Grants in
   migrations use the `<database>_<role>` name.
9. **System tools.** Chromium, ffmpeg, libvips → `packages`; their paths via a variable in
   `[env]` with a default (`CHROMIUM_PATH = "/usr/bin/chromium"`).
10. **`[env]`.** Every key the code reads that Ferrum does not set, including build-time
    public keys, each in one place: a bare name in `required`, or a table when it has a
    sentence or a non-secret default. For each key ask "does the app start and build without
    it?" — if yes, it is a table with `optional = true`. Keep `.env.example` for local
    development if you like; Ferrum does not read it.
11. **Write `ferrum.toml`.** Commit it. Keep `package.json` `build` and `start` working as
    the bare commands.
12. **Prove it locally.** `PORT=4000 HOST=127.0.0.1 bun run start` for each process, with the
    variables `[env]` names exported, in a fresh clone's build output. If it needs anything
    the file does not describe, the deploy will too.
13. **Deploy.** Push a tag. In the panel: create the app from the repository (the file
    prefills the form, including the required variables and their defaults), add the names,
    set the remaining values, link the database, deploy.

## Three shapes, as ferrum.toml

One Next.js process on several hostnames (apex, `admin.`, `partner.` all served by `web`,
`www` redirecting, the app routing on `Host`):

```toml
[processes.web]
start = "bun run start"
health = "/api/healthz"

[env]
required = ["SESSION_SECRET", "NEXT_PUBLIC_SITE_URL", "NEXT_PUBLIC_ADMIN_HOST", "NEXT_PUBLIC_PARTNER_HOST"]

[env.UPLOADS_DIR]
default = "{{shared}}/uploads"
```

Two Bun servers from one build, each on its own name (`example.com` → `site`,
`admin.example.com` → `admin`), uploads shared through `UPLOADS_DIR`:

```toml
build = "bun run build"

[processes.site]
start = "bun run start:site"
port = true
health = "/healthz"

[processes.admin]
start = "bun run start:admin"
port = true
health = "/healthz"

[env]
required = ["SITE_URL", "ADMIN_URL"]

[env.UPLOADS_DIR]
about = "Shared by site and admin; served by the app under /uploads"
default = "{{shared}}/uploads"
```

A monorepo with a web app, a realtime server on a path, two workers, a wildcard for
tenants, two Postgres roles and a headless browser:

```toml
build = "bun run --filter web build"
migrate = "bun run db:migrate"
packages = ["chromium-browser"]

[processes.web]
start = "bun run start"
dir = "apps/web"
health = "/api/healthz"

[processes.realtime]
start = "bun run start"
dir = "apps/realtime"
port = true
path = "/live"
websocket = true
health = "/healthz"

[processes.jobs]
start = "bun run start"
dir = "apps/jobs"

[processes.whatsapp]
start = "bun run start"
dir = "apps/whatsapp"

[database]
url = "DATABASE_ADMIN_URL"
bypass_rls = true

[database.roles.app]
url = "DATABASE_URL"

[env]
required = ["BASE_DOMAIN", "SESSION_SECRET"]

[env.STORAGE_DIR]
default = "{{shared}}/storage"

[env.CHROMIUM_PATH]
default = "/usr/bin/chromium-browser"
```

In the panel: `example.com` serves `web` (primary), `*.example.com` serves `web` with a DNS
provider picked, `www.example.com` redirects.

## What a refused or failed deploy usually means

- "X is required by ferrum.toml and has no value": set it on the Environment tab and deploy
  again. If it is not a secret, give it a `default` in the file; if the app runs without it,
  make it a table with `optional = true`.
- "[env] names X, which Ferrum sets itself": X is `PORT`, `HOST`, the `<NAME>_PORT` of one of
  the file's own processes, or a database/Redis label. Remove it from `[env]`. (Ferrum 0.2.1
  wrongly said this for every name ending in `_PORT`; update Ferrum.)
- "[env] names X twice" (0.2.1): X is in `required` and in a table. Keep the table only.
- "[env] lists X as required and marks it optional": remove X from `required`.
- "has no process named X, which Y points at": the tag's file dropped a process a name or
  path still uses. Repoint in the panel, redeploy.
- "web healthy after …" missing, deploy rolled back: the health path did not answer within
  the startup budget — wrong path, the process listened on a literal port, or a missing
  variable crashed it (the deploy log names it).
- "jobs stopped within …": the worker exited; it must keep running.
- "is already the label of …": two things in the app's env want the same variable name.
- A `.env` or secret in the repo is never read; set values in the panel.
