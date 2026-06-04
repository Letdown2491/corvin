#!/usr/bin/env bash
# Hash + minisign-sign a directory of Corvin release artifacts.
#
#   scripts/sign-release.sh <release-dir> [minisign-secret-key]
#
# Produces SHA256SUMS over every file in the directory (except the sums/sig files
# themselves) and a detached minisign signature SHA256SUMS.minisig. Users verify
# the signature with the public key, then check the hashes — see docs/releases.md.
#
# The secret key is the maintainer's release identity: generate it once
# (`minisign -G -p minisign.pub -s ~/.minisign/corvin-release.key`), keep the
# .key OFFLINE, and commit only the .pub. Never put the secret key in the repo.
set -euo pipefail

DIR="${1:?usage: sign-release.sh <release-dir> [minisign-secret-key]}"
SECKEY="${2:-${MINISIGN_SECRET_KEY:-$HOME/.minisign/corvin-release.key}}"

command -v minisign >/dev/null 2>&1 || {
  echo "error: minisign not found. Install it (apt install minisign / brew install minisign / pacman -S minisign)." >&2
  exit 1
}
[ -d "$DIR" ] || { echo "error: no such directory: $DIR" >&2; exit 1; }
[ -f "$SECKEY" ] || {
  echo "error: minisign secret key not found at: $SECKEY" >&2
  echo "       Generate one once: minisign -G -p minisign.pub -s '$SECKEY'" >&2
  echo "       Keep the .key offline; commit only minisign.pub. See docs/releases.md." >&2
  exit 1
}

sha256_cmd() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$@"
  else echo "error: need sha256sum or shasum" >&2; return 1; fi
}

cd "$DIR"
# Every artifact except the sums/signature files. Portable (no GNU -printf).
mapfile -t files < <(find . -maxdepth 1 -type f ! -name 'SHA256SUMS*' | sed 's|^\./||' | sort)
[ "${#files[@]}" -gt 0 ] || { echo "error: no artifacts to hash in $DIR" >&2; exit 1; }

sha256_cmd "${files[@]}" > SHA256SUMS
echo "Wrote SHA256SUMS:"
sed 's/^/  /' SHA256SUMS

# A trusted comment travels inside the signature; minisign -V prints it on verify.
minisign -S -m SHA256SUMS -s "$SECKEY" \
  -t "Corvin release SHA256SUMS, signed $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "Signed -> $DIR/SHA256SUMS.minisig"
