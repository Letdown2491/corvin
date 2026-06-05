# Reproducible builds + signed releases

Status: **Phase 1 shipped; Phase 3 mechanism in place (not yet certified); Phases 2
and 4 planned.** Goal: let a user verify that a downloaded artifact matches the public
source, closing the "trusting the build machine" gap that every money wallet needs to
close.

Phase 1 (signed releases + hashes) is implemented; see **`docs/releases.md`** for the
user-verify and maintainer-cut flow. In short: `rust-toolchain.toml` pins the compiler;
`just release` builds the headless Linux binary `--locked` and stages it; `just
release-sign` (`scripts/sign-release.sh`) writes `SHA256SUMS` plus a minisign signature;
`scripts/verify-release.sh` checks both. CI (`.github/workflows/release.yml`)
independently builds and publishes the binary's hash (signing stays offline). The minisign
public key lives at the repo root (`minisign.pub`, generated once by the maintainer); the
secret key never enters the repo or CI (enforced by `.gitignore`).

> Two distinct things, often conflated:
> - **Reproducibility:** anyone rebuilding the same source and environment gets the same
>   bytes (verifiable via published hashes). This is about the binary.
> - **Signing / notarization:** the artifact carries a vendor signature so the OS and
>   users trust its origin. This is identity, not reproducibility.
>
> Both matter and are separate work. Signing is easier and high-value (it also kills the
> Gatekeeper/SmartScreen warnings the UX testing guide flagged). Full bit-for-bit
> reproducibility is harder and partly not achievable for installers.

## What's achievable per artifact

1. **Headless Linux binary (`corvin`):** the most tractable to make reproducible, and the
   artifact Start9 and self-hosters actually run. Target: bit-for-bit reproducible in a
   pinned container.
2. **Desktop bundles (.dmg / .msi / deb/rpm):** installers embed timestamps, per-machine
   signatures, and compression metadata, so bit-for-bit repro is impractical. Target
   instead: signed, notarized, and published hashes, built in a documented pipeline. (The
   inner binary can still be reproducible even if the installer wrapper isn't.)
3. **Start9 `.s9pk`:** Start9's format is already signed and merkle-verified by their
   pipeline, so cryptographic verification is largely free; the reproducibility concern is
   pinning the Docker build image the package is built from.

## Techniques (Rust reproducibility)

Rust is not bit-for-bit out of the box; the known recipe
(reproducible-builds.org/docs/rust):

- **Pin the toolchain** with `rust-toolchain.toml` (exact channel and version).
- **Locked deps:** build with `--locked` so `Cargo.lock` is authoritative.
- **Path remapping:** `RUSTFLAGS="--remap-path-prefix=$HOME=/r --remap-path-prefix=$PWD=/b"`
  so `$CARGO_HOME` and `$PWD` don't leak into the binary.
- **`SOURCE_DATE_EPOCH`** pinned for any embedded timestamps.
- **Controlled environment:** a tagged container image (Docker/Podman, or Nix/Guix for the
  strict version) pinning rustc, the C toolchain (which matters for `ring`, `native-tls`),
  and system libs.
- **Reproducible frontend:** pin Node and the lockfile, and confirm `npm run build`
  (Vite/SvelteKit adapter-static) is deterministic, since `dist/` is embedded into the
  binary via `rust_embed`.
- **Verify** with `diffoscope` on `target/` to hunt residual nondeterminism.

## Techniques (signing)

- **Releases manifest:** publish `SHA256SUMS` plus a detached minisign (and/or GPG)
  signature over it. This is the cross-platform "verify what you downloaded" baseline and
  the thing reproducibility checkers compare against.
- **macOS:** an Apple Developer cert plus notarization (Tauri does it in-build given
  credentials), so there's no Gatekeeper "unidentified developer" wall.
- **Windows:** a code-signing cert (OV, or Azure Key Vault), so there's no SmartScreen
  wall.
- Store signing material as CI secrets, decoded into a temp keychain/store at build.

## Phased plan

**Phase 1: signed releases + hashes (highest value, lowest effort). Done.**
- `rust-toolchain.toml` pins 1.95.0 and release builds use `--locked`.
- `scripts/sign-release.sh` hashes a release dir into `SHA256SUMS` plus a minisign sig;
  `just release` / `just release-sign` wrap it. CI builds and hashes independently.
- `scripts/verify-release.sh` and `docs/releases.md` document `minisign -Vm` and hash
  verification for users; the pubkey is published at the repo root.

**Phase 2: desktop notarization/signing.**
- macOS notarization and Windows signing in the bundle pipeline (this kills the install
  warnings testers hit). Needs the developer certs and accounts, which are a real-world
  dependency, not just code.

**Phase 3: reproducible Linux binary. Mechanism in place, not yet certified.**
- Built: `Dockerfile.repro` (multi-stage, base images pinned by `@sha256` digest, fixed
  `WORKDIR`/`CARGO_HOME`/`TZ`/`LC_ALL`, `--remap-path-prefix`, `SOURCE_DATE_EPOCH`),
  `scripts/repro-build.sh` (plus `just repro` / `just repro-verify SHA256SUMS`). The script
  uses a fixed `SOURCE_DATE_EPOCH` constant (it only normalizes embedded frontend mtimes;
  a constant is identical across git and non-git checkouts) and extracts `out/corvin` plus
  its hash; `--verify` compares it to a published `SHA256SUMS`.
- Wired into CI: `release.yml` runs this container build on a tag and publishes the hash,
  so a release has three independently-produced hashes to cross-check (GitHub's runner, the
  user's own `just repro`, and the maintainer's signed `SHA256SUMS`), which is a stronger
  signal than building twice on one machine.
- Determinism fixes baked in: the frontend builds with `npm ci` (lockfile-exact), and
  `dist/` file mtimes are normalized to `SOURCE_DATE_EPOCH` because `rust_embed` embeds
  them. The only build-time value the binary embeds is `CARGO_PKG_VERSION` (deterministic).
- To certify (needs real runs):
  1. Build it twice (ideally on two machines) and confirm the `out/corvin` hashes match;
     chase any residual nondeterminism with `diffoscope`.
  2. Record the pinned base-image digests in `docs/releases.md` per release, and bump a
     digest only to one you've `podman pull`ed.
  3. Confirm `cargo build --release` codegen is stable across machines; if `diffoscope`
     finds noise, set `codegen-units = 1` in the release profile.
- Simplification pending #19: the headless binary still links the USB hardware-wallet
  crates, so the image installs `libusb-1.0-0-dev` + `libudev-dev`. Once the `hw` cargo
  feature gates those out for headless, drop them from `Dockerfile.repro`. (At-rest
  encryption uses pure-Rust crypto and adds no C dependency, so it has no effect here.)

**Phase 4: Start9 packaging + its build image.**
- Pin the `.s9pk` wrapper's Docker base image so the package is reproducible on top of
  Start9's already-signed format.

## Open / to decide

- **minisign vs GPG** (or both) for the release signature. minisign is simpler and modern;
  GPG is more familiar to some. Leaning minisign, optionally both.
- Apple Developer Program ($99/yr) plus a Windows code-signing cert are paid,
  identity-bound prerequisites for Phase 2; decide if and when to acquire them.
