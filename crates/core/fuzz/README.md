# Fuzzing the untrusted-input parsers

These targets throw arbitrary bytes at the parsers that handle attacker/user-controlled
input, to prove they never panic, hang, or overflow. Fuzzing needs **nightly** Rust and
`cargo-fuzz` (libFuzzer), so this crate is excluded from the normal workspace.

## Run

```sh
cargo install cargo-fuzz            # one-time
rustup toolchain install nightly    # one-time
cd crates/core
cargo +nightly fuzz run parse_input        # descriptors / xpubs / addresses
cargo +nightly fuzz run describe_policy     # descriptor → policy summary
```

A crash drops a reproducer under `fuzz/artifacts/`; re-run a single case with
`cargo +nightly fuzz run parse_input fuzz/artifacts/parse_input/<case>`.

## Targets

| target | function | surface |
|---|---|---|
| `parse_input` | `corvin_core::descriptor::parse_input` | the main "paste a descriptor/xpub/address" entry |
| `describe_policy` | `corvin_core::descriptor::describe_policy` | descriptor → human-readable policy |

Good next targets to add: PSBT combine/finalize, the BBQr/UR frame decoder, BIP-329 import.
