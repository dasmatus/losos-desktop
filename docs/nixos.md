# LosOS Desktop on NixOS

`flake.nix` and `nixos/` are the operating system. It is a derisk desktop that
uses systemd for everything it can, installs as an image, updates with
`systemd-sysupdate`, and makes every user a `systemd-homed` LUKS volume.
nixpkgs from the `nixos-unstable` channel, pinned in `flake.lock`, provides
the stock glibc package set,
and this repository decides how those packages fit together. Builds use
cache.nixos.org by default; the project's own cache can provide project-built
paths as well.

This repository used to build the same OS a second time, as a from-source
distribution of ninety-odd pm recipes. The table at the end of this file says
where each of its pieces went. One thing went with it and has no replacement.
The recipe tree compiled every package with cross-DSO control-flow integrity
and ThinLTO, from a toolchain it configured itself. nixpkgs on this platform
enables fortify, the stack protector, stack clash protection, RELRO, bind-now
and zeroed call-used registers, going by a derivation's
`NIX_HARDENING_ENABLE`, and no CFI. The only code of this repository's own
left is `src/losos-security`, `src/losos-swap` and `src/losos-installer`,
which `nixos/pkgs/` builds. pm ships in the image as the system manager.

## What this trusts from outside

Everything the flake builds from, besides this repository:

- **nixpkgs `nixos-unstable`**, the channel tarball from `channels.nixos.org`,
  pinned by
  `narHash` in `flake.lock`. Its expressions are read, not trusted blindly:
  a different tarball fails the hash.
- **Stock package binaries from cache.nixos.org.** nixpkgs' normal glibc
  package set is used without libc-specific package overrides, so available
  package closures can be substituted rather than rebuilt locally.
- **Upstream sources nixpkgs fetches**, each pinned by hash in nixpkgs, for
  packages that must be built locally.
- **Fixed-output inputs in the build closure.** CI preloads these from
  cache.nixos.org where available (`tools/nix-fetch-sources`) to avoid making
  a cold build depend on every upstream being available. The selection includes
  source archives and can also include pinned bootstrap tools or upstream
  prebuilt compilers; it is not source-only. Each copied flat or NAR output is
  checked against its derivation's hash, and outputs using other hash methods
  are rejected.
- **pm**, cloned from `github.com/dichhead/pm` at a pinned commit and hash
  (`nixos/pkgs/pm.nix`), and **crates.io**, for its, losos-security's and
  losos-installer's dependencies, each pinned by `Cargo.lock` and checked by hash.
- **Purism's adaptive GTK patches**, copied into
  `nixos/pkgs/patches/gtk3` and `gtk4` from PureOS's packaging
  (source.puri.sm, `Librem5/debs/gtk4` and `sebastian.krzyszkowiak/gtk`,
  branch `pureos/latest`). They are reviewed source in this repository, not
  fetched at build time.
- **Device firmware**, from nixpkgs' `linux-firmware`, pinned by hash like any
  source. It is prebuilt by the hardware vendors, and nothing can compile it.
  `hardware.nix` ships it because amdgpu and nouveau cannot start current
  GPUs without it.
- **derisk**, cloned from `github.com/dasmatus/derisk` at a pinned commit and
  hash (`nixos/pkgs/derisk.nix`), with its crates, mcsapi among them, pinned
  by its `Cargo.lock` and `cargoHash`. It is the desktop, the display manager
  and the portal backend.
- **Flathub**, for apps installed after the fact. Its repo file, with the
  signing key every install is checked against, is
  `nixos/modules/flathub.flatpakrepo` in this tree; the image adds the remote
  from that copy at boot and never fetches the key. The apps themselves are
  Flathub's builds, sandboxed by Flatpak and verified against that key, and
  nothing in the image depends on one.
- **The project's binary cache** in GHCR, through `proxy/` (below): optional
  paths the CI job stores and signs. A client checks every narinfo against
  the public key it was told to trust, so the proxy and GHCR carry bytes but
  cannot vouch for them.
- **In CI only:** the official Nix installer from `releases.nixos.org`,
  pinned by version. Nix is what builds Nix, so this binary is the one
  bootstrap; `.#nix` is the nixpkgs Nix package for a build host after it.

## Building

Image-producing outputs require `LOSOS_PROXY_URL` to point at the deployment
that serves updates. Since the flake reads this environment variable, pass
`--impure` when building an image, installer, QCOW2, release, UKI, or VM.

```sh
nix build --impure              # the disk image: dd it to a disk and boot
nix build .#installer --impure  # the installer ISO (below)
nix build .#release --impure    # the files CI publishes through the proxy
nix run .#vm --impure           # boot the image in QEMU with UEFI firmware
nix flake check        # both architectures, losos-security's and
                       # losos-installer's tests, and
                       # (on a builder with KVM) a VM boot test
```

The stock nixpkgs closure is substituted from cache.nixos.org when available;
the image and project-specific packages are built locally unless present in
the project's cache (below). The image is built by `systemd-repart` inside
the build sandbox; it needs no loop device, no root and no KVM. Only the VM
test needs KVM.

## glibc and stock packages

