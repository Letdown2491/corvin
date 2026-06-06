# Releases: signing & verification

Corvin publishes a `SHA256SUMS` manifest with each release plus a detached
[minisign](https://jedisct1.github.io/minisign/) signature over it. That lets you
verify two things about anything you download:

- **Integrity**: the bytes weren't corrupted or tampered with in transit (the SHA-256
  hashes match).
- **Origin**: the manifest was signed by the Corvin release key, not an impostor (the
  minisign signature checks out).

This is Phase 1 of `docs/reproducible-builds.md`. It does not yet prove the binary
was built from this exact source (that's bit-for-bit reproducibility, a later phase),
but it's the baseline every money wallet needs, and what a reproducibility checker
compares against.

## Verifying a release (users)

You need: the artifact(s) you downloaded, `SHA256SUMS`, `SHA256SUMS.minisig`, and
the Corvin public key `minisign.pub` (committed at the repo root and pinned in the
release notes). Install minisign (`apt install minisign`, `brew install minisign`, or
`pacman -S minisign`), then:

```sh
# From the folder holding your downloads:
minisign -Vm SHA256SUMS -p minisign.pub      # 1. signature must be valid
sha256sum --ignore-missing -c SHA256SUMS     # 2. hashes of what you downloaded
```

Or run the bundled helper, which does both and works on Linux and macOS:

```sh
scripts/verify-release.sh .                  # dir holding the downloads
```

A valid run prints `Signature and comment signature verified` (from minisign) and
an `OK` line per artifact. **If either step fails, do not run the binary.**

> Trust is rooted in the public key. Get `minisign.pub` from a source you trust
> (the repo over HTTPS, or the pinned value in release notes); verifying a download
> against a key the same attacker could swap proves nothing.

## Cutting a release (maintainers)

### One-time key setup

Generate the release key once and keep the secret key **offline** (never in the
repo or CI):

```sh
minisign -G -p minisign.pub -s ~/.minisign/corvin-release.key
```

Commit `minisign.pub` at the repo root. Back up the `.key` securely; losing it
means rotating to a new key (and telling users).

### Per release

```sh
just release                 # reproducibly builds the headless Linux binary in the pinned
                             # container (Dockerfile.repro) and stages it under release/ as
                             # corvin-headless-<version>-linux-<arch>. Needs docker/podman.
just fetch-desktop <run-id>  # pulls the desktop bundles (Linux .deb/.rpm + macOS .dmg +
                             # Windows .msi) built by the desktop-build CI run into release/.
                             # Needs the gh CLI. (Linux-only alternative: just release-desktop.)
just release-sign            # writes release/SHA256SUMS + SHA256SUMS.minisig over everything
```

`just release` builds through the same pinned container as `just repro` and CI, so the
signed binary **is** the reproducible one: a downloader can rebuild with `just repro`
and confirm the hash matches the signed `SHA256SUMS`, and CI publishes the same hash
independently. CI also builds the tagged commit **twice** and fails if the two hashes
differ, so reproducibility is certified automatically on every release. (It always
produces a Linux binary; for a quick host build use `just build`.)

**Desktop installers** are per-OS, since Tauri can't cross-compile them. The `desktop-build`
CI workflow builds them all (Linux `.deb`/`.rpm`, macOS `.dmg`, Windows `.msi`) on a tag or
manual dispatch; `just fetch-desktop <run-id>` downloads them into `release/` so they're
folded into the signed `SHA256SUMS`. (For a Linux-only build with no CI, `just
release-desktop` host-builds the `.deb`/`.rpm`.) AppImage is not shipped. Desktop bundles
are minisign-signed for authenticity but are **not** bit-for-bit reproducible, and OS-level
code signing / notarization (Apple/Windows certs) is a separate, optional layer that removes
Gatekeeper/SmartScreen warnings.

`just release-sign` reads the secret key from `MINISIGN_SECRET_KEY` (or
`~/.minisign/corvin-release.key`) and hashes **every file** in `release/`, so stage all
artifacts (headless binary + every desktop installer) there before signing.

Publish `release/*` (artifacts + `SHA256SUMS` + `SHA256SUMS.minisig`) as the
release assets, and include the `minisign.pub` value in the release notes.

### The release body

The GitHub release body is the version's `CHANGELOG.md` section plus a short
downloads-and-verification block. Reusable template (replace `<ver>`):

> First public release candidate. Pre-release: test with small amounts and keep your
> seed backed up. See the changelog for the full feature list.
>
> | Asset | For |
> |---|---|
> | `corvin-headless-<ver>-linux-x86_64` | Headless server binary (self-host / CLI). The reproducible build. |
> | `corvin-desktop_<ver>_amd64.deb` | Desktop app (GUI): Debian / Ubuntu |
> | `corvin-desktop-<ver>-1.x86_64.rpm` | Desktop app (GUI): Fedora / RHEL / openSUSE |
> | `corvin-desktop_<ver>_aarch64.dmg` | Desktop app (GUI): macOS (Apple Silicon) |
> | `corvin-desktop_<ver>_x64-setup.exe` | Desktop app (GUI): Windows |
> | `SHA256SUMS` · `SHA256SUMS.minisig` · `minisign.pub` | Checksums + signature |
>
> Verify (minisign key ID `534BC9B81861C67F`):
> `minisign -Vm SHA256SUMS -p minisign.pub && sha256sum -c SHA256SUMS`. Full steps in the README.
>
> Desktop builds aren't OS-code-signed. **macOS:** if it says "damaged", run
> `xattr -cr /Applications/Corvin.app`. **Windows:** SmartScreen → **More info → Run anyway**.

### Notes

- The toolchain is pinned in `rust-toolchain.toml`; bump it deliberately and call
  out the change in the release, since the compiler version affects the bytes.
- Builds use `--locked`, so `Cargo.lock` is authoritative; release only from a
  lockfile you've reviewed (`git diff Cargo.lock`) and built/tested from.
- Note: rust-bitcoin's `.100` patch versions (`bitcoin 0.32.100`,
  `bitcoin_hashes 0.14.100`) are **real** crates.io releases (a coordinated
  stable-maintenance bump), not artifacts; do not "revert" them out of the lock.
- Desktop installer **signing/notarization** (macOS Gatekeeper, Windows
  SmartScreen) is Phase 2 and needs paid developer certs; see
  `docs/reproducible-builds.md`.
