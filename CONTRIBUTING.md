# Contributing

Issues and pull requests are welcome.

## Bug reports

The most useful report carries the Ferrum version (`ferrum version`), the Ubuntu release,
what you did, and the relevant lines from `journalctl -u ferrum` or the deploy log in the
panel. Never paste an API token, a database password or the contents of an app's `.env`.

## Pull requests

- Your first pull request is asked to sign the [Contributor License Agreement](CLA.md) by
  replying with one sentence on the pull request. It is a one-time agreement; you keep the
  copyright in your contribution.
- Rust is formatted with `cargo fmt` and clean under `cargo clippy --workspace --all-targets`
  with warnings denied. The suite is `cargo test --workspace`.
- The panel in `web/` is built and tested with Bun only: `bun install`, `bun run typecheck`,
  `bun test`, `bun run build`.
- Every command that touches the host goes through the `Platform` trait in
  `crates/ferrum-platform`; nothing outside it runs `apt`, `systemctl` or `useradd`.
- Commit messages follow Conventional Commits: `feat(nginx): ...`, `fix(deploy): ...`.
