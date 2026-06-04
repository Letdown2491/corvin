#!/usr/bin/env bash
# Verify a downloaded Corvin release.
#
#   scripts/verify-release.sh [dir] [minisign-public-key]
#
# 1. Checks the minisign signature on SHA256SUMS against the public key.
# 2. Recomputes the hash of every listed artifact present in the directory.
#
# Run it from (or point it at) the folder holding your downloads: the artifact(s),
# SHA256SUMS, SHA256SUMS.minisig, and minisign.pub. See docs/releases.md.
set -euo pipefail

DIR="${1:-.}"
PUBKEY="${2:-${MINISIGN_PUBKEY:-minisign.pub}}"

command -v minisign >/dev/null 2>&1 || {
  echo "error: minisign not found. Install it (apt install minisign / brew install minisign)." >&2
  exit 1
}

sha256_cmd() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$@"
  else echo "error: need sha256sum or shasum" >&2; return 1; fi
}

cd "$DIR"
[ -f SHA256SUMS ] || { echo "error: SHA256SUMS not found in $DIR" >&2; exit 1; }
[ -f SHA256SUMS.minisig ] || { echo "error: SHA256SUMS.minisig not found in $DIR" >&2; exit 1; }
[ -f "$PUBKEY" ] || { echo "error: public key not found: $PUBKEY (get it from the repo / release notes)" >&2; exit 1; }

echo "== Verifying signature on SHA256SUMS =="
minisign -V -m SHA256SUMS -p "$PUBKEY"

echo "== Verifying artifact hashes =="
fail=0
checked=0
while read -r want name; do
  [ -n "${name:-}" ] || continue
  name="${name#\*}"          # strip a leading '*' (binary-mode marker), if any
  [ -f "$name" ] || continue # only verify what you actually downloaded
  got=$(sha256_cmd "$name" | awk '{print $1}')
  if [ "$got" = "$want" ]; then
    echo "  OK    $name"
    checked=$((checked + 1))
  else
    echo "  FAIL  $name"
    fail=1
  fi
done < SHA256SUMS

[ "$checked" -gt 0 ] || { echo "error: none of the listed artifacts were present to verify" >&2; exit 1; }
[ "$fail" -eq 0 ] || { echo "VERIFICATION FAILED — a hash did not match." >&2; exit 1; }
echo "OK: signature valid and $checked artifact(s) match."
