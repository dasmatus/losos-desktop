# The installer

The pm tree's installer was `systemd-sysinstall`, new in systemd v261.
nixpkgs 26.05 shipped 260.4, so the installer does the same job by hand with
the same tools. nixos-unstable now carries 261, so sysinstall is available;
the installer has not been moved onto it yet. It is its own small live system, `nixos/installer/`, on its own ISO,
and `nixos/modules/installer.nix` evaluates it from the OS's configuration.
A machine that keeps Windows uses the [Windows installer](windows-installer.md)
instead, which installs the same release beside it.

It boots by UEFI only: the ISO's appended FAT partition holds the installer's
UKI as `EFI/BOOT/BOOT<ARCH>.EFI`, with no bootloader in front of it, and the
initrd mounts the ISO by its volume label and the Nix store from a squashfs
on it. It carries `wpa_supplicant` and no NetworkManager. networkd runs DHCP
on every physical wired port, built in or USB, as soon as a cable is in, at
boot or later, and prefers it over Wi-Fi when both are up; a machine with a
cable in is online with nothing asked. Otherwise derisk talks to
`wpa_supplicant` over its control socket to scan for and join a Wi-Fi
network, which networkd then runs DHCP on too. Of the firmware NixOS would add, only
`linux-firmware` is on the medium, because most Wi-Fi cards do not start
without it.

The installer is `derisk installer` on tty1, a logind session run as root,
with a root shell on tty2 for anything it does not cover. It draws the same
pages first-boot setup is made of, so it works with a mouse, a touchscreen or
a phone-sized screen. It starts `losos-installer serve` (`src/losos-installer`)
as its backend and talks to it in JSON lines on stdin and stdout: derisk owns
the screen and the network, the backend owns the disks. The Network page
shows each wired port as having no cable, getting an address, or online, and
is skipped when the machine is already online. Then the backend lists the
disks, leaving out the one the ISO booted from and refusing any disk it did
not list, and the installer names the disk, its size and the source once more
behind an "Erase and install" button before anything is touched. Then:

1. `systemd-repart --empty=force` lays out the ESP, with systemd-boot and
   `loader.conf` copied in, and both `/usr` slots at full size, labelled
   `_empty` (sysupdate refuses a disk with only one). These are `disk.nix`'s
   own definitions, so the disk is laid out the way the
   installed system expects to find it.
2. `systemd-sysupdate update` fills them with the channel's newest release,
   from the same URL and through the same transfers as `update.nix`, aimed at
   the chosen disk instead of `auto` and at the new ESP, mounted under
   `/run/losos-installer`. With `losos.update.pubring` set, the installer
   checks `SHA256SUMS.gpg` against it as an update does.
3. The machine reboots into the installed system, whose first boot creates
   root, `/home` and swap from `disk.nix`, the path an image written
   with `dd` takes.

So an install is an update into an empty slot: what lands on the disk is
what the channel serves at the time, not a copy of the OS on the medium that
could have gone stale. The image used to be its own installer, with a second
UKI that booted it into a tmpfs root and copied its own `/usr` with
`CopyBlocks=`; that is gone, and so is the `losos.install` condition it
needed in `disk.nix`. When nixpkgs reaches v261, the installer should be
measured against `systemd-sysinstall` again.

A machine that cannot reach the channel installs from a release disk
instead: any filesystem labelled `LOSOS-RELEASE` holding a release's
`SHA256SUMS`, `SHA256SUMS.gpg` and the `/usr`, verity and UKI files they
list. udev mounts it at `/run/losos/release` whenever it appears, and the
installer then turns the transfers' sources into local files. sysupdate
verifies signatures only on downloads, so the installer checks the release
first: `SHA256SUMS` against the signing key with `gpg`, as `systemd-pull`
does, then every file on the disk against its line there, refusing a file
not listed. The case it exists for is a VM in a network that re-signs TLS
with its own authority, where the channel's certificate never verifies; with
the disk attached the installer does not need the network at all: the
backend's `hello` names the disk as `release`, and derisk then skips its
Network page.
