<div align="center">

<img src="web/public/icon.svg" width="96" alt="">

# Ferrum

**Deploy your apps to a server you own, from one binary.**

GitHub → build → nginx → certificate → PostgreSQL → Redis — no glue, no YAML, no agent to babysit.

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)
[![Platform: Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%20%7C%2024.04%20·%20x86__64%20%7C%20arm64-lightgrey.svg)](https://github.com/irixsoft/ferrum/releases)
[![Release](https://img.shields.io/github/v/release/irixsoft/ferrum)](https://github.com/irixsoft/ferrum/releases)

[Install](#install) · [The panel](#the-panel) · [Features](#features) · [CLI](#cli-reference)

</div>

---

A VPS is cheap. Running an app on it is not: nginx, a certificate that renews, a systemd unit
per app, a database with a role that is not `postgres`, a firewall, a way to deploy without
SSHing in at midnight. Every one of those is a thing to learn, configure and keep alive.

Ferrum is all of it in a single Rust binary. You run one install command on a fresh Ubuntu
server, answer two questions, and you have a panel in your browser. Connect GitHub, pick a
repository, push a tag, and it is live on your domain with HTTPS. Databases, Redis, logs,
rollbacks and host hardening are one click each, in the same panel.

Published by [irixsoft.com](https://irixsoft.com).

## Install

On a fresh Ubuntu 22.04 or 24.04 server, x86_64 or arm64:

```sh
curl -fsSL https://raw.githubusercontent.com/irixsoft/ferrum/main/install.sh | sudo sh
```

The installer downloads the latest release for your architecture, verifies its signature and
checksum, installs the binary and a systemd unit, and starts setup. Setup asks for two things:

- **The panel's hostname**, such as `panel.example.com`. It prints the DNS record to add and
  waits for it to resolve.
- **An email address** for certificate notices.

On a small server it offers to create a swapfile, because builds routinely need more memory
than the machine has. Then it installs nginx, requests a Let's Encrypt certificate for the
panel, starts Ferrum, and prints a single-use link to create your passkey. Open it, and you are
in.

To install a specific release instead of the latest, pass `FERRUM_VERSION`:

```sh
curl -fsSL https://raw.githubusercontent.com/irixsoft/ferrum/main/install.sh | sudo FERRUM_VERSION=v0.1.0 sh
```

### Before you start

| Requirement | Detail |
| --- | --- |
| **A server** | Ubuntu 22.04 or 24.04, x86_64 or arm64, that is yours alone. Ferrum runs as root and owns nginx on it. |
| **A hostname** | An A record for the panel, such as `panel.example.com`, pointing at the server's public IP. Setup prints it. |
| **Open ports** | 22 for SSH, 80 and 443 for the panel and your apps. Nothing else. |
| **A GitHub account** | Ferrum creates a private GitHub App in it, with read access only to the repositories you pick. |

Everything else is installed when you first need it: PostgreSQL the first time you create a
database, Redis the first time an app asks for one, the firewall and fail2ban when you enable
them on the System page.

## The panel

Ferrum's panel is a web app that installs to a phone home screen or a desktop dock. You sign in
with a passkey; there are no passwords.

**Apps**

- **Deploy from GitHub.** Connect your account, or an organisation, as a private GitHub App.
  Pick a repository and a tag, and Ferrum inspects the code: it detects the runtime, the
  install, build and start commands, the migration script, the port, and the environment
  variables the code reads. Every field is prefilled and editable.
- **Every tag you push deploys.** Tag a commit, and Ferrum clones, installs, builds, runs the
  migration with the app paused, swaps the release in behind nginx, and checks the health
  endpoint before it counts as live. Rolling back is one click, and can restore the database
  snapshot the deploy took first.
- **Node, Bun, .NET and static sites.** Toolchains are installed per version and shared between
  apps. A Bun app builds and runs on Bun, with nothing else installed.
- **Environment variables** with a file import: pick your `.env` and the rows fill in.
  Nothing leaves your browser until you click Save.
- **Domains and certificates.** Add a domain, and nginx and Let's Encrypt are configured for
  it. Certificates renew on their own.
- **Logs, resources and nginx** per app: the process log, access and error logs, memory and
  CPU over time, and the generated nginx configuration with room for your own directives.

**Databases**

- **PostgreSQL** is installed on first use and pinned to that major version.
- Every database gets **its own role and password**, injected into the linked app as
  `DATABASE_URL`, and no other role can connect to it.
- **Create a database from a dump.** Upload what `pg_dump` wrote, from any PostgreSQL host.
  Ferrum turns on the extensions the dump needs, then loads it, and every table ends up owned
  by the app's role.
- **Extensions**: anything the server offers, including pgvector, searchable from the form.
- **Redis** per app, password protected, with a memory limit and persistence.

**System**

- **Firewall** with ufw, keeping SSH, 80 and 443 open, and only those.
- **fail2ban** for SSH and nginx.
- **Security updates** installed automatically.
- **SSH hardening**: see the keys installed, then turn password login off.

**Settings**

- **People and passkeys**, with a link to hand a new person for their first passkey.
- **API tokens** for the CLI and for agents.
- **Your agent.** Ferrum is an MCP server. Give your AI agent a token, and it can deploy, read
  logs, create databases and check the host, with a read-only token when that is all it should
  do.
- **Updates.** Ferrum checks for a new release daily and shows a banner. Install it with one
  click, or turn automatic updates on. Every update is signature-verified before it replaces
  the running binary, and the previous one is kept.

Press `⌘K` anywhere for the command palette.

## Features

**Deploy**

- One private GitHub App per account or organisation, read-only, created from the panel
- Deploys on every pushed tag; manual deploy of any tag, branch or commit from the CLI
- Runtime detection for Node, Bun, .NET and static sites, with editable results
- Per-app system user, systemd unit, resource limits and health check
- Migration step with the app paused and a database snapshot taken first
- Instant rollback to any kept release

**Data**

- PostgreSQL, one role and password per database, connection limits per role
- Create blank, or create from a `pg_dump` in custom or plain SQL format
- Any extension the server offers; pgvector's package ships with the server
- Redis per app with AOF persistence and `noeviction`

**Host**

- nginx in front of everything; the daemon itself listens on `127.0.0.1` only
- Let's Encrypt certificates issued and renewed for the panel and every app domain
- ufw, fail2ban, unattended security updates and SSH key-only login, each one click
- Swap created at setup when the machine needs it

**Operations**

- Signed releases: the installer and the updater verify an Ed25519 signature before anything
  runs
- Self-updating from the panel or `ferrum update`, previous binary kept at
  `/usr/local/bin/ferrum.prev`
- A CLI that talks to the running daemon with a token, and an MCP server for your agent

## CLI reference

| Command | Description |
| --- | --- |
| `ferrum setup` | Prepares the host: packages, nginx, the panel's certificate and the first passkey. Resumable. |
| `ferrum doctor` | Checks that this host is an Ubuntu release Ferrum supports. |
| `ferrum passkey enroll` | Prints a single-use link to create a passkey. |
| `ferrum token create --name <what for>` | Mints an API token, shown once. Add `--read-only` for one that can only watch. |
| `ferrum deploy <app>` | Queues a deploy and follows its log. `--ref` picks a tag, branch or commit. |
| `ferrum status` | Prints the host card the Dashboard shows. |
| `ferrum logs <app>` | Prints an app's log. `--follow` streams it, `--source` picks app, access or error. |
| `ferrum restart <app>` | Restarts an app's unit and prints its status. |
| `ferrum rollback <app>` | Rolls back to the previous release. `--to` picks one, `--restore` brings the database snapshot with it. |
| `ferrum update` | Installs the latest release. `--check` only reports whether there is one. |
| `ferrum version` | Prints the version, build id and commit this binary was built from. |

`deploy`, `status`, `logs`, `restart`, `rollback` and `update` talk to the daemon with a token:
pass `--token` or set `FERRUM_TOKEN`.

### Ports

| Port | Use |
| --- | --- |
| 22 | SSH, yours |
| 80 | HTTP, the certificate challenge and the redirect to 443 |
| 443 | HTTPS, the panel and every app |

The Ferrum daemon listens on `127.0.0.1:8443` only. nginx is the only thing facing the network.

## Repository layout

```
ferrum/
├─ crates/
│  ├─ ferrum/           # the binary: CLI, HTTP server, routes, MCP
│  ├─ ferrum-core/      # deploys, apps, databases, certificates, host state
│  └─ ferrum-platform/  # everything that touches Ubuntu, behind one trait
├─ web/                 # the panel (React, Vite, Tailwind), embedded in the binary
├─ packaging/           # systemd unit, nginx templates, the release signing public key
└─ install.sh           # the install command above
```

The panel is compiled and embedded, so a running Ferrum has no static files to serve or keep
in sync.

## Who it's for

Ferrum is for one person, or a small team, running their own applications on one server they
own. It is built to be understood and operated by one person, from the panel, without a
deployment tool to learn first.

It is not aimed at fleets, multi-tenant hosting or anything spanning more than one machine.

## Status

0.1.0 is the first stable release. Static Linux binaries for x86_64 and arm64 ship on the
[releases page](https://github.com/irixsoft/ferrum/releases), and the installer always fetches
the latest.

It has not yet accumulated years of production hours across many servers. Keep backups of
your databases, and open an issue when something breaks.

## Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).

Pull requests require agreement to the [Contributor License Agreement](CLA.md). It is a
one-time agreement, given as a sentence on your first pull request. You keep the copyright in
your contribution; IRIXSOFT LTD receives the rights needed to distribute and license Ferrum as
a whole.

## Trademark

Ferrum and the Ferrum logo are trademarks of IRIXSOFT LTD. The license below covers the source
code and grants no rights to the name or the logo.

Forks and modified versions may not use the Ferrum name or logo in a way that suggests they
are the official project or endorsed by IRIXSOFT LTD. Naming your fork something else is the
simplest way to stay clear of this.

## License

AGPL-3.0-only. Copyright © 2026 IRIXSOFT LTD. See [LICENSE](LICENSE) for the full text.
