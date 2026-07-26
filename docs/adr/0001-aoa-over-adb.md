# 0001 — Use AOA rather than ADB

## Context

Getting a phone's screen onto a computer over USB has an obvious answer: ADB.
It is well documented, it is what scrcpy uses, and it is faster to build
against than anything else.

It also requires the user to open Settings, tap the build number seven times,
enable USB debugging, and accept an RSA fingerprint prompt. For a developer
that is nothing. For everyone else it is a wall — and it leaves debugging
enabled afterwards, which is a real security cost paid for a screen-sharing
app.

Android Open Accessory is the other option. The phone enumerates as a USB
accessory, the system shows one "allow this app to access the accessory"
prompt, and that is the whole setup.

## Decision

Use AOA. Accept that it is harder to build against.

## Consequences

The setup is one prompt on the phone, and on Linux one `udev` rule on the host.
No developer mode, no ADB, nothing left enabled afterwards.

The cost is that AOA is a plain byte pipe with no message framing and no
metadata, so:

- The framing in `packages/mirror-protocol` exists to put message boundaries
  back. A demuxer that resynchronises on a magic number is not something ADB
  would have needed.
- The host has to be granted raw USB access, which is why `platform/` has a
  driver-installation step at all — a `udev` rule on Linux, a WinUSB binding on
  Windows.
- Windows needs a driver installed before libusb can open the device, and that
  driver has to be signed or installed through libwdi. This is the single
  largest source of setup friction in the project.

This also rules out iOS permanently. There is no AOA equivalent, and the
alternatives all need either a developer account or a jailbreak.

`scrcpy` remains the better tool for anyone who already has ADB set up. This
project is for the case where they do not.
