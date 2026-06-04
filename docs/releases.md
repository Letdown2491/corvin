# Releases: signing & verification

Corvin publishes a `SHA256SUMS` manifest with each release plus a detached
[minisign](https://jedisct1.github.io/minisign/) signature over it. That lets you
verify two things about anything you download:

- **Integrity** — the bytes weren't corrupted or tampered with in transit
  (the SHA-256 hashes match).
- **Origin** — the manifest was signed by the Corvin release key, not an
  impostor (the minisign signature checks out).

This is Phase 1 of `docs/reproducible-builds.md`. It does not yet prove the binary
was built from this exact source (that's bit-for-bit reproducibility, a later
phase) — but it's the baseline every money wallet needs, and what a reproducibility
checker compares against.

## Verifying a release (users)

You need: the artifact(s) you downloaded, `SHA256SUMS`, `SHA256SUMS.minisig`, and
the Corvin public key `minisign.pub` (committed at the repo root and pinned in the
release notes). Install minisign (`apt install minisign`, `brew install minisign`,
`pacman -S minisign`, …), then:

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
> (the repo over HTTPS, the pinned value in release notes) — verifying a download
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
just release          # reproducibly builds the headless Linux binary in the pinned
                      # container (Dockerfile.repro) and stages it under release/ as
                      # corvin-<version>-linux-<arch>. Needs docker/podman.
just release-sign     # writes release/SHA256SUMS + SHA256SUMS.minisig
```

`just release` builds through the same pinned container as `just repro` and CI, so the
signed binary **is** the reproducible one: a downloader can rebuild with `just repro`
and confirm the hash matches the signed `SHA256SUMS`, and CI publishes the same hash
independently. (It always produces a Linux binary; for a quick host build use `just
build`.)

`just release-sign` reads the secret key from `MINISIGN_SECRET_KEY` (or
`~/.minisign/corvin-release.key`). Add any other artifacts (desktop bundles built
on their own platforms) to `release/` *before* signing so they're covered by the
same manifest — `sign-release.sh` hashes every file in the directory.

Publish `release/*` (artifacts + `SHA256SUMS` + `SHA256SUMS.minisig`) as the
release assets, and include the `minisign.pub` value in the release notes.

### Notes

- The toolchain is pinned in `rust-toolchain.toml`; bump it deliberately and call
  out the change in the release, since the compiler version affects the bytes.
- Builds use `--locked`, so `Cargo.lock` is authoritative. (Heads up: the dev
  sandbox can rewrite `Cargo.lock` with bogus `.100` crate versions — never
  release from a lockfile in that state. See the `cargo-update-proxy-100-artifact`
  note.)
- Desktop installer **signing/notarization** (macOS Gatekeeper, Windows
  SmartScreen) is Phase 2 and needs paid developer certs — see
  `docs/reproducible-builds.md`.
