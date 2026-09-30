# LosOS Desktop on NixOS

`flake.nix` and `nixos/` describe the same operating system as the pm recipes
under `recipes/`: a GNOME desktop that is systemd end to end, image-based,
updated by `systemd-sysupdate`, with every user a `systemd-homed` LUKS volume.
What changes is how it is built. pm compiles ninety packages from pinned
tarballs; this takes them from nixpkgs 26.05, pinned in `flake.lock`, and
spends its effort on how they are wired together instead.

The pm tree is still here and is the primary build: it is self-contained,
compiling every package from a tarball pinned by SHA-256, and this is not.
Nothing in `nixos/` reads from it except the two programs this repository
writes itself, `losos-security` and `losos-swap`, which are built from the same
sources under `recipes/10-core/`. pm itself ships in this image too, as the
system manager, pinned to the same commit CI builds the pm tree with.

## What this trusts from outside

Everything the flake builds from, besides this repository:

- **nixpkgs 26.05**, the channel tarball from `channels.nixos.org`, pinned by
  `narHash` in `flake.lock`. Its expressions are read, not trusted blindly:
  a different tarball fails the hash.
- **cache.nixos.org.** Every package nixpkgs has already built -- the kernel,
  systemd, GNOME, the compilers -- is downloaded as a binary rather than
  compiled. The store paths are fixed by the pinned expressions, and each
  download is checked against the cache's signing key, but the binaries
  themselves were compiled by NixOS's build farm, not here. This is the
  trade the pm build does not make. `nix build --option substitute false`
  compiles everything locally instead, at the cost of a very long build.
- **The upstream sources nixpkgs fetches** for anything the cache lacks, each
  pinned by hash in nixpkgs.
- **pm**, cloned from `github.com/dichhead/pm` at a pinned commit and hash
  (`nixos/pkgs/pm.nix`), and **crates.io**, for its and losos-security's
  dependencies, each pinned by `Cargo.lock` and checked by hash.
- **In CI only:** the `cachix/install-nix-action` action that installs nix.

## Building

```sh
nix build              # the disk image: dd it to a disk and boot
nix build .#installer  # the same image, booting the installer by default
nix build .#release    # what a release uploads, with SHA256SUMS
nix run .#vm           # boot the image in QEMU with UEFI firmware
nix flake check        # both architectures, losos-security's tests, and
                       # (on a builder with KVM) a VM boot test
```

Nothing is compiled that nixpkgs has already built except the two programs
above and the configuration itself, so a build is mostly a download from
cache.nixos.org. The image is built by `systemd-repart` inside the build
sandbox; it needs no loop device, no root and no KVM. Only the VM test needs
KVM.

## Driving the flake from pm

pm stays the system manager here too, so a pm build file can build and check
the flake: `nix build`, `nix flake check`, `nix eval` and the other commands
that read or build are named by the `losos-nix` plugin (`plugins/README.md`),
which pm's fingerprint table otherwise refuses (C2). Two things about pm's
jail shape such a step. `/nix/store` is mounted read-only and the daemon
socket not at all, so the step builds into a store of its own, `nix --store
/build/nix ...`. And a step that says `--offline` gets no network, so the jail
enforces what the step claims; without it the step, and so the whole build
file (C8), is on the host network, because evaluating fetches the inputs and
building substitutes from cache.nixos.org. `nixos-rebuild`, `nix profile`,
`nix run` and `nix flake update` are recognised and deliberately left
unclassified: this OS is updated by systemd-sysupdate, never by switching a
generation, and a pin moves by hand.

## Where the OS lives

The one structural change is where the operating system is.

In the pm tree it was the root partition: `/usr` and `/etc` together, replaced
wholesale by sysupdate, with `/home` beside it. Here the operating system is
the Nix store, on its own `/usr` partition, protected by dm-verity, and the
root partition holds only state: `/etc`'s writable layer, `/var`, the journal.

```
ESP          systemd-boot, the UKIs                  image
usr-verity A dm-verity hash tree for slot A          image
usr A        the Nix store, erofs                    image, grown on first boot
usr-verity B                                         first boot, label _empty
usr B        where the next update is written        first boot, label _empty
root         state only                              first boot, FactoryReset=yes
home         homed's LUKS images                     first boot, FactoryReset=yes
swap         RAM-sized, random key each boot         first boot (losos-swap)
```

That buys two things the pm tree wrote down as limits:

