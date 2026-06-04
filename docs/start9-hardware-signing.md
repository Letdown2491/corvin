# Start9 hardware-wallet signing — decision (#18)

**Decision:** on Start9 (and the self-hosted / web deployment generally), Corvin signs
with hardware wallets via **air-gapped QR / PSBT**, not server-side USB and not
browser WebHID (for v1). Adopted 2026-06-03 after research; this resolves task #18.

## Why not "plug a signer into the Start9"
- **StartOS can't do it.** The package manifest exposes a Docker image, named volume
  mounts, network ports, health checks, and backups — and **no USB device passthrough,
  no privileged mode, no host-device mounting** (`/dev/bus/usb`, `/dev/hidraw`). A
  package physically cannot reach a USB device plugged into the server. (StartOS
  manifest spec.)
- **It's against Start9's design.** Their own guidance is explicit: the hardware wallet
  is never plugged into the server; the stack is Node (on StartOS) ← Wallet (on your
  laptop) ← Signer (on your laptop).
- **Physical UX is wrong anyway.** A Start9 is a headless box; approving on a device at
  the box, while you drive the wallet from a browser elsewhere, is backwards.

## Why not browser WebHID/WebUSB (for v1)
- It's a whole separate, browser-side signing architecture, **per vendor**, and
  **Chromium-only** (no Firefox/Safari; and WebKitGTK on the Tauri desktop can't do it
  either, which is why desktop keeps its Rust USB path).
- The transports are fragmented: Ledger/BitBox = WebHID/WebUSB, Trezor = Trezor Connect
  (iframe), **Jade = WebSerial with no browser JS library** (only a Python client). It
  can't cleanly do Jade or Coldcard at all.
- Deferred to a post-v1 convenience for the two devices with good web libs (Ledger,
  BitBox); see "Later" below.

## Why QR/PSBT (and why it's basically free)
The air-gap path is **already built** and release-grade:
- `QrSignFlow` + `lib/qr.ts`: animated multi-frame **BBQr** and **UR (`crypto-psbt`,
  fountain-coded)**, both encode and decode, with a robust camera collector (jsQR +
  getUserMedia).
- Return transports: **camera QR, file upload (`.psbt`/`.txt`), and paste** — every
  no-webcam case is covered.
- Backend `combinePsbt` merges + reports `ready`/`sigs_present`/`signed_fingerprints`,
  finalizes single-sig, and `broadcastSigned` broadcasts. **Multisig co-signer
  collection works** through the same path.

### Device coverage
| Device | On Start9 via | Status |
|---|---|---|
| Coldcard | BBQr | ✅ supported today |
| Jade | BC-UR (= the existing UR `crypto-psbt` path) | ✅ by format; needs a real-device test + labeling |
| Keystone / SeedSigner / Passport / Krux / Foundation | BBQr or UR | ✅ supported today |
| Sparrow / Specter / Nunchuk (as the signer's coordinator) | UR | ✅ |
| Ledger / Trezor / BitBox02 | no QR → PSBT file/paste shuffle via their vendor app | ⚠️ works, clunkier; WebHID later |

## Build impact (ties to #19)
Because Start9 signs via QR/PSBT, the Start9 build compiles with the **`hw` cargo
feature OFF**:
- Drops `bitbox-api` + `ledger-transport-hid` (native USB) → smaller binary, less attack
  surface.
- Lets `Dockerfile.repro` drop `libusb-1.0-0-dev` + `libudev-dev`.
- **Desktop keeps `hw` ON** (Tauri/WebKit can't do WebHID, so desktop needs the Rust
  server-side USB path).

## Small follow-ups for a polished QR-on-Start9 story
- **Jade labeling — done.** Named in `QrSignFlow` (comment + UR format tooltip) and the
  help "Sign with a hardware wallet" article (USB vs air-gap device split). **Still
  pending: a real-device test on actual Jade hardware** (it's covered by the UR
  `crypto-psbt` path by format; needs confirming end-to-end on the device).
- **No-camera fallback — done.** `QrSignFlow` now has a "No camera? Import the signed
  PSBT instead" section (load `.psbt`/`.txt` or paste), reusing the same `onSigned`
  return path as the scanner — so it works for single-sig, multisig, consolidate, and
  fee-bump alike (no longer a multisig-co-signer-only path). Help article updated.

## Later (post-v1)
- **WebHID/WebUSB one-click for Ledger + BitBox** (Chromium-only convenience). Jade stays
  QR (its best fit); Coldcard stays QR; Trezor stays PSBT-shuffle unless someone wants
  Connect. This is Option C's second phase, never a release blocker.
