#!/usr/bin/env bash
# Reproducibly build the headless `corvin` Linux binary in a pinned container and
# print its SHA-256. See docs/reproducible-builds.md (Phase 3).
#
#   scripts/repro-build.sh                 # build, write out/corvin, print its hash
#   scripts/repro-build.sh --verify FILE   # also compare the hash to a SHA256SUMS file
#   SOURCE_DATE_EPOCH=... scripts/repro-build.sh   # override the embedded timestamp
#
# Uses docker or podman, whichever is present. SOURCE_DATE_EPOCH defaults to the
# HEAD commit time, so two people building the same tag get the same bytes.
set -euo pipefail

cd "$(dirname "$0")/.."

VERIFY=""
if [[ "${1:-}" == "--verify" ]]; then
  VERIFY="${2:?--verify needs a SHA256SUMS file}"
fi

# Pick a container engine.
ENGINE=""
for e in docker podman; do
  if command -v "$e" >/dev/null 2>&1; then ENGINE="$e"; break; fi
done
[[ -n "$ENGINE" ]] || { echo "error: need docker or podman on PATH" >&2; exit 1; }

# Fixed timestamp used only to normalize the embedded frontend mtimes (rust_embed
# bakes them in). The exact value is irrelevant; it just has to be IDENTICAL for
# everyone, so a constant is more robust than a git commit time — the latter would
# diverge whenever someone builds from a non-git checkout (tarball, copied dir).
# Override with SOURCE_DATE_EPOCH=... only if you know why.
SDE="${SOURCE_DATE_EPOCH:-1735689600}"
echo "Engine:            $ENGINE"
echo "SOURCE_DATE_EPOCH: $SDE ($(date -u -d "@$SDE" 2>/dev/null || true))"

rm -rf out && mkdir -p out
"$ENGINE" build -f Dockerfile.repro \
  --build-arg "SOURCE_DATE_EPOCH=$SDE" \
  -o out .

[[ -f out/corvin ]] || { echo "error: out/corvin was not produced" >&2; exit 1; }

sha256_cmd() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$@"
  else echo "error: need sha256sum or shasum" >&2; return 1; fi
}

HASH="$(sha256_cmd out/corvin | awk '{print $1}')"
echo
echo "built: out/corvin"
echo "sha256: $HASH"

if [[ -n "$VERIFY" ]]; then
  if grep -qi "$HASH" "$VERIFY"; then
    echo "MATCH: hash is present in $VERIFY — reproducible."
  else
    echo "MISMATCH: $HASH is NOT in $VERIFY." >&2
    echo "The build is not reproducible against that manifest (or you built a different commit)." >&2
    exit 1
  fi
fi
