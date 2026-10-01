# Processes

A process is a program Ferrum keeps running for an app, or a folder of built files nginx serves. An app has one or more, all built from the same release.

## What every process shares

The repository, the build, the release directory, the environment file, the Linux user and the `shared/` directory. One deploy updates all of them and one rollback puts all of them back, so they always run the same version of the code.

## What each process has of its own

- A name, a command and the folder it starts in.
- A port, when visitors reach it. It arrives as `PORT`, set for that process only.
- A health path, for processes with a port.
- A memory limit, set in the panel. It stays when the repository's file changes.
- Its own systemd unit, `ferrum-app-<slug>-<process>`, its own log, and its own Restart button.

## Reaching a sibling

Every process with a port is named in the shared environment as `<NAME>_PORT`: `WEB_PORT`, `REALTIME_PORT`. `HOST` is always `127.0.0.1`. Most apps never need this, because nginx routes paths and names to processes for you; see *Domains and paths*.

## How a deploy treats them

1. Clone, install and build once.
2. Read `ferrum.toml` or the Procfile from the tag, when the app follows its file.
3. Snapshot the database and run the migration command once. With "Pause traffic while migrations run" on, the maintenance page goes up and every process is stopped first, so no worker touches the database mid-change.
4. Write one unit per process, switch `current` to the new release, and start or restart every process.
5. A process with a port passes when its health path answers within the startup budget. A worker passes when it is still running ten seconds after starting.
6. If any process fails, the whole deploy fails and every process goes back to the previous release.

## Folders

A process with `static = "apps/admin/dist"` instead of a command is a folder. nginx serves it straight from the release: at `/` when a name points at it, or under its path. An app made only of folders has nothing to restart and no application log.

## Something a process needs that its siblings don't

Put it in front of the command: `MEDUSA_WORKER_MODE=worker bun run start`. Every process reads the same environment otherwise.