- **`/usr` is immutable for real.** The pm tree's `docs/boot.md` named
  `systemd-veritysetup` in the boot chain but never built a verity partition.
  Here the root hash is `usrhash=` on the UKI's command line, so a changed
  block in `/usr` fails verification, and a different `/usr` needs a
  different UKI.
- **A factory reset resets the machine.** `docs/factory-reset.md` explained
  that marking only `/home` left `/etc` and `/var` behind, and that the fix was
  to get state off the partition the OS lives on. That is now the layout, so
  root is marked too, and a reset returns the machine to exactly what the image
  contained.

## What maps to what

| pm tree | NixOS | Notes |
|---|---|---|
| `recipes/10-systemd/linux`, `losos.config` | nixpkgs' kernel | `CONFIG_HIDRAW` is already set for every architecture in nixpkgs' `common-config.nix`, so FIDO2 needs no fragment |
| `recipes/90-image/losos-image` (mkosi), `plugins/` | `nixos/modules/image.nix` (`image/repart.nix`) | the same `systemd-repart`; no fingerprint table to get past, so no plugins |
| `mkuki.py`, `files/cmdline` | `boot.uki`, verity-store module | the UKI carries `usrhash=` |
| `overlay/usr/lib/repart.d/` | `systemd.repart.partitions` in `disk.nix` | runs in the initrd, as before |
| `overlay/usr/lib/sysupdate.d/` | `systemd.sysupdate.transfers` in `update.nix` | UKI plus both `/usr` halves |
| `overlay/usr/lib/repart.sysinstall.d/`, `systemd-sysinstall` | `installer.nix` | see below |
| `overlay/usr/lib/systemd/system-preset/10-losos.preset` | the module options themselves | on NixOS a unit is enabled by being wanted, not by a preset |
| `overlay/usr/lib/sysusers.d/losos.conf` | `systemd.sysusers.enable` over `users.users` | the upstream tool, not NixOS's perl script |
| `overlay/usr/lib/tmpfiles.d/losos.conf` | `systemd.tmpfiles.settings` in `services.nix` | |
| `overlay/usr/lib/systemd/network/20-wired.network` | `systemd.network.networks."20-wired"` | same match and settings |
| `overlay/etc/crypttab`, `overlay/etc/fstab` | `environment.etc.crypttab`, `swapDevices` | |
| `user@.service.d/10-oomd.conf` | `systemd.oomd`, `systemd.services."user@"` | |
| `losos-security.service`, its D-Bus files | `services.nix` | same sandbox, line for line |
| `50-losos-factory-reset.rules` | `security.polkit.extraConfig` in `disk.nix` | same rule |
| `losos-selftest.service`, `losos-ota-test.service` | `testing.nix` | same kernel command line conditions |
| `recipes/10-core/losos-release` | `system.nixos.distroId`, `system.image.*` | NixOS writes os-release itself, with `IMAGE_VERSION` |
| `manifest/architectures.yaml` | `losos.arch` in `options.nix` | x86_64 and aarch64 |
| `tools/configure --version --channel` | `losos.version`, `losos.channel` | the flake derives the version from the commit date |
| `tools/vm-test` | `nixos/tests/boot.nix` | boots the real image under UEFI |
| pm, the system manager | pm, the system manager (`pm.nix`) | the same pm commit in both images |

## Everything systemd, and the exceptions

Every row of the table in `README.md` has a counterpart here. The places a
non-systemd component remains are the places systemd has no equivalent, or
NixOS requires one:

- **nsncd.** The `homed` module asserts `services.nscd.enable`: NixOS routes
  NSS through a forwarder so glibc can find `nss-systemd` in the store. It is
  stateless and holds no idea of its own about who exists.
- **nftables.** systemd has no packet filter. The pm tree shipped none at all;
  this keeps NixOS's firewall and opens mDNS and LLMNR for resolved.
- **A PAM stack.** The pm tree shipped none, and `docs/limits.md` said nothing
  in the image could authenticate anyone. NixOS generates one, with
  `pam_systemd_home` in all four management groups -- the condition for a
  login to open the home area rather than succeed and find it locked.