`flake.nix` selects nixpkgs' standard `x86_64-linux` and `aarch64-linux`
platforms. The NixOS configuration uses nixpkgs' stock glibc packages; its
only overlay adds this repository's own packages and patches GTK and Qt
(below, "GTK and Qt on a phone"). In particular, ShellCheck
and the package-generated documentation retain nixpkgs' defaults. glibc NSS
lets GDM and ordinary account lookups see systemd-homed users through
`nss-systemd` and the configured nscd forwarder.

## The memory allocator

On a PC every dynamically linked process allocates through GrapheneOS's
[hardened_malloc](https://github.com/GrapheneOS/hardened_malloc), nixpkgs'
`graphene-hardened-malloc` in its default (not light) configuration.
`nixos/modules/allocator.nix` names it in `/etc/ld-nix.so.preload`, which
nixpkgs' glibc reads in place of `/etc/ld.so.preload`. derisk is built with
its `hardened-malloc` feature, mcsapi's `mcsapi-hardened-malloc` crate as its
Rust global allocator: the same `libhardened_malloc.so`, mapped once, with
Rust's frees going through `free_sized` so a wrong length aborts.

The module writes the preload itself rather than setting NixOS's
`environment.memoryAllocator.provider`, which preloads a copy of the library
from another store path. The library has no `DT_SONAME`, so `ld.so` would map
that copy and derisk's as two allocators. Halium does not import the module:
see "What is not done".

## Binary cache

`proxy/` is a Vercel edge function whose decisions are a WebAssembly module
compiled from Rust (`proxy/src/lib.rs`); the JavaScript around it only
fetches. It serves two things from this project's GHCR namespace:

- **A Nix binary cache.** `tools/nix-cache-push` pushes each store path as
  one OCI artifact, `nix-cache:<store hash>`, holding its signed narinfo and
  its NAR. The proxy answers `/<hash>.narinfo` (with its `URL:` pointed back
  at itself) and redirects `/nar/<hash>/<file>` to GHCR's blob storage, so a
  NAR never passes through it. A real `nix copy` pulled and verified a signed
  path through it against a fake registry; the tests are in `proxy/test/`.
- **Updates.** `/updates/<channel>/<arch>/<file>` serves a file of
  `images:<channel>-<arch>`, which CI's publish job moves to each release
  that passed verification. CI sets `losos.update.baseUrl` to
  `<proxy>/updates/<channel>/<arch>/`, so sysupdate fetches manifests and
  images through the proxy. The GitHub nightly release is announcement-only:
  GitHub rejects release assets of 2 GiB or larger.

To use it for a build:

```
substituters = https://cache.nixos.org/ https://<proxy>/
trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= <the public half of NIX_CACHE_SIGNING_KEY>
```

Keep cache.nixos.org in the substituter list when adding the project cache.
Without the optional project cache, Nix's default cache.nixos.org remains
enabled.

What has to be set up once, outside the repository:

1. **Vercel**: a project with root directory `proxy/` and environment
   variable `GHCR_REPOSITORY=dasmatus/losos-desktop`. `vercel.json` has the
   rest. Its build installs a pinned Rust with rustup when absent and adds
   the WebAssembly target when Rust is already installed.
2. **GHCR**: the `losos-desktop/nix-cache` and `losos-desktop/images`
   packages public, once CI has created them; or a read-only token in
   Vercel as `GHCR_TOKEN`, with its owner's GitHub login as `GHCR_USERNAME`.
3. **A signing key**: `nix key generate-secret --key-name losos-desktop-1`,
   stored as the secret `NIX_CACHE_SIGNING_KEY`; its public half, from `nix
   key convert-secret-to-public`, as the variable `NIX_CACHE_PUBLIC_KEY`.
4. **`LOSOS_PROXY_URL`**, the deployment's URL without a trailing slash, as a
   repository variable. CI requires it and uses it both for Nix cache
   substitutions and as the base of each image's update URL. Flake evaluation
   reads this environment variable, so CI uses `--impure` when building,
   checking and collecting the release. CI uploads its signed cache paths to
   GHCR whenever `NIX_CACHE_SIGNING_KEY` is configured, even if this URL is
   unset for a non-CI build; the proxy can serve them once configured.
5. **The update signing key**, which every image trusts and every release
   is signed with. Make it on a machine you trust, with no passphrase,
   because CI signs unattended:

   ```
   export GNUPGHOME=$(mktemp -d)
   mkdir -p nixos/keys
   gpg --batch --pinentry-mode loopback --passphrase '' --quick-gen-key 'LosOS Desktop updates' ed25519 sign never
   gpg --armor --export > nixos/keys/update-signing.asc
   gpg --armor --export-secret-keys   # paste into the secret UPDATE_SIGNING_KEY
   rm -rf "$GNUPGHOME"                # once the secret and an offline copy are saved
   ```

   Commit `nixos/keys/update-signing.asc`; `losos.update.pubring` picks it
   up and turns sysupdate's verification on. Put the key's fingerprint
   (`gpg --show-keys --with-colons nixos/keys/update-signing.asc`) in
   `UPDATE_SIGNING_FPR` in `ci.yml`. The `push` job fails without the secret,
   or with a secret for a different key, rather than publish a release the
   images would refuse. Keep an offline copy of the secret half: an image
   only ever trusts the key it shipped with, so a lost key means every
   installed machine stops taking updates until it is reinstalled.

Before setting `LOSOS_PROXY_URL`, check the public deployment without a
Vercel login: `/nix-cache-info` must return the cache metadata, an absent
store hash must return 404 rather than `502 token: 403`, and a published
release's `/updates/<channel>/<arch>/SHA256SUMS` must be readable. A ready
deployment alone does not prove GHCR access works. The variable also changes
the pm image's update source, so leave it unset while the registry is
inaccessible. Vercel deployment protection must allow anonymous clients.

## pm's plugins

`nixos/pkgs/pm-plugins.nix` builds the same seven components `just plugins`
builds for the pm tree, this repository's four and pm's own `sysext`,
`sysupdate` and `systemd`, and the image carries them in
`/run/current-system/sw/share/pm/plugins`. pm loads plugins only from a user's
own `~/.config/pm/plugins`, and only signed by a key that user trusts, so the
image offers them and trusts them for no one:

```
install -Dm644 -t ~/.config/pm/plugins /run/current-system/sw/share/pm/plugins/*.wasm
for p in ~/.config/pm/plugins/*.wasm; do pm sign "$p"; done
```

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
building substitutes from the project's cache. `nixos-rebuild`, `nix profile`,
`nix run` and `nix flake update` are recognised and deliberately left
unclassified: this OS is updated by systemd-sysupdate, never by switching a
generation, and a pin moves by hand.

## nixpkgs in a pm build

All of nixpkgs is reachable from a pm build file, at the revision
`flake.lock` pins and built for the same stock glibc platform as this OS.
The flake's `legacyPackages` is the image's own package set, so
`.#ripgrep` is nixpkgs' ripgrep, and the same store path the image would carry.
`.#pm-payloads.ripgrep` is what a pm package can actually hold
(`nixos/pkgs/pm-payloads.nix`): the same package from `pkgsStatic`, every output
merged into `usr/bin`, `usr/sbin`, `usr/libexec` and `usr/share`.

Static, because pm extracts a package at `/pkg` and runs it in a jail whose
`/nix/store` is the host's or nothing: a dynamic program names its loader and
libraries by store path. The payload sets `allowedReferences = [ ]`, so Nix
itself refuses one in which any file still names a store path, and says which.
`gnugrep` is refused that way, because `egrep` is a script whose shebang is
bash's store path. That is the check working: such a package could not run
from `/pkg`.

A build file that installs one:

```yaml
name: ripgrep
version: ['15', '1', '0']
dependencies: []
steps:
- stage: Build
  dl_urls: null
  name: build
  run:
  - nix --store /build/nix build --out-link /build/payload github:dasmatus/losos-desktop/<commit>#pm-payloads.ripgrep
- stage: Install
  dl_urls: null
  name: stage
  run:
  - find /build/nix/nix/store -maxdepth 1 -type d -name *-pm-payload-ripgrep-* -exec cp -rT {}/usr /dest/usr ;
```

The second step is `find` rather than `cp /build/payload/usr`, because the
out-link points at `/nix/store/...` and in pm's jail that is the host's store,
not the one at `/build/nix` the step built into. There is no shell, so the
pattern reaches `find` unexpanded.

`losos-nix` holds the flake reference to the same rule as a pin
(`plugins/README.md`): a commit, a path, or a URL with `narHash=` is classified;
`nixpkgs#ripgrep`, `github:NixOS/nixpkgs/nixos-26.05`, `<nixpkgs>` and anything
else that resolves to whatever is newest gets no verdict, and pm refuses the
build file before it runs.

Untested beyond evaluation and the payload builder itself: both architectures
evaluate `.#pm-payloads.<name>`, and the builder, run against nixpkgs' own
cached static packages, produced a static `hello` and refused `gnugrep`. No pm
build has run the steps above, and CI does not build any payload, so the first
one of each package compiles its whole static closure.

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

- **`/usr` is immutable for real.** The pm tree's boot documentation named
  `systemd-veritysetup` in the boot chain but never built a verity partition.
  Here the root hash is `usrhash=` on the UKI's command line, so a changed
  block in `/usr` fails verification, and a different `/usr` needs a
  different UKI.
- **A factory reset resets the machine.** The pm tree's factory-reset notes explained
  that marking only `/home` left `/etc` and `/var` behind, and that the fix was
  to get state off the partition the OS lives on. That is now the layout, so
  root is marked too, and a reset returns the machine to exactly what the image
  contained.

## What maps to what

| pm tree (removed) | NixOS | Notes |
|---|---|---|
| `recipes/10-systemd/linux`, `losos.config` | nixpkgs' kernel, `hardware.nix` | nixpkgs' `common-config.nix` already sets what systemd needs, `CONFIG_HIDRAW` included. zswap is built in and off, and `boot.zswap.enable` turns it on |
| `recipes/10-systemd/nvidia-open` | nouveau, in nixpkgs' kernel | not configured; Mesa's NVK uses nouveau. `hardware.nix` says why |
| `recipes/00-*` through `30-gnome`, `manifest/`, `share/` | nixpkgs | every package the recipes built. Their build-system patches existed only for the CFI toolchain |
| `recipes/30-gnome/gnome-control-center` patches | `nixos/pkgs/patches/gnome-control-center/` | kept and not applied, see "What is not done" |
| `recipes/90-image/losos-image` (mkosi), `plugins/` | `nixos/modules/image.nix` (`image/repart.nix`) | the same `systemd-repart`; no fingerprint table to get past, so no plugins |
| `mkuki.py`, `files/cmdline` | `boot.uki`, verity-store module | the UKI carries `usrhash=` |
| `overlay/usr/lib/repart.d/` | `systemd.repart.partitions` in `disk.nix` | runs in the initrd, as before |
| `overlay/usr/lib/sysupdate.d/` | `systemd.sysupdate.transfers` in `update.nix` | UKI plus both `/usr` halves |
| `overlay/usr/lib/repart.sysinstall.d/`, `systemd-sysinstall` | `installer.nix`, `nixos/installer/` | see below |
| `overlay/usr/lib/systemd/system-preset/10-losos.preset` | the module options themselves | on NixOS a unit is enabled by being wanted, not by a preset |
| `overlay/usr/lib/sysusers.d/losos.conf` | `systemd.sysusers.enable` over `users.users` | the upstream tool, not NixOS's perl script |
| `overlay/usr/lib/tmpfiles.d/losos.conf` | `systemd.tmpfiles.settings` in `services.nix` | |
| `overlay/usr/lib/systemd/network/20-wired.network` | `systemd.network.networks."20-wired"` | same match and settings |
| `overlay/etc/crypttab`, `overlay/etc/fstab` | `environment.etc.crypttab`, `swapDevices` | |
| `user@.service.d/10-oomd.conf` | `systemd.oomd`, `systemd.slices.user` | |
| `losos-security.service`, its D-Bus files | `services.nix` | same sandbox, line for line |
| `50-losos-factory-reset.rules` | `security.polkit.extraConfig` in `disk.nix` | same rule |
| `losos-selftest.service` | `testing.nix` | same kernel command line condition; `losos-ota-test.service` is gone, see `testing.nix` |
| `recipes/10-core/losos-release` | `system.nixos.distroId`, `system.image.*` | NixOS writes os-release itself, with `IMAGE_VERSION` |
| `manifest/architectures.yaml` | `losos.arch` in `options.nix` | x86_64 and aarch64 |
| `tools/configure --version --channel` | `losos.version`, `losos.channel` | the flake derives the version from the commit date |
| `tools/vm-test` | `nixos/tests/boot.nix` | boots the real image under UEFI |
| `tools/` gates, `Containerfile`, `./do` | `nix flake check`, `nix fmt` | the gates checked generated recipes against pm's fingerprint table, and neither exists now |
| pm, the system manager | pm, the system manager (`pm.nix`) | pinned to a commit |

## Everything systemd, and the exceptions

Every row of the table in `README.md` has a counterpart here. The places a
non-systemd component remains are the places systemd has no equivalent, or
NixOS requires one:

- **nsncd.** The `homed` module asserts `services.nscd.enable`: NixOS routes
  NSS through a forwarder so a libc can find `nss-systemd` in the store. It is
  stateless and holds no idea of its own about who exists. glibc's NSS uses it
  to find `nss-systemd`.
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

The pm tree's installer was `systemd-sysinstall`, new in systemd v261.
nixpkgs 26.05 shipped 260.4, so the installer does the same job by hand with
the same tools. nixos-unstable now carries 261, so sysinstall is available;
the installer has not been moved onto it yet. It is its own small live system, `nixos/installer/`, on its own ISO,
and `nixos/modules/installer.nix` evaluates it from the OS's configuration.

It boots by UEFI only: the ISO's appended FAT partition holds the installer's
UKI as `EFI/BOOT/BOOT<ARCH>.EFI`, with no bootloader in front of it, and the
initrd mounts the ISO by its volume label and the Nix store from a squashfs
on it. It carries `wpa_supplicant` and no NetworkManager. networkd runs DHCP
on every physical wired port, built in or USB, as soon as a cable is in, at
boot or later, and prefers it over Wi-Fi when both are up; a machine with a
cable in is online with nothing asked. Otherwise `losos-installer`
(`src/losos-installer`) talks to `wpa_supplicant` over its control socket to
scan for and join a Wi-Fi network, which networkd then runs DHCP on too. Of the firmware NixOS would add, only
`linux-firmware` is on the medium, because most Wi-Fi cards do not start
without it.

`losos-installer` is a terminal interface on tty1, with a root shell on tty2
for anything it does not cover. Its Network screen shows each wired port as
having no cable, getting an address, or online, and it moves on by itself as
soon as the machine is online, by cable or by Wi-Fi. Then it lists the
disks, leaving out the one the ISO booted from, and asks for `erase` to be
typed before it touches the one chosen. Then:

1. `systemd-repart --empty=force` lays out the ESP, with systemd-boot and
   `loader.conf` copied in, and slot A at full size, labelled `_empty`. These
   are `disk.nix`'s own definitions, so the disk is laid out the way the
   installed system expects to find it.
2. `systemd-sysupdate update` fills them with the channel's newest release,
   from the same URL and through the same transfers as `update.nix`, aimed at
   the chosen disk instead of `auto` and at the new ESP, mounted under
   `/run/losos-installer`. With `losos.update.pubring` set, the installer
   checks `SHA256SUMS.gpg` against it as an update does.
3. The machine reboots into the installed system, whose first boot creates
   slot B, root, `/home` and swap from `disk.nix`, the path an image written
   with `dd` takes.

So an install is an update into an empty slot: what lands on the disk is
what the channel serves at the time, not a copy of the OS on the medium that
could have gone stale. The image used to be its own installer, with a second
UKI that booted it into a tmpfs root and copied its own `/usr` with
`CopyBlocks=`; that is gone, and so is the `losos.install` condition it
needed in `disk.nix`. When nixpkgs reaches v261, the installer should be
measured against `systemd-sysinstall` again.

## Releases and GHCR

`nix build .#release` writes a flat directory:

```
losos-desktop_<v>_x86_64.efi                        the UKI sysupdate installs
losos-desktop_<v>_usr-x86-64_<uuid>.raw.xz          /usr for a sysupdate slot
losos-desktop_<v>_usr-x86-64-verity_<uuid>.raw.xz   its hash tree
losos-desktop_<v>_x86_64.raw.xz                     the disk image
losos-desktop_<v>_x86_64-installer.iso              the installer
losos-desktop_<v>_x86_64.qcow2                      the disk image, for a VM
SHA256SUMS
```

CI pushes it to GHCR as one OCI artifact per architecture with `oras`, and
`proxy/` serves the newest one to sysupdate, which cannot fetch from an OCI
registry itself (see "Binary cache"). The publish job then replaces the
`nightly` GitHub release with the installer ISOs and their lines of
`SHA256SUMS`, and nothing else: GitHub rejects release assets of 2 GiB or
more, which `/usr` and the disk image are not far from, and the ISO is what a
person downloads by hand.

The `/usr` halves are cut out of the finished disk image at the offsets repart
reported, not built a second time, so the bytes sysupdate installs are the
bytes the image boots. Each name carries its partition's UUID, which sysupdate
reads with `@u` and gives the partition it writes. The initrd finds `/usr` by
the UUIDs repart derived from `usrhash=`, so a slot that kept the random UUID
repart created it with would hold the right bytes and never be found.

## Halium

`nixos/halium/` is a second target: the same OS on a phone or tablet through
[Halium](https://halium.org), the Android hardware layer that ports such as
Ubuntu Touch and Droidian build on. `nixosModules.halium` imports
`nixos/modules/base.nix`, which holds everything a PC and a phone share (the
derisk desktop, homed accounts, networkd, pm, no Nix on the device, the `/etc`
overlay, sysusers), and adds its own boot and Android layer in place of the
PC's UEFI, verity `/usr`, sysupdate and installer.

How a device boots it:

1. The Android bootloader loads `boot.img` from the boot partition. It holds
   the device's kernel and NixOS's systemd initrd, with `init=` naming the
   system as the UKI does on a PC. Halium's own rootfs boots through
   halium-boot, a busybox ramdisk; this keeps systemd in the initrd instead.
2. The initrd mounts `userdata` at `/run/halium/userdata`, grows
   `losos/rootfs.img` on it to `losos.halium.rootfsSize`, and loop-mounts it
   as `/` with `x-systemd.growfs`.
3. After switch-root, the Halium system image (system-as-root, `/init` at its
   top) is mounted read-only at `/android/system`, `vendor` at `/vendor`, and
   `/android/system/system` at `/system`, where libhybris's linker looks.
4. `losos-android.service` starts Android's init with `lxc-start`. The
   container shares the host's `/dev` and network and gets userdata as
   `/data`, as in Halium's lxc-android. It is LXC rather than
   `systemd-nspawn` because nspawn always gives a container a private `/dev`,
   and the HAL device nodes ueventd creates must be the ones the host opens.
   It starts before the display manager.
5. libhybris is a GLVND EGL vendor in `hardware.graphics`, beside Mesa.

Every mount on the Android side is `nofail`, so a device whose Android half is
missing or broken still reaches the login screen.

The container is not a security boundary. Android's init and the vendor HALs
run as host root with the host's whole `/dev`, every block device included,
so the vendor partition's closed blobs are trusted as much as the kernel, and
nothing here plays the part of the PC build's verity `/usr`. It drops
`sys_module`, `sys_rawio`, `sys_time` and the MAC capabilities; a `/dev`
holding only the HAL nodes would narrow it further, but which nodes those are
is per device, and it waits for a device bring-up to find out.

`nix build .#packages.aarch64-linux.halium` produces `boot.img` and `rootfs.img.xz` with
`SHA256SUMS`. Install with `fastboot flash boot boot.img`, unpack
`rootfs.img.xz`, and copy it in from a recovery with
`adb push rootfs.img /data/losos/rootfs.img`.

### What a device port supplies

`losos-desktop-halium-aarch64` has no device in it: it carries nixpkgs'
generic kernel, warns that it boots nothing, and exists so CI evaluates and
builds the target. A port is that configuration plus:

- `losos.halium.kernelPackages`: the port's kernel, for example from
  `pkgs.linuxManualConfig` over the vendor tree and its Halium defconfig. It
  must be 5.10 or newer, systemd's minimum baseline, and evaluation fails
  otherwise. That rules out most Android 9 and 10 era ports, which run 4.x
  kernels.
- `losos.halium.mkbootimgArgs` and `losos.halium.dtb`: the device's
  `BOARD_MKBOOTIMG_ARGS` and device tree, copied from its `BoardConfig.mk`.
- `losos.halium.android.system` and `.vendor` when the partition labels
  differ (A/B slots), and `losos.halium.userdataFsType` for f2fs.
- `losos.halium.android.udevRules`: rules generated from the device's
  `ueventd.rc`.
- The Halium system image itself, built from the Halium tree for the device
  and flashed to `system`.

`nixos/pkgs/` builds `libhybris` from its upstream master and Halium's
`android-headers` (one tree serves Halium 11 to 16). A device whose vendor
HALs need older headers overrides `android-headers`.

## GTK and Qt on a phone

GTK and Qt applications are written for a desktop: dialogs open at a
desktop size, the GTK3 file chooser puts a sidebar beside its file list, and
a finger on a Qt list selects instead of scrolling. On a phone that means
windows that run off the screen. The overlay (`nixos/pkgs/default.nix`)
replaces `gtk3`, `gtk4` and Qt 6's `qtbase` with patched builds, so every
package that links them gets the patched toolkit, on the PC and on Halium.

Every patch decides for itself whether it is on a phone: it is when every
screen is under 600 logical pixels on its short side, the line derisk's
shell draws between its phone and tablet layouts (`src/adaptive.rs` in
derisk). A phone docked to a monitor is then a desktop again, and a PC
behaves as stock. The one exception is Qt's touch scrolling, which follows a
touchscreen rather than a screen size.

- **GTK3** carries Purism's adaptive series, the one PureOS and Mobian ship
  on phones: an adaptive file chooser (which xdg-desktop-portal-gtk shows for
  every application that asks the portal for a file), about, print, font and
  shortcuts windows that fit a phone, maximized dialogs, a back button in
  dialog header bars, message dialogs with stacked buttons, and touch event
  fixes. It copies a few libhandy widgets into GTK to build those. Purism
  turns it on with an `org.gtk.Settings.Purism` `is-phone` key per device;
  `0033` adds the screen-size check, and the key still forces it on.
- **GTK4** carries Purism's three: resizable dialogs and transient windows
  open maximized and get only a close button. postmarketOS carried the same
  two until libadwaita 1.5, whose `AdwDialog` and breakpoints adapt on their
  own, so libadwaita needs nothing; the patches are for plain GTK4 windows,
  such as gcr's prompts. `0004` is the screen-size check.
- **Qt 6** has no phone patches anywhere to adopt: Plasma Mobile adapts in
  Kirigami and its own shell. Both are this repository's. A finger scrolls
  any Qt Widgets scroll area kinetically through `QScroller`, which
  `QAbstractScrollArea` already supported but left to each application to
  turn on; `QT_TOUCH_SCROLLER=0` turns it off for an application that draws
  with a finger. On a phone, resizable dialogs open maximized, and the
  minimum size a window asks the compositor for is capped at the screen.
  Since Qt 6.10 the Wayland client is part of qtbase, so `qtwayland` is left
  stock. Qt 5, which nothing in the image links, gets no phone patches, only
  the icon theme one (below, "One icon theme").

The on-screen keyboard is derisk's. GTK3, GTK4 and Qt 6 all speak Wayland's
`text-input-unstable-v3` without patches, so the keyboard can follow text
focus once derisk's compositor offers that protocol.

What it costs: a patched toolkit no longer substitutes from
cache.nixos.org, and neither does anything built against it. In this image
that is GTK3 and GTK4, Qt 6's qtbase, qtdeclarative, qtsvg, qttools and
qtshadertools (qtbase links GTK3 for its GTK platform theme, and
breeze-icons, which papirus-icon-theme builds against, needs Qt), and
gjs, gcr, gnome-keyring, gnome-desktop, gnome-settings-daemon, libsecret,
xdg-desktop-portal and its GTK backend, ostree, flatpak and geoclue: about
thirty derivations per architecture. CI builds them once, pushes them to the
project's cache, and later runs substitute them from there until nixpkgs
moves GTK or Qt. Flatpak applications run on their runtime's own GTK and Qt
and do not get these patches.

When nixpkgs moves GTK or Qt and a patch stops applying, the build fails at
that patch. Purism rebases the GTK series for each Debian release;
refreshing means copying the series from `pureos/latest` again and
rebasing the two `Treat a display of phone-sized monitors` patches.

## One icon theme

Every app draws its icons from the icon theme derisk's theme names
(Papirus-Dark or Papirus by default; Settings, Appearance), not from one it
ships or names in code. Two halves make that hold.

derisk hands the name to every toolkit, through the channel each already
reads (`src/theme.rs` in derisk):

- **GTK 3 and 4** read `gtk-icon-theme-name` from `settings.ini` on the XDG
  config path, which derisk publishes, but prefer GSettings'
  `org.gnome.desktop.interface icon-theme` whenever the GNOME schemas are
  installed, as they are for any app nixpkgs wraps. So derisk also writes
  that key with `dconf`, which `programs.dconf` provides
  (`nixos/modules/desktop.nix`); without it GSettings answers its default,
  Adwaita, and GTK apps ignored derisk's theme. Running apps follow a change.
- **Qt 5 and 6** ask their platform theme, which in a session that is
  neither GNOME nor Plasma names no icon theme, so Qt apps had only
  hicolor's few icons. Both read `QT_QPA_SYSTEM_ICON_THEME` first, and derisk
  sets it in every app it launches and in the user manager's and D-Bus
  activation's environment. A running Qt app keeps the theme it started
  with.
- **Flatpak apps** see the host's icon themes at `/run/host/share/icons`
  (nixpkgs' flatpak binds `/run/current-system/sw/share/icons` there), and
  their GTK asks the settings portal for the name, which the GTK backend
  answers from the same GSettings key.

The toolkits then refuse an app's attempt to replace it, in the overlay's
patches (`nixos/pkgs/patches`, `gtk3/0034`, `gtk4/0005`, `qtbase/0003`,
`qtbase5/0001`):

- GTK ignores `gtk-icon-theme-name` set by the application on
  `GtkSettings`, or by a GTK theme's own `settings.ini`. Only the desktop's
  settings and the user's `~/.config/gtk-*/settings.ini` choose it.
  `gtk_icon_theme_set_custom_theme()` (GTK 3) and
  `gtk_icon_theme_set_theme_name()` (GTK 4) already refused the display's
  icon theme.
- Qt ignores `QIcon::setThemeName()` while the system names a theme. The
  name the app asked for becomes its fallback theme instead, unless it set
  one, so an icon only its own theme has still shows. This is how KDE apps
  ask for Breeze on every desktop but Plasma.
- In both, the system's icon directories always lead the search path,
  whatever the app sets or prepends, so an app cannot shadow the system
  theme with a directory of the same name or drop the directories it lives
  in. App directories still follow, and icons in an app's own resources stay
  hicolor-level fallbacks, as the icon theme specification has them.

What is left: an app that loads an image file or resource by path instead of
asking for an icon by name gets that image, since there is no name to look
up. Chromium and Electron draw their own UI icons that way and use GTK only
for file icons and the file chooser, which follow the theme. Flatpak apps
run on their runtime's unpatched GTK and Qt, so one can still name its own
theme in code; KDE runtime apps, which carry Breeze and never read
`QT_QPA_SYSTEM_ICON_THEME` from the host, keep Breeze.

## What is not done

- **pm on a NixOS host.** pm's jail mirrors the host's `/bin`, `/lib` and
  `/usr`, and additionally mounts `/nix/store` and the store-backed `PATH`
  directories read-only, so a step whose tools come from the store runs: a
  `losos-nix` step (`nix eval`, `nix hash`) ran that way in pm's jail on a
  Nix-provisioned host. What still finds nothing is a recipe that names FHS
  paths, `/bin/cat` or a compiler under `/usr`, as every recipe in the old pm
  tree did; in this image `/usr` is the Nix store's partition and `/bin` holds
  only `sh`. So pm itself runs here (`pm --help`, `pm source-path`, signing,
  `explain`), and the boot test checks it is installed, but a recipe written
  for an FHS host does not build on it. For the same reason seven of pm's test targets
  are skipped in the nix build (the list is in `nixos/pkgs/pm.nix`).
- **A complete image build.** CI builds release images within its runner time
  budget; local image builds may take longer.
- **No stable channel.** CI publishes `nightly` from `main` and nothing else.
  A `stable` release needs a second release output built with
  `losos.channel = "stable"`, and the flake has only the one.

- **Signing.** Until `nixos/keys/update-signing.asc` is committed,
  `losos.update.pubring` is unset, sysupdate installs updates without
  verifying `SHA256SUMS.gpg`, and the build warns. The `SHA256SUMS` attached
  to the GitHub release for the installer ISOs is not signed, and there is no
  key rotation: a new key needs an update signed by the old one that carries
  both. Secure Boot signing
  of the UKI is not done either; as in the pm tree, a key belongs to whoever
  owns the machine.
- **The two gnome-control-center patches** in `nixos/pkgs/patches/`. Nothing
  upstream has them, so they stay. They are written against
  gnome-control-center 51.0, and nixpkgs carries 50.4, which moved the
  System panel to Blueprint; neither applies. The factory reset is reachable
  through `systemctl start factory-reset.target` and the Varlink API, and the
  security report answers on the bus, but neither has a button in Settings.
- **derisk drives one display.** `derisk display-manager` starts
  `derisk greeter`, derisk's lock screen as the login screen, and after a
  login `derisk session --execute`; each takes the seat from logind and
  scans out through DRM/KMS on the first connected display, at its preferred
  mode (`nixos/modules/desktop.nix`). A second display stays dark and
  hotplug is not handled. Its lock screen (PAM service `derisk`) locks when asked and on logind's Lock,
  but nothing locks before sleep or on idle yet: that needs derisk to hold a
  logind `sleep` delay inhibitor until its lock screen is up. derisk also has no polkit agent, no layer-shell or
  XWayland yet, and no UI for
  Bluetooth or power profiles, so those are left off. `run0` from a terminal
  still asks polkit on the terminal. The gnome-control-center patches below
  have no Settings app to go into any more.
- **Flatpak installs need wheel.** flatpak's polkit rule lets an active,
  local wheel member install for the whole system without a password, which
  is how Bazaar installs. With no polkit agent in derisk, anyone else cannot
  be asked, and installs only for themselves. derisk's portal backend has no
  area or color picker and does not set the lock screen's picture; it asks
  for consent through GTK's access dialog, not one of its own.
- **Halium has not run on a device.** It evaluates, `libhybris` builds on
  x86_64, and an x86_64 build of the target (nixpkgs' kernel, no Android
  partitions) booted under QEMU from a disk whose only partition was
  `userdata`: the initrd mounted it, grew and loop-mounted `rootfs.img`,
  switched root, timed out on the missing `vendor` and `system` without
  failing, skipped the Android container and reached a login prompt. No
  arm64 build, `boot.img` on a device, or Android container has been run.
  Beyond that:
  - **The display.** derisk drives the display itself through DRM/KMS. A
    device whose kernel has a DRM driver (msm, mediatek, panfrost) can show
    the desktop with Mesa; one that only has Android's hwcomposer cannot,
    because nothing here drives hwcomposer. That needs a hwcomposer backend
    in derisk or a compositor in front of it.
  - **Dynamic partitions.** Devices from Android 10 on keep `system` and
    `vendor` inside `super`. Nothing maps its logical partitions yet, so such
    a device has to name device-mapper targets set up by its port.
  - **Updates.** sysupdate is not wired up; an update is a new `rootfs.img`
    and `boot.img`, flashed by hand. There is no verity on the root either.
  - **hardened_malloc.** The PC build preloads it into every process; Halium
    does not. Its default configuration reserves 32 GiB per size class per
    arena and needs a 48-bit address space, and Android kernels are usually
    built with 39-bit virtual addresses. A Halium build needs a
    hardened_malloc with a smaller `CONFIG_CLASS_REGION_SIZE`, and derisk
    built against it.
  - **Telephony, audio, sensors, camera.** No ofono, no PulseAudio/PipeWire
    droid modules, no sensorfw. Android's init starts the HALs, and nothing
    on the Linux side talks to them yet beyond EGL.
- **Wi-Fi.** As in the pm tree: networkd handles wired links, and nothing in
  the session configures Wi-Fi. `iwd` would be the smallest
  non-systemd addition that fixes it.
- **systemd-boot updates.** `systemd-boot-update.service` copies a new
  bootloader from `/usr/lib/systemd/boot`, which NixOS does not have. The
  bootloader on the ESP is the one the image shipped.
- **sysext.** Extensions merge into `/usr`, which here holds little but the
  Nix store, so an extension can add a program and cannot replace one.
- **Android apps.** `nixos/modules/atl.nix` has two options, both off by
  default. `losos.android.enable` installs the Android Translation Layer.
  `losos.android.openApks` additionally makes it the default handler for
  `.apk` files. The package (`nixos/pkgs/android-translation-layer.nix`) is
  pinned to a full commit with a real fixed-output hash, and it evaluates, but
  it has not built to completion. Two gaps remain, and the package header
  lists the first: ATL's build shells out to the Android SDK build-tools
  (`dx`, `aapt`), which the recipe does not yet provide, and the pinned tree
  has no `thirdparty/art_standalone/build`, which ART's Makefile includes, so
  it needs another source input. The package is marked `meta.broken`, so
  enabling `losos.android` fails at evaluation with nixpkgs' broken-package
  error until both are fixed. Because both options are off, `nix flake
  check` never realises the package, so the bring-up can land and mature
  without gating the image.
  **APKs run unsandboxed.** ATL runs an app's dex and native code as an
  ordinary process of the session user: Android's per-app UID, permission
  model and SELinux domain are not there, so a malicious APK has the reach of
  any native program the user runs, including their home, their Wayland
  session, the agent socket and the network. For that reason the default
  `.apk` handler is a second, separate opt-in, `losos.android.openApks`, off
  by default, so "open a downloaded file" never silently means "run untrusted
  code"; with it off an APK is run only by someone who chose to launch ATL on
  it. The real fix is a sandboxed launcher (bubblewrap with the usual
  namespaces and a restricted filesystem view, or a confined transient
  systemd user unit), which ATL's own README lists as future work. That is
  not done; until it is, turn `openApks` on knowingly.
- **Nothing has booted yet.** The disk image builds, with the layout above
  and a `usrhash=` equal to the root hash repart reported. The VM test
  evaluates and needs KVM, which the machine this was written on did not
  have; so the first-boot repart run, the gpt-auto root, and the installer
  have not been seen working. The installer ISO and its live system evaluate
  for both architectures and `losos-installer`'s tests pass, but the ISO has
  not been built, nor sysupdate seen writing to a disk that is not the one
  running. `nix build .#release` was not completed there either, for
  lack of disk space rather than an error.
