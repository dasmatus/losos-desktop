# Installing

The installer is `systemd-sysinstall`, which systemd gained in v261. Before it
existed this repository shipped no installer at all and said so: the "install"
was writing the root filesystem onto a disk by hand and letting
`systemd-repart` create the rest on first boot. That works, and it is not
something to hand anyone who has not read `docs/boot.md`.

## Two UKIs, one root filesystem

The installer is not a separate image. It is the same image, booted with one
extra word on its kernel command line:

| Artifact | `.cmdline` | What boots |
|---|---|---|
| `losos.efi` | `quiet rw systemd.machine_id=firstboot console=tty0` | the desktop |
| `losos-installer.efi` | the same, plus `losos.install` | `systemd-sysinstall` |

`overlay/usr/lib/systemd/system/systemd-sysinstall.service.d/10-losos.conf`
conditions the unit on that word and gives it an `[Install]` section — systemd
ships the unit without one, because upstream cannot know which images are
installer media. The preset enables it on every image; the condition decides.

A failed condition is not a failed unit, so on an installed system it is
enabled, skipped and silent. That matters more than it looks: the unit carries
`FailureAction=halt`, so a unit that could *fail* on a normal boot would halt
the machine.

Both UKIs are built in one step from the same stub, kernel, initrd and
os-release, so the installer and the thing it installs cannot be built from
different inputs. `just check` asserts the word is actually in the installer
UKI — without it, that image boots to a desktop and installs nothing, which is
a failure nobody notices until after they have written the USB stick.

## What it does

From `systemd-sysinstall(8)`, in order: prompt for the target disk, check it is
big enough, offer to erase it or to add alongside what is there, offer to
register the new OS in the firmware boot menu, show a summary and ask for
confirmation, seal the current locale/keymap/timezone into TPM-locked
credentials for the target, run `systemd-repart` against it, `bootctl link` the
kernel onto its ESP, `bootctl install` systemd-boot, and reboot.

It does not ask for a user account or a root password. Neither would mean
anything here: human users are `systemd-homed` records created on the installed
system, and there is no `/etc/passwd` entry to write. See "Users do not exist
until someone makes one" in `docs/boot.md`.

## The definitions directory is the install

`systemd-sysinstall` partitions with `systemd-repart` reading
`/usr/lib/repart.sysinstall.d/`, falling back to `/usr/lib/repart.d/` when that
is empty. This OS ships both, and the difference between them is one line:

```
CopyBlocks=auto
```

on the root partition. It copies the root partition the installer is running
from, block for block, onto the newly created one. So installing is duplicating
the medium you booted, and there is no third artifact — no squashfs, no OS
image inside the installer image — that could drift out of step with what
actually boots.

`/usr/lib/repart.d/` keeps `Format=ext4` and copies nothing, because it runs in
the initrd against the disk the system is *already* booted from. Writing that
same `Format=` into the installer's root definition would write a fresh
filesystem over the image just copied in, and the failure is a target disk that
partitions perfectly and boots to nothing. The two directories are kept
separate rather than unified behind a condition for exactly that reason.

## Making the medium

`just build` produces `losos.iso`, and writing it to a USB stick is the whole
procedure:

```sh
dd if=losos-desktop_<version>_x86_64.iso of=/dev/sdX bs=4M status=progress conv=fsync
```

It is a hybrid image, so the same file also boots from a DVD and can be handed
to QEMU as a CD. Firmware booting the DVD follows the El Torito catalog to the
ESP; firmware booting the USB stick finds the ESP through the GPT. Both are the
same bytes, and the root partition the installer copies is on the medium beside
them — which is what `CopyBlocks=auto` above means in practice.

`losos.qcow2` is the same OS as a disk that boots straight to the desktop, for
a VM. It runs the installer on nothing; it is already installed.

The older path still works and needs no image at all: write
`losos-rootfs.tar.xz` onto a disk's root partition and `losos-installer.efi`
onto its ESP at `EFI/Linux/`, or boot the installer UKI however your firmware
prefers. The installer needs nothing else: `bootctl` and `systemd-repart` are
in the image already, and so is the OS it is about to copy.

`docs/images.md` says how the media are built, and why by this repository
rather than by `xorriso` and `mkfs`.