Replaced by the systemd equivalent where NixOS would otherwise pick something
else: `run0` instead of sudo (with a `sudo` alias that refuses sudo's flags);
networkd instead of NetworkManager, which GNOME turns on by default; resolved's
mDNS instead of avahi; `systemd-sysusers` instead of the perl user script; the
`/etc` overlay instead of activation scripts writing files; homed's first-boot
wizard instead of no way to create the first user at all.

## The installer

The pm tree's installer was `systemd-sysinstall`, new in systemd v261. nixpkgs
26.05 ships 260.4, so `installer.nix` does the same three things by hand with
the same tools: it asks for a disk with `systemd-ask-password`, runs
`systemd-repart` with `CopyBlocks=auto` for the `/usr` pair and `CopyFiles=`
for the ESP, and reboots. The installed system's first boot then creates slot
B, root, `/home` and swap from `disk.nix`, which is the same path an image
written with `dd` takes.

The installer UKI is the ordinary one with `root=tmpfs losos.install
systemd.unit=losos-install.target` appended. systemd takes the last `root=`,
so the installer boots with a tmpfs root and never repartitions its own medium.
When nixpkgs reaches v261, `installer.nix` should shrink to sysinstall's
drop-in and the repart definitions.

## Releases and GHCR

`nix build .#release` writes a flat directory:

```
losos-desktop_<v>_x86_64.efi                 the UKI sysupdate installs
losos-desktop_<v>_usr-x86-64.raw.xz          /usr for a sysupdate slot
losos-desktop_<v>_usr-x86-64-verity.raw.xz   its hash tree
losos-desktop_<v>_x86_64.raw.xz              the disk image
losos-desktop_<v>_x86_64-installer.raw.xz    the installer medium
losos-desktop_<v>_x86_64.qcow2               the disk image, for a VM
SHA256SUMS
```

It is the same shape as the pm tree's `release/`, so `just release-images --
publish --source result` pushes it to GHCR unchanged, as PR #79 set up. The
GitHub release remains the URL sysupdate reads, because systemd-sysupdate
cannot fetch from an OCI registry.

The `/usr` halves are cut out of the finished disk image at the offsets repart
reported, not built a second time, so the bytes sysupdate installs are the
bytes the image boots.

## What is not done

- **pm on a NixOS host.** pm's jail mirrors the host's `/bin`, `/lib` and
  `/usr`, and additionally mounts `/nix/store` and the store-backed `PATH`
  directories read-only, so a step whose tools come from the store runs: a
  `losos-nix` step (`nix eval`, `nix hash`) ran that way in pm's jail on a
  Nix-provisioned host. What still finds nothing is a recipe that names FHS
  paths, `/bin/cat` or a compiler under `/usr`, and every recipe in this tree
  does; in this image `/usr` is the Nix store's partition and `/bin` holds
  only `sh`. So pm itself runs here (`pm --help`, `pm source-path`, signing,
  `explain`), and the boot test checks it is installed, but this tree's
  recipes do not build on it. For the same reason seven of pm's test targets
  are skipped in the nix build (the list is in `nixos/pkgs/pm.nix`), and the
  wasm plugins are not installed in the image.
- **CI does not publish the flake's release.** `ci.yml` builds and checks it;
  the release is the pm build's.

- **Signing.** `losos.update.pubring` is unset, so sysupdate installs updates
  without verifying `SHA256SUMS.gpg`, and the build warns. Secure Boot signing
  of the UKI is not done either; as in the pm tree, a key belongs to whoever
  owns the machine.
- **The two gnome-control-center patches.** They are written against
  gnome-control-center 51.0, and nixpkgs 26.05 carries 50.4, which moved the
  System panel to Blueprint; neither applies. The factory reset is reachable
  through `systemctl start factory-reset.target` and the Varlink API, and the
  security report answers on the bus, but neither has a button in Settings.
- **Wi-Fi.** As in the pm tree: networkd handles wired links, and GNOME's
  network panel is inert without NetworkManager. `iwd` would be the smallest
  non-systemd addition that fixes it.
- **systemd-boot updates.** `systemd-boot-update.service` copies a new
  bootloader from `/usr/lib/systemd/boot`, which NixOS does not have. The
  bootloader on the ESP is the one the image shipped.
- **sysext.** Extensions merge into `/usr`, which here holds little but the
  Nix store, so an extension can add a program and cannot replace one.
- **Nothing has booted yet.** The disk image and the installer image build,
  with the layout above, the installer UKI's command line as described, and a
  `usrhash=` equal to the root hash repart reported. The VM test evaluates
  and needs KVM, which the machine this was written on did not have; so the
  first-boot repart run, the gpt-auto root, and the installer have not been
  seen working. `nix build .#release` was not completed there either, for
  lack of disk space rather than an error.
