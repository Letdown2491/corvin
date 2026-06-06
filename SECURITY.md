# Security Policy

Corvin is a Bitcoin wallet, so security reports are taken seriously. Thank you for
helping keep users' funds and privacy safe.

## Supported versions

Corvin is pre-1.0 (currently `1.0.0-rc.2`). Security fixes are applied to the latest
release only; always run the most recent version.

| Version                               | Supported |
| ------------------------------------- | --------- |
| Latest release (incl. `1.0.0-rc.2`)   | ✅        |
| Older / superseded pre-release builds | ❌        |

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately via GitHub's private vulnerability reporting:

1. Open the **Security** tab of this repository.
2. Click **Report a vulnerability**.
3. Describe the issue with enough detail to reproduce it.

If that channel is unavailable, open a minimal, non-sensitive issue asking how to
reach the maintainers privately, but **do not include vulnerability details** in a
public issue.

We aim to acknowledge reports within a few days and will keep you informed on the
fix and a coordinated-disclosure timeline.

### What to include

- A clear description and the impact (funds at risk? privacy leak? key exposure?).
- Steps to reproduce, the affected version or commit, and the environment
  (desktop / Start9 / PWA, OS, Bitcoin network).
- A proof of concept if you have one.

## Scope & threat model

Corvin is designed for a **single user on a trusted local machine or a trusted
Start9 host**. It binds to `127.0.0.1` and has **no API authentication or
multi-tenant hardening, by design**. The following are therefore **not** treated as
vulnerabilities:

- "The API has no authentication" or "it is reachable from localhost."
- Denial-of-service or resource exhaustion against your own instance.
- Issues that require an already-compromised host or physical access to the machine
  running Corvin.

**In scope (please do report):**

- Anything that could cause **loss of funds** or **signing an unintended
  transaction**.
- **Seed / private-key exposure**: on disk, in logs, in memory beyond its intended
  lifetime, or over the network.
- **Privacy leaks**: for example, addresses, xpubs, or txids sent to a party that
  should not receive them.
- Incorrect cryptography or protocol handling (BIP-32/39/322/352, Payjoin/BIP-77,
  PSBT, at-rest encryption).
- Remote-triggerable issues when Corvin is reached over its intended interface
  (for example, Tor / LAN on Start9).

## Safe harbor

We will not pursue or support legal action against researchers who act in good
faith, follow this policy, and avoid privacy violations and destruction of data.
