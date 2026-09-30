# Domains and paths

Every name Ferrum answers for is a row on the app's Configuration tab. Point each name at this server in your DNS first; Ferrum never writes DNS records for ordinary names.

## A name does one of two things

- **Serve**, and which process answers. Several names can point at the same process (`oshaawards.org`, `admin.oshaawards.org` and `partner.oshaawards.org` all served by `web`, with the app deciding what to show from the `Host` header), or at different ones (`example.com` → `site`, `admin.example.com` → `admin`).
- **Redirect**, to another served name of the same app: `www.example.com` → `example.com`.

One name is the app's **primary**: the address the panel shows, and the default target when you add a redirect. Change it any time.

## Paths

A path sends part of the URL space to a process, on every served name: `/live` → `realtime`. Paths come from `ferrum.toml` when the app follows its file, or from the Paths card otherwise. A process without a path of its own answers `/` on the names that point at it.

## Certificates

Each name gets its own certificate from Let's Encrypt, redirects included, and its own nginx block, so one name whose certificate fails never takes the others down. Until a certificate exists a name is served over plain HTTP. Every served name tells browsers to use HTTPS for that exact name for a year; subdomains hosted elsewhere are not affected.

## Wildcards

`*.example.com` is one more row, served like any name, and covers every subdomain that is not its own row. It needs a certificate that only a DNS proof can give; see *Wildcards and DNS providers*. `example.com` itself is a separate row.

## Headers your app sees

nginx passes `Host`, `X-Forwarded-Host`, `X-Forwarded-Proto` and `X-Forwarded-For`, so a framework that builds absolute URLs behind a proxy has what it needs. WebSocket paths get a 24-hour read timeout.
