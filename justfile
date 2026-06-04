# sentinelle build targets

# Run backend in dev mode (no embedded frontend)
dev-backend:
    cd crates/server && cargo run

# Run frontend dev server with proxy to backend
dev-frontend:
    cd frontend && /usr/bin/npm run dev

# Build the frontend
build-frontend:
    cd frontend && /usr/bin/npm install && /usr/bin/npm run build

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

# Bundle the desktop app into installers (.deb/.rpm/.AppImage on Linux, .dmg on
# macOS, .msi/NSIS on Windows). Build the frontend first so it's embedded in the
# release binary. Requires the platform's packaging tools (Linux: rpm-build /
# dpkg / AppImage tooling). The Tauri CLI is found via tauri.conf.json in
# crates/desktop. Use `tauri build --no-bundle` to compile the binary only.
bundle-desktop: build-frontend
    cd crates/desktop && tauri build

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
    out="release/corvin-${version}-linux-${arch}"
    mkdir -p release
    cp out/corvin "$out"
    echo "Staged $out (reproducible container build)"
    echo "Next: 'just release-sign' to write SHA256SUMS + SHA256SUMS.minisig."

# Hash + minisign-sign everything staged under release/ (needs the secret key).
release-sign:
    scripts/sign-release.sh "{{justfile_directory()}}/release"

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
