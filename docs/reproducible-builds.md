# Reproducible builds + signed releases

Status: **Phase 1 shipped; Phases 2–4 planned.** Goal: let a user verify that a
downloaded artifact matches the public source — close the "trusting the build
machine" gap that every money wallet needs to close.

Phase 1 (signed releases + hashes) is implemented — see **`docs/releases.md`** for
the user-verify + maintainer-cut flow. In short: `rust-toolchain.toml` pins the
compiler; `just release` builds the headless Linux binary `--locked` and stages it;
`just release-sign` (`scripts/sign-release.sh`) writes `SHA256SUMS` + a minisign
signature; `scripts/verify-release.sh` checks both. CI (`.github/workflows/release.yml`)
independently builds + publishes the binary's hash (signing stays offline). The
minisign public key lives at the repo root (`minisign.pub`, generated once by the
maintainer); the secret key never enters the repo or CI (enforced by `.gitignore`).

> Two distinct things, often conflated:
> - **Reproducibility** = anyone rebuilding the same source + environment gets the
>   *same bytes* (verifiable via published hashes). This is about the binary.
> - **Signing / notarization** = the artifact carries a vendor signature so the OS
>   (and users) trust its origin. This is identity, not reproducibility.
>
> Both matter; they're separate work. Signing is easier and high-value (also kills
> the Gatekeeper/SmartScreen warnings the UX testing guide flagged). Full bit-for-bit
> reproducibility is harder and partly not achievable for installers.

## Current state (baseline)

- No CI (`.github/workflows` absent), no pinned Rust toolchain (`rust-toolchain.toml`
  absent), no release signing, no published hashes.
- Build: `just build` → frontend (`npm run build` → `dist/`) then
  `cargo build --release -p corvin`; desktop via `tauri build` (per-platform).
- `Cargo.lock` is committed (good — prerequisite).
- Frontend is embedded at compile time (`rust_embed`), so a reproducible binary also
  requires a **reproducible `dist/`** (npm build determinism).
- No Start9 package exists yet (`packaging/` only has udev rules).
- At-rest encryption (shipped) adds **no** new C dependency: it uses pure-Rust
  RustCrypto (`argon2`, `chacha20poly1305`) and the BDK `.db` was replaced by a sealed
  CBOR blob, so `rusqlite`/SQLCipher were dropped entirely. The originally-planned
  `bundled-sqlcipher` (vendored C) was rejected, which keeps the C-toolchain surface
  the same as before. See `docs/at-rest-encryption.md`.

## What's achievable per artifact

1. **Headless Linux binary (`corvin`)** — the most tractable to make reproducible, and
   the artifact Start9 / self-hosters actually run. Target: **bit-for-bit reproducible
   in a pinned container.**
