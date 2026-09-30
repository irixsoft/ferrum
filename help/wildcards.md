# Wildcards and DNS providers

A name like `*.example.com` covers every subdomain at once, for apps that give each customer their own: `school.example.com`, `other.example.com`, without a fixed list.

## Why it needs DNS access

Let's Encrypt proves an ordinary name by visiting it. There is no single address to visit for "every subdomain", so a wildcard is proven by a DNS record instead: a text record named `_acme-challenge.example.com` holding a code. Certificates last 45 days and every renewal needs a fresh code, so Ferrum must be able to write that record itself.

## Setting it up

1. At your DNS provider, create an API token that can edit the records of that one zone. Cloudflare and Route 53 are supported.
2. Under Settings › Connections › DNS providers, add the provider with the token. Ferrum writes and removes a test record on save, so a bad token fails right there.
3. On the app's Configuration tab, add `*.example.com` and pick the provider. A wildcard without a provider is refused.
4. Add `example.com` as its own row if the apex should be served too; a wildcard does not cover it.

Ferrum writes the one `_acme-challenge` record at each issue and renewal, waits until the zone's own nameservers answer with it, and removes it after. Tokens are stored encrypted and never shown again.

## Renewals

Every certificate, wildcard or not, is renewed when Let's Encrypt's renewal information says to, which also keeps renewals outside the rate limits. When that information is unavailable, a certificate is renewed with a third of its lifetime left.

## What a wildcard cannot do

Cover a customer's own domain (`portal.theirschool.edu`). Add such a name as an ordinary row instead.
