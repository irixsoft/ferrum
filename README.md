<div align="center">

<img src="web/public/icon.svg" width="96" alt="">

# Ferrum

**Deploy your apps to a server you own, from one binary.**

GitHub → build → nginx → certificate → PostgreSQL → Redis — no glue, no YAML, no agent to babysit.

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)
[![Platform: Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%20%7C%2024.04%20·%20x86__64%20%7C%20arm64-lightgrey.svg)](https://github.com/irixsoft/ferrum/releases)
[![Release](https://img.shields.io/github/v/release/irixsoft/ferrum)](https://github.com/irixsoft/ferrum/releases)

[Install](#install) · [What a repository needs](#what-a-repository-needs) · [What you get](#what-you-get) · [CLI](#cli-reference)

</div>

---

A VPS is cheap. Running an app on it is not: nginx, a certificate that renews, a systemd unit
per process, a database with a role that is not `postgres`, a firewall, a way to deploy without
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
curl -fsSL https://raw.githubusercontent.com/irixsoft/ferrum/main/install.sh | sudo FERRUM_VERSION=v0.2.0 sh
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

## What a repository needs

Nothing, for the common case: Ferrum reads `package.json` or the `.csproj`, works out the
runtime and the install, build, start and migration commands, and prefills the form. Beyond
that, one file:

- Listen on `PORT` and `HOST` from the environment; `HOST` is always `127.0.0.1`.
- A `ferrum.toml` at the root names the app's processes, their paths, its commands, the
  Ubuntu packages it needs, the variables it reads, and the names it reads its database
  under. Ferrum reads it on every deploy; a deploy that would leave a required variable
  empty is refused before anything is built.
- Push a tag to deploy it.

Every convention is explained, with examples, under **Help** in the panel, and your AI agent
can read the same pages through Ferrum's MCP server.

### Your AI agent

[`skills/ferrum-app/SKILL.md`](skills/ferrum-app/SKILL.md) is a skill for Claude Code and
compatible agents: copy the `ferrum-app` folder into your agent's skills directory and it
knows how to write a `ferrum.toml`, lay out processes, env and packages, and walk an existing
app through the conventions above, without reading this repository.

## What you get

**Apps.** One app can be several processes from one build: a web server, a realtime server, a
worker, a bot, a folder of built files served by nginx. Each gets its own systemd unit, port,
log, memory limit and health check, and they deploy and roll back together. Node, Bun and .NET
toolchains install per version, private to Ferrum.

**Deploys.** Every tag you push is cloned, built, migrated with a database snapshot taken first,
checked for health per process, and swapped in behind nginx. A tag that would break the app is
refused before anything is switched, and rolling back is one click.

**Domains.** Each name serves a process or redirects to another name; several names can share a
process, and a wildcard covers every subdomain. One Let's Encrypt certificate per name,
renewed when the CA says to; wildcards through a Cloudflare or Route 53 token.

**Databases.** PostgreSQL installed on first use. Every database has an owner and can have
restricted roles for code that must not bypass row-level security. Create blank or from a
`pg_dump`, any extension the server offers, connect from your machine through the tunnel the
panel shows you. Redis per app.

**Host.** ufw, fail2ban, unattended security updates and SSH key-only login, each one click.
nginx faces the network; the daemon listens on `127.0.0.1` only.

**Notifications.** The panel installs to a phone, and tells you when a deploy was refused or
went live, when something broke on its own, and when an update is waiting.

**Operations.** Signed releases verified before anything runs; self-updating from the panel or
the CLI with the previous binary kept; a CLI and an MCP server for your agent, with read-only
tokens when that is all it should do. Press `⌘K` anywhere for the command palette.

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
| `ferrum restart <app>` | Restarts an app's processes and prints their status. |
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
├─ help/                # the Help topics, embedded in the binary
├─ skills/              # the ferrum-app skill for AI agents
├─ packaging/           # systemd unit, nginx templates, the release signing public key
└─ install.sh           # the install command above
```

The panel and the help are compiled in, so a running Ferrum has no static files to serve or
keep in sync.

## Who it's for

Ferrum is for one person, or a small team, running their own applications on one server they
own. It is built to be understood and operated by one person, from the panel, without a
deployment tool to learn first.

It is not aimed at fleets, multi-tenant hosting or anything spanning more than one machine.

## Status

Static Linux binaries for x86_64 and arm64 ship on the
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
