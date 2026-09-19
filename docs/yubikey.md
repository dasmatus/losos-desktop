# Hardware tokens

What a YubiKey — or any FIDO2 token — can do on this OS, what it cannot, and
why "unlock the system" is two different questions with two different answers.

Read `docs/systemd-inventory.md` for the components named here and
`docs/limits.md` for the ones that are absent.

## The software was already here; the kernel was not

`libcbor` and `libfido2` are pinned in `losos-10-base`, and systemd is built
`-Dlibfido2=enabled`, which is what gives `systemd-cryptenroll`,
`systemd-cryptsetup` and `systemd-homed` their FIDO2 support. `fido_id` and
`60-fido-id.rules` come out of the same build. Nothing about the userspace
needed adding.

What was missing sat one layer down. libfido2 talks to a token through
`/dev/hidraw` and through nothing else, and `CONFIG_HIDRAW` defaults to `n`.
`x86_64_defconfig` happens to set it; arm64's `defconfig` sets neither it nor
`CONFIG_USB_HID`. So the feature worked on one of the two architectures this
repository builds and was silently absent on the other — and the symptom is
identical on both sides of that line. `systemd-cryptenroll`, `homectl` and a
browser all report *no device found*, which reads as a dead key rather than as
a kernel option nobody stated. `recipes/10-systemd/linux/files/losos.config`
now names `CONFIG_HID`, `CONFIG_HIDRAW`, `CONFIG_HID_GENERIC` and
`CONFIG_USB_HID`, built in rather than modular so an unlock that happens before
switch-root does not depend on a module reaching the initrd.

That one change is also what makes a YubiKey's OTP mode work, which needs no
daemon at all: in that mode the key is an ordinary USB HID keyboard that types
a one-time password when you touch it, and `CONFIG_USB_HID` plus the
`CONFIG_INPUT_EVDEV` already in the fragment is the whole requirement.

## "Unlocking the system" splits in two

### Unlocking your user — this works

**On this OS a user *is* a LUKS volume.** There is no `useradd` in the image
and no line in `/etc/passwd`: a user is a `systemd-homed` record plus an
encrypted home image on the home partition, which is what survives an A/B root
replacement. `homed` is in the preset, and it is built with FIDO2 support.

So binding your login to a token is one command, run as yourself on a booted
system with the key inserted:

```sh
homectl update $USER --fido2-device=auto
```

`homectl create --fido2-device=auto` does the same at the moment the user is
made. From then on `systemd-homed` needs the token to open the home image, and
`pam_systemd_home` is what asks for it — so the same enrolment covers logging
in and coming back from a locked screen, because both are the same PAM
conversation against the same home area.

There is a catch, and it is not about tokens. **This tree ships no PAM
configuration.** Until it does, `pam_systemd_home` is not in any stack, so the
enrolment above is a thing the OS can store and nothing yet consumes.

That absence is also less stable than it looks. gdm's `default-pam-config`
option defaults to `autodetect`, which is `test -f /etc/arch-release` and four
siblings evaluated against the *build container* rather than the image: on the
current base none match and gdm installs no PAM files, but a `Containerfile`
that changes the base changes whose PAM stack ships, with nothing saying so.
The recipe therefore spells `-Ddefault-pam-config=none`, so the absence is
deliberate rather than a property of the container. That pin travels with the
change that makes it bite, which is whichever one moves the container base.

Shipping a PAM stack of this OS's own is the next piece of work, and it is what
turns the paragraph above from enrolled into enforced.

### Unlocking the disk at boot — there is no encrypted disk

This is the reading the question usually has, and here it has nothing to
attach to. `overlay/usr/lib/repart.d/20-root.conf.in` creates the root
partition with `Format=ext4` and no `Encrypt=`; the ESP is a FAT partition by
definition. Nothing on the disk is a LUKS volume except each user's home
image, which is the case above. A FIDO2 factor unlocks a LUKS volume — with no
LUKS volume there is no factor to enrol, and `systemd-cryptenroll` would have
nothing to point at.

`systemd-cryptenroll --fido2-device=auto /dev/<device>` works today against a
LUKS volume you made yourself, and the initrd already carries
`systemd-cryptsetup` and its generator, so a hand-made encrypted volume with
`fido2-device=auto` in `crypttab` would be unlocked at boot. That is a thing a
user can do, not a thing this image does.

Making the *root* partition encrypted is its own change and a larger one than
it looks: `systemd-repart` offers `Encrypt=key-file` and `Encrypt=tpm2` and no
FIDO2 mode, so first boot would have to create the volume with one factor and
enrol the token afterwards; the installer copies the root partition block for
block (`CopyBlocks=auto`), so what it copies would have to be the encrypted
image with the installing machine's key already in it; and `systemd-sysupdate`
replaces that partition wholesale, which means the enrolment has to survive an
A/B update or be re-done by it. None of that is blocked — it is a design, and
it belongs in its own change with `docs/boot.md` moving with it.

