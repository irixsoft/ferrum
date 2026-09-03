# Ferrum

Deploy and database management for a server you own.

Ferrum is one binary on one Ubuntu VPS. It deploys applications from GitHub, gives each one a
PostgreSQL database and a Redis instance when it asks for them, puts nginx and a Let's Encrypt
certificate in front of every domain, and shows all of it in a panel that installs as a PWA.
The same binary is an MCP server, so your own agent can drive the box with a token you mint.

## Requirements

- Ubuntu 22.04 or 24.04, x86_64 or aarch64
- Root, on a machine that is yours alone
- A hostname for the panel with an A record pointing at the server

## Install

```
curl -fsSL https://raw.githubusercontent.com/irixsoft/ferrum/main/install.sh | sudo sh
```

The script downloads the latest release for your architecture together with its `SHA256SUMS`
and the Ed25519 signature over them, verifies the signature against the key in
`packaging/ferrum-pub.pem`, checks the binary's checksum, writes `/usr/local/bin/ferrum` and
the systemd unit, and starts `ferrum setup`.

Setup installs nginx, PostgreSQL and Redis, configures ufw and fail2ban, adds swap on a small
box, issues the panel's certificate once the DNS record resolves, and prints a single-use link
to create the first passkey. `ferrum setup --staging` uses Let's Encrypt's staging directory;
the panel marks those certificates until a production setup replaces them.

The daemon listens on `127.0.0.1:8443` only. nginx is the only thing facing the network.

## The CLI

```
ferrum version    Prints the version, build id and commit this binary was built from
ferrum doctor     Checks that this host is an Ubuntu release Ferrum supports
ferrum setup      Prepares the host: packages, nginx, the panel's certificate and the first passkey
ferrum passkey    Passkeys for the panel; `enroll` prints a single-use link
ferrum token      API tokens for the CLI and for agents over MCP; `create` prints one once
ferrum deploy     Queues a deploy through the running daemon and follows its log
ferrum status     Prints the host card the Dashboard shows
ferrum logs       Prints an application's log; `--follow` streams it until Ctrl-C
ferrum restart    Restarts an application's unit and prints its status
ferrum rollback   Rolls an application back to the release before the current one and follows the log
ferrum update     Checks GitHub for a newer release and installs it through the running daemon
```

`deploy`, `status`, `logs`, `restart`, `rollback` and `update` talk to the daemon with a token:
pass `--token` or set `FERRUM_TOKEN`. Mint one with `ferrum token create --name <what for>`;
add `--read-only` for a token that can watch but not change anything.

## Updates

Ferrum checks GitHub for a newer release once a day. When there is one, the panel shows a
banner with the release notes and an Update button, and `ferrum update --check` says the same
from a shell. Nothing is installed until you click, run `ferrum update`, or turn on automatic
updates under Settings.

Every release is verified the way the installer verifies it: the signature over `SHA256SUMS`
must check against the key built into the running binary, and the binary's checksum must
match. The downloaded binary is asked for its own version before it replaces the old one,
which is kept at `/usr/local/bin/ferrum.prev`. The restart takes a few seconds; your
applications keep running behind nginx throughout.

## Licence

AGPL-3.0-only. See `LICENSE`.

The Ferrum name, wordmark and logo are trademarks of IRIXSOFT Ltd and are not
covered by the AGPL grant.

## Contributions

Not accepted. See `CONTRIBUTING.md`. Bug reports are welcome as issues.
