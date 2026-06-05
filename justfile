# sentinelle build targets

# Run backend in dev mode (no embedded frontend)
dev-backend:
    cd crates/server && cargo run

# Run frontend dev server with proxy to backend
dev-frontend:
    cd frontend && npm run dev

# Build the frontend
build-frontend:
    cd frontend && npm install && npm run build

# Build the release binary (frontend must be built first)
build: build-frontend
    cargo build --release --package corvin
    @echo "Binary: target/release/corvin"

# Run the release binary
run: build
    ./target/release/corvin

# Run the desktop (Tauri) app. Serves the embedded SPA in-process, so the
# frontend must be built first.
dev-desktop: build-frontend
    cargo run --package corvin-desktop

# Build the desktop app release binary (frontend must be built first).
build-desktop: build-frontend
    cargo build --release --package corvin-desktop
    @echo "Binary: target/release/corvin-desktop"

# Bundle the desktop app into Linux installers (.deb + .rpm). AppImage is intentionally
# not built (its linuxdeploy tooling needs FUSE; we don't ship it). Build the frontend
# first so it's embedded. Requires rpm-build / dpkg + the Tauri CLI. On macOS/Windows,
# pass the platform's bundles explicitly (e.g. `tauri build --bundles dmg`).
bundle-desktop: build-frontend
    cd crates/desktop && tauri build --bundles deb,rpm

# Run cargo check on all crates
check:
    cargo check --workspace

# Run tests
test:
    cargo test --workspace

# ── Releases (see docs/releases.md) ─────────────────────────────────────────

# Reproducibly build the headless Linux binary in the pinned container (same build
# as `just repro` and CI) and stage it under release/ for signing — so the SIGNED
# artifact is the REPRODUCIBLE one. Always produces a Linux binary; needs docker/podman.
# (For a quick host build instead, use `just build`.)
release:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    scripts/repro-build.sh
    version=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
    arch=$(uname -m); [ "$arch" = "arm64" ] && arch=aarch64
    out="release/corvin-headless-${version}-linux-${arch}"
    rm -rf release && mkdir -p release   # start fresh so stale/old-version artifacts never get signed
    cp out/corvin "$out"
    echo "Staged $out (reproducible container build)"
    echo "Next: optionally 'just release-desktop' (Linux installers), then 'just release-sign'."

# Build the Linux desktop installers (.deb + .rpm) and stage them under release/ next to
# the headless binary, so 'just release-sign' covers them in the same SHA256SUMS +
# signature. AppImage is intentionally not shipped (needs FUSE tooling). HOST build (not
# the reproducible container) — bundles aren't bit-for-bit reproducible; minisign still
# proves authenticity. macOS (.dmg) / Windows (.msi) must be built on those OSes. Run
# after 'just release'. Needs the Tauri CLI.
release-desktop: build-frontend
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    (cd crates/desktop && tauri build --bundles deb,rpm)
    mkdir -p release
    shopt -s nullglob
    staged=0
    for f in target/release/bundle/deb/*.deb target/release/bundle/rpm/*.rpm; do
        # Rename the release asset Corvin* -> corvin-desktop* so the GitHub releases page
        # clearly distinguishes the desktop installers from the headless binary. (The
        # installed package/app identity is unchanged — only the download filename.)
        dest="release/$(basename "$f" | sed 's/^Corvin/corvin-desktop/')"
        cp "$f" "$dest" && echo "Staged $dest"
        staged=$((staged + 1))
    done
    [ "$staged" -gt 0 ] || { echo "error: no .deb/.rpm under target/release/bundle/" >&2; exit 1; }
    echo "Next: 'just release-sign'."

# Download the desktop bundles built by the 'desktop-build' CI workflow into release/, so
# they get folded into SHA256SUMS by 'just release-sign'. This is how macOS/Windows builds
# (which can't be built on Linux) become part of the signed, verifiable release. Pulls all
# platforms (deb/rpm/dmg/msi), so use this INSTEAD of 'release-desktop' for a cross-platform
# release. Find the run id with 'gh run list --workflow=desktop-build'. Needs the gh CLI.
fetch-desktop run-id:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    command -v gh >/dev/null || { echo "error: GitHub CLI 'gh' not found (install it, then 'gh auth login')" >&2; exit 1; }
    mkdir -p release
    tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
    gh run download "{{run-id}}" --dir "$tmp"
    shopt -s globstar nullglob
    staged=0
    for f in "$tmp"/**/*.deb "$tmp"/**/*.rpm "$tmp"/**/*.dmg "$tmp"/**/*.msi "$tmp"/**/*-setup.exe; do
        # Same Corvin* -> corvin-desktop* asset rename as release-desktop (download name only).
        dest="release/$(basename "$f" | sed 's/^Corvin/corvin-desktop/')"
        cp "$f" "$dest" && echo "Staged $dest"
        staged=$((staged + 1))
    done
    [ "$staged" -gt 0 ] || { echo "error: no desktop bundles found in run {{run-id}}" >&2; exit 1; }
    echo "Next: 'just release-sign' to sign everything in release/."