## In the browser

WebAuthn against a hardware key needs one thing from the OS: the browser,
running as you and not as root, has to be able to open the token's hidraw node.
systemd does all of it and this repository ships no udev rule of its own.
`60-fido-id.rules` runs `fido_id`, which reads the HID report descriptor and
sets `ID_SECURITY_TOKEN=1` on anything declaring the FIDO usage page;
`70-uaccess.rules` turns that property into an ACL for the user of the active
seat. Both, and `fido_id` itself, are now in the systemd inventory, because
`70-uaccess.rules` is generated only when `-Dlogind` **and** `-Dacl` are both
enabled and a feature that stops being generated is exactly the kind that goes
quiet rather than red.

Yubico's own `70-u2f.rules` is deliberately *not* shipped. It exists for
systems whose udev predates `fido_id`, and it works by listing USB vendor and
product ids — which means it needs a new entry for every key model ever
released. `fido_id` asks the device what it is instead. Adding the static list
on top would not make anything work that does not already.

**What this does not come with is a browser.** There is none in
`manifest/layers.yaml` and none anywhere in `recipes/`; see `docs/limits.md`.
So the device access is in place and untested by construction, and the first
browser to land here should be the thing that proves this section rather than
assuming it.

## Smartcard mode is not supported, and what it would cost

A YubiKey's PIV, OpenPGP and OATH applets speak CCID, not FIDO, and reaching
them needs `pcsc-lite` running `pcscd` plus the CCID driver — neither of which
is in this tree. `60-fido-id.rules` already tags a CCID interface
`ID_SMARTCARD_READER` and `70-uaccess.rules` already gives the seat user an ACL
on it, so the device access half is free; the daemon is not.

The cost is two new C packages in a CFI-hardened, musl cross build. `pcsc-lite`
is autotools and would need `--host` like everything else here
(`tools/gates/cross-configure.py` enforces it), and `ccid` needs `libusb` —
which this tree has, but in `losos-40-gnome` under fwupd, above the layer a
system daemon would want to live in. So it is a package move as well as two
additions, and `pcscd` is a long-running daemon that would need a unit, a
socket and a place in the preset. Worth doing if smartcard login or GPG-on-a-
key is wanted; not worth doing as a side effect of WebAuthn, which does not use
it.

`ykman`, Yubico's configuration tool, is Python and would drag a Python runtime
into the image; nothing here ships one. A key is configurable from any other
machine, which is the reason not to.

## libfido2 exports nothing under hidden visibility

`manifest/toolchain.yaml` carries a `drops: [visibility]` exception for
libfido2 and for libqrencode. Without it neither library exports a single
symbol, and this page is undone: systemd is built `-Dlibfido2=enabled`, and it
would link against an empty dynamic table.

libfido2 looks like it should not need this, which is the interesting part. It
ships a linker version script, `src/export.gnu`, naming 267 symbols -- exactly
the mechanism a library is supposed to use to control its ABI. But a version
script cannot re-export a symbol the compiler has already marked hidden:
`-fvisibility=hidden` writes `STV_HIDDEN` into the object, and `global:` has
nothing left to promote. libfido2 annotates no export anywhere in its sources
to make up for it -- zero `visibility` attributes in the pinned tarball, not
one. **So "ships a version script" does not clear a package on this axis. Only
annotations do.**

Measured rather than argued, on bytes whose SHA-256 matches `sources.lock`:

| build | dynamic functions defined | `fido_dev_open` |
|---|---|---|
| `-fvisibility=default` | 267 | present |
| `-fvisibility=hidden` | 0 | absent |

libqrencode is the same story without the version script -- 75 against 0 --
and systemd wants it for the recovery-key QR that `systemd-cryptenroll`
prints.

The exception is the narrow one on purpose. `drops: [cfi]` would also fix the
symbols, and it is the wrong trade here: of every library in this tree
libfido2 is the last one to unharden, because its input is CBOR arriving from
a device the user just plugged in. clang accepts the whole CFI scheme set,
cross-DSO included, at `-fvisibility=default`; it objects only to the flag
being absent altogether. So every check stays on. What is given up is the
narrower guarantee that no exported symbol is interposed at load time.

The alternative -- patching libfido2 to annotate 267 symbols upstream does not
annotate, and re-deriving that patch at every version bump -- is what this
avoids. Where upstream has an export macro already, patching it is the better
answer and the tree does exactly that for zlib.

## Summary

| Want | State | What it needs |
|---|---|---|
| WebAuthn in a browser | device access ready | a browser (`docs/limits.md`) |
| OTP mode (touch-to-type) | works with this change | nothing |
| Unlock your home / lock screen | enrolment works, nothing consumes it | a PAM stack for this OS |
| Unlock an encrypted root at boot | no encrypted root exists | `Encrypt=` in repart, and the install and update paths to match |
| PIV / OpenPGP / OATH | not supported | `pcsc-lite`, `ccid`, and `libusb` moved down a layer |
