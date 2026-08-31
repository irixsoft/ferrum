#!/usr/bin/env bash
set -euo pipefail
out="${1:-.}"
openssl genpkey -algorithm ed25519 -out "$out/ferrum-signing-key.pem"
chmod 600 "$out/ferrum-signing-key.pem"
openssl pkey -in "$out/ferrum-signing-key.pem" -pubout -out "$out/ferrum-pub.pem"
echo "private: $out/ferrum-signing-key.pem"
echo "public:  $out/ferrum-pub.pem"