# Hash + minisign-sign everything staged under release/ (needs the secret key).
# Copies minisign.pub into release/ first, so the published set includes the public key
# and 'just verify-release release' can find it without a manual copy.
release-sign:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    cp minisign.pub release/
    scripts/sign-release.sh release

# Verify a staged/downloaded release (signature + hashes) via minisign.pub.
verify-release dir=".":
    scripts/verify-release.sh "{{dir}}"

# Bit-for-bit build the headless binary in a pinned container → out/corvin (+ hash).
repro:
    scripts/repro-build.sh

# Same, then check the result against a published SHA256SUMS.
repro-verify sha256sums:
    scripts/repro-build.sh --verify "{{sha256sums}}"

# Regenerate the frontend TS types from the Rust core types (ts-rs).
# Writes frontend/src/lib/generated/*.ts — commit the result.
gen-types:
    TS_RS_EXPORT_DIR="{{justfile_directory()}}/frontend/src/lib/generated" cargo test -p corvin-core --features ts export_bindings

# Drift guard: fail if the committed TS bindings are stale (a core type changed
# without re-running `just gen-types`). Regenerates to a temp dir and diffs —
# wire this into CI to keep the Rust <-> TS types honest.
check-types:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    TS_RS_EXPORT_DIR="$tmp" cargo test -p corvin-core --features ts export_bindings >/dev/null
    if diff -rq "$tmp" "{{justfile_directory()}}/frontend/src/lib/generated" >/dev/null; then
        echo "TS bindings are up to date."
    else
        echo "ERROR: TS bindings are stale — run 'just gen-types' and commit the result." >&2
        diff -ru "{{justfile_directory()}}/frontend/src/lib/generated" "$tmp" || true
        exit 1
    fi

# Bump the project version in every place it lives, then sync Cargo.lock.
# Usage: just bump-version 1.0.0
# NOTE: the StartOS package version is in the SEPARATE corvin-startos repo
# (startos/versions/current.ts, format "<version>:0") — update it there too.
bump-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    v="{{version}}"
    sed -i -E '0,/^version = "/ s/^version = "[^"]*"/version = "'"$v"'"/' Cargo.toml
    sed -i '0,/"version":/ s/"version": "[^"]*"/"version": "'"$v"'"/' frontend/package.json
    sed -i '0,/"version":/ s/"version": "[^"]*"/"version": "'"$v"'"/' crates/desktop/tauri.conf.json
    cargo update --workspace --offline   # rewrite the workspace crates' versions in Cargo.lock (no registry hit)
    echo "Bumped to $v: Cargo.toml, frontend/package.json, crates/desktop/tauri.conf.json, Cargo.lock"
    echo "REMEMBER: bump corvin-startos startos/versions/current.ts to '$v:0'"
