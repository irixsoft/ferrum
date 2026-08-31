#!/bin/sh
set -eu

REPO="irixsoft/ferrum"
BIN="/usr/local/bin/ferrum"
UNIT="/etc/systemd/system/ferrum.service"

PUBKEY='-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAr64AfKYROLirDNrJNte6Y3dk19Rl6+9XYiHhkSEVCPE=
-----END PUBLIC KEY-----'

die() { printf '\n%s\n\n' "$1" >&2; exit 1; }

[ "$(id -u)" = "0" ] || die "Ferrum must be installed as root. Try: curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sudo sh"

[ -r /etc/os-release ] || die "Cannot identify this system: /etc/os-release is missing."
. /etc/os-release

case "${ID:-}:${VERSION_ID:-}" in
  ubuntu:22.04|ubuntu:24.04) ;;
  *) die "Ferrum supports Ubuntu 22.04 and 24.04.
This host is ${PRETTY_NAME:-unknown}." ;;
esac

case "$(uname -m)" in
  x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
  aarch64) TARGET="aarch64-unknown-linux-musl" ;;
  *) die "Ferrum supports x86_64 and aarch64. This host is $(uname -m)." ;;
esac

for cmd in curl openssl systemctl sha256sum; do
  command -v "$cmd" >/dev/null 2>&1 || die "Required command not found: $cmd"
done

TAG="${FERRUM_VERSION:-}"
if [ -z "$TAG" ]; then
  TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
fi
[ -n "$TAG" ] || die "Could not determine the latest Ferrum release."

BASE="https://github.com/$REPO/releases/download/$TAG"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

printf 'Installing Ferrum %s for %s\n' "$TAG" "$TARGET"

curl -fsSL "$BASE/ferrum-$TARGET" -o "$TMP/ferrum"
curl -fsSL "$BASE/SHA256SUMS"     -o "$TMP/SHA256SUMS"
curl -fsSL "$BASE/SHA256SUMS.sig" -o "$TMP/SHA256SUMS.sig"

printf '%s\n' "$PUBKEY" > "$TMP/pub.pem"
if ! openssl pkeyutl -verify -pubin -inkey "$TMP/pub.pem" \
     -rawin -in "$TMP/SHA256SUMS" -sigfile "$TMP/SHA256SUMS.sig" >/dev/null 2>&1; then
  die "Signature verification failed. Refusing to install."
fi

EXPECTED=$(grep " ferrum-$TARGET\$" "$TMP/SHA256SUMS" | cut -d' ' -f1)
ACTUAL=$(sha256sum "$TMP/ferrum" | cut -d' ' -f1)
if [ -z "$EXPECTED" ] || [ "$EXPECTED" != "$ACTUAL" ]; then
  die "Checksum mismatch. Refusing to install."
fi

install -m 0755 "$TMP/ferrum" "$BIN"

cat > "$UNIT" <<'UNITEOF'
[Unit]
Description=Ferrum
Documentation=https://github.com/irixsoft/ferrum
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
ExecStart=/usr/local/bin/ferrum serve --data-dir /var/lib/ferrum
Restart=on-failure
RestartSec=2s
KillSignal=SIGTERM
TimeoutStopSec=30s
StateDirectory=ferrum
StateDirectoryMode=0700
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ProtectControlGroups=no
Environment=FERRUM_LOG=info

[Install]
WantedBy=multi-user.target
UNITEOF

systemctl daemon-reload

printf '\n%s\n' "$("$BIN" version)"

rm -rf "$TMP"
trap - EXIT INT TERM

if [ -t 0 ]; then
  exec "$BIN" setup
elif [ -e /dev/tty ] && (: >/dev/tty) 2>/dev/null; then
  exec "$BIN" setup < /dev/tty
else
  printf '\n%s\n' "Ferrum is installed. No terminal is attached, so setup did not start."
  printf '%s\n\n' "Run: ferrum setup"
fi
