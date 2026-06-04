# Corvin packaging artifacts

Files in this directory are consumed by deployment targets, not by the running
binary itself. Each subdirectory groups files by target platform.

## `udev/51-corvin.rules`

Linux udev rules granting non-root access to supported hardware-wallet USB
devices. Without this, Corvin's HW detection will fail with "permission
denied" — Linux blocks direct HID/USB reads unless either the process is root
or a udev rule explicitly grants the calling user access.

### Install (system-wide, recommended)

```sh
sudo cp packaging/udev/51-corvin.rules /lib/udev/rules.d/
sudo udevadm control --reload
sudo udevadm trigger
```

Then unplug and replug your device. Test with:

```sh
ls -l /dev/hidraw* /dev/bus/usb/*/*
```

The device files should be group-readable by `plugdev` (or whichever group
your distro uses; edit the rules file if needed).

### Covered devices

| Brand | VID | Notes |
| --- | --- | --- |
| BitBox02 | 0x03eb | All editions |
| Ledger Nano S/S+/X/Stax/Flex | 0x2c97, 0x2581 | All Bitcoin app firmware |
| Trezor One | 0x534c | Legacy VID |
| Trezor Model T / Safe 3 / Safe 5 | 0x1209 | Production VID |
| OneKey Classic / Pro | (Trezor protocol) | Covered by Trezor rules |

Not yet covered: Blockstream Jade (task #84), Keystone (QR-only — no USB rule needed).

## Start9 manifest (TODO)

When the Start9 package is being prepared, USB device access also has to be
declared in the package's `manifest.yaml` so StartOS forwards device events to
the container. Sketch of what'll be needed:

```yaml
hardware-requirements:
  usb-devices:
    - vendor-id: 0x03eb   # BitBox
    - vendor-id: 0x2c97   # Ledger
    - vendor-id: 0x2581   # Ledger (legacy)
    - vendor-id: 0x1209   # Trezor / OneKey
    - vendor-id: 0x534c   # Trezor One (legacy)
```

This file will live alongside the rest of the Start9 packaging once that work
begins.