2. **Desktop bundles (.dmg / .msi / AppImage / deb/rpm)** — installers embed
   timestamps, per-machine signatures, compression metadata; **bit-for-bit repro is
   impractical**. Target instead: **signed + notarized + published hashes**, built in
   a documented pipeline. (The *inner binary* can still be reproducible even if the
   installer wrapper isn't.)
3. **Start9 `.s9pk`** — Start9's format is already **signed + merkle-verified** by
   their pipeline, so cryptographic verification is largely free; the reproducibility
   concern is pinning the **Docker build image** the package is built from.

## Techniques (Rust reproducibility)

Rust is **not** bit-for-bit OOTB; the known recipe (reproducible-builds.org/docs/rust):
- **Pin the toolchain** — add `rust-toolchain.toml` (exact channel + version).
- **Locked deps** — build with `--locked` so `Cargo.lock` is authoritative.
- **Path remapping** — `RUSTFLAGS="--remap-path-prefix=$HOME=/r --remap-path-prefix=$PWD=/b"`
  so `$CARGO_HOME` / `$PWD` don't leak into the binary.
- **`SOURCE_DATE_EPOCH`** pinned for any embedded timestamps.
- **Controlled environment** — a tagged container image (Docker/Podman, or Nix/Guix
  for the strict version) pinning rustc, the C toolchain (matters for `ring`,
  `native-tls`), and system libs.
- **Reproducible frontend** — pin Node + lockfile; confirm `npm run build` (Vite/
  SvelteKit adapter-static) is deterministic, since `dist/` is embedded.
- **Verify** with `diffoscope` on `target/` to hunt residual nondeterminism.

## Techniques (signing)

- **Releases manifest:** publish `SHA256SUMS` + a detached **minisign** (and/or GPG)
  signature over it. This is the cross-platform "verify what you downloaded" baseline
  and the thing reproducibility checkers compare against.
- **macOS:** Apple Developer cert + **notarization** (Tauri does it in-build given
  credentials) → no Gatekeeper "unidentified developer" wall.
- **Windows:** code-signing cert (OV, or Azure Key Vault) → no SmartScreen wall.
- **AppImage:** `SIGN=1` + passphrase env at build.
- Store signing material as CI secrets, decode into a temp keychain/store at build.

## Phased plan

**Phase 1 — Signed releases + hashes (highest value / lowest effort; do first). ✅ DONE.**
- ✅ `rust-toolchain.toml` (pins 1.95.0) + release builds use `--locked`.
- ✅ `scripts/sign-release.sh` hashes a release dir → `SHA256SUMS` + minisign sig;
  `just release` / `just release-sign` wrap it. CI builds + hashes independently.
- ✅ `scripts/verify-release.sh` + `docs/releases.md` document `minisign -Vm` + hash
  verification for users; pubkey published at repo root.
- *Outcome:* users can verify integrity + origin even before bit-for-bit repro lands.
- *Remaining one-time maintainer step:* generate the minisign key
  (`minisign -G -p minisign.pub -s ~/.minisign/corvin-release.key`) and commit
  `minisign.pub`. Not done here — it's an identity the maintainer must hold offline.

**Phase 2 — Desktop notarization/signing.**
- macOS notarization + Windows signing in the bundle pipeline (kills the install
  warnings testers hit). Needs the developer certs/accounts — a real-world dependency,
  not just code.

**Phase 3 — Reproducible Linux binary. 🟡 IN PROGRESS (mechanism in place; not yet certified).**
- **Built:** `Dockerfile.repro` (multi-stage, pinned `node` + `rust` images, fixed
  `WORKDIR`/`CARGO_HOME`/`TZ`/`LC_ALL`, `--remap-path-prefix`, `SOURCE_DATE_EPOCH`),
  `scripts/repro-build.sh` (+ `just repro` / `just repro-verify SHA256SUMS`). The script
  uses a fixed `SOURCE_DATE_EPOCH` constant (only normalizes embedded frontend mtimes;
  a constant is identical across git and non-git checkouts) and extracts `out/corvin` +
  its hash; `--verify` compares it to a published `SHA256SUMS`.
- **Wired into CI:** `.github/workflows/release.yml` runs this container build on a tag
  and publishes the hash. So a release has three independently-produced hashes to cross-
  check — GitHub's runner, the user's own `just repro`, and the maintainer's signed
  `SHA256SUMS` — which is a stronger signal than building twice on one machine.
- **Two determinism fixes baked in:** the frontend builds with `npm ci` (lockfile-exact),
  and `dist/` file mtimes are normalized to `SOURCE_DATE_EPOCH` because `rust_embed`
  embeds them into the binary (otherwise build time would leak in). The only build-time
  value the binary embeds is `CARGO_PKG_VERSION` (deterministic).
- **Still to certify (needs real runs — not doable without a container engine):**
  1. Build it **twice** (ideally on two machines) and confirm the `out/corvin` hashes
     match; chase any residual nondeterminism with `diffoscope`.
  2. **Pin the base images by `@sha256:<digest>`** (currently tags) and record the
     digests in `docs/releases.md` for each release.
  3. ✅ `just release` now builds **from the repro container**, so the signed binary *is*
     the reproducible one.
  4. Confirm `cargo build --release` codegen is stable across machines; if `diffoscope`
     finds noise, set `codegen-units = 1` in the release profile.
- **Simplification pending #19:** the headless binary still links the USB hardware-wallet
  crates, so the image installs `libusb-1.0-0-dev` + `libudev-dev`. Once the `hw` cargo
  feature gates those out for headless, drop them from `Dockerfile.repro`.
- (At-rest encryption added no C dependency, so there's no SQLCipher C-compile
  determinism to settle here; the C surface is unchanged.)

**Phase 4 — Start9 packaging + its build image.**
- When the Start9 `.s9pk` wrapper is built (separate work), pin its Docker base image
  so the package is reproducible on top of Start9's already-signed format.

## Open / to decide

- **minisign vs GPG** (or both) for the release signature. minisign is simpler +
  modern; GPG is more familiar to some. Lean minisign, optionally both.
- Whether to stand up **CI (GitHub Actions or self-hosted)** now or script releases
  locally first. CI is where signing secrets + multi-OS bundles naturally live.
- Apple Developer Program ($99/yr) + a Windows code-signing cert are **paid, identity-
  bound** prerequisites for Phase 2 — decide if/when to acquire.
- ~~Sequence vs at-rest encryption: SQLCipher's C build affects Phase 3 determinism.~~
  Resolved: at-rest encryption shipped with pure-Rust crypto and dropped `rusqlite`
  entirely, so it has no effect on the repro container's C surface.
