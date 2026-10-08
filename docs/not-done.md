# What is not done

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
- **The Windows installer has not run on Windows.** Its install was checked
  into a disk image laid out like Windows', which then booted
  ([The Windows installer](windows-installer.md)); the half that talks to
  Windows (shrinking C:, the partition table IOCTLs, the ESP mount, firmware
  variables, BitLocker) has only been compiled. Beyond that:
  - **Secure Boot must be off**, for the reason above.
  - **SmartScreen.** The `.exe` has no Authenticode signature, so Windows
    warns before running it.
  - **Windows' recovery partition.** Updates that grow it (as KB5034441
    did) shrink C: to make room behind it; with LosOS there they cannot,
    and fail as they do on any disk without free space after C:.
  - **The boot order.** Some firmware, and some Windows updates, put
    Windows Boot Manager first again. LosOS is then one pick in the
    firmware's boot menu away, and nothing puts its entry back in front.
  - **arm64.** The installer is built for x86_64 only, because Windows on
    arm64 runs on laptops this OS has no kernel configuration for.
- **The two gnome-control-center patches** in `nixos/pkgs/patches/`. Nothing
  upstream has them, so they stay. They are written against
  gnome-control-center 51.0, and nixpkgs carries 50.4, which moved the
  System panel to Blueprint; neither applies. The factory reset is reachable
  through `systemctl start factory-reset.target` and the Varlink API, and the
  security report answers on the bus, but neither has a button in Settings.
- **NVIDIA on real hardware.** The driver choice was tested in a VM with a
  report that names an RTX 4090 ([Hardware and NVIDIA](drivers.md)); no
  NVIDIA card has run the image. Untested: the open module taking a card,
  derisk's compositor on it, explicit sync, suspend and hibernation,
  VA-API, hardened_malloc under NVIDIA's libraries, and laptops with two
  GPUs. A machine with an NVIDIA card and another GPU draws on whichever
  is the boot display; nothing offloads an app to the other.
- **derisk drives one display.** `derisk display-manager` starts
  `derisk greeter`, derisk's lock screen as the login screen, and after a
  login `derisk session --execute`; each takes the seat from logind and
  scans out through DRM/KMS on the first connected display, at its preferred
  mode (`nixos/modules/desktop.nix`). A second display stays dark and
  hotplug is not handled. derisk has no XWayland yet, and no UI for
  Bluetooth or power profiles, so those are left off. The gnome-control-center patches above
  have no Settings app to go into any more.
- **derisk's portal has gaps.** derisk's portal backend has no
  area or color picker and does not set the lock screen's picture; it asks
  for consent through GTK's access dialog, not one of its own. Screen
  sharing (xdg-desktop-portal-wlr) copies frames into shared memory, not
  dma-bufs, and a shared window is cut out of the screen, so anything
  covering it is shared too.
- **The Halium GSI has not run on a device.** It evaluates for aarch64, and
  an x86_64 build of the same boot path booted under QEMU (see the pull
  request that made it the one target). No arm64 build has been flashed, no
  bootloader has loaded its `init_boot`, and the web flasher has only talked
  to a simulated fastboot device. What only a phone can confirm: that a
  generic kernel's configuration has everything systemd and the desktop use
  (`CONFIG_USER_NS` for Flatpak's sandbox above all), that `vendor_boot`'s
  ramdisk and this one unpack together as intended, and that Halium's
  Android 14 system image starts each vendor's HALs. Beyond that:
  - **The display.** derisk drives the display itself through DRM/KMS. A
    device whose kernel has a DRM driver (msm, mediatek, panfrost) can show
    the desktop with Mesa; one that only has Android's hwcomposer cannot,
    because nothing here drives hwcomposer. That needs a hwcomposer backend
    in derisk or a compositor in front of it.
  - **Updates.** sysupdate is not wired up; the flasher writes userdata whole,
    so installing a newer release erases the phone. There is no verity on the
    root either, and the bootloader stays unlocked.
  - **Phones without `init_boot`.** The flasher writes their own boot image
    back with this ramdisk after theirs. Its repacking matches AOSP's
    `mkbootimg` byte for byte for header versions 0 to 4, and it was run
    against a simulated phone in fastbootd, but no real bootloader has
    booted such an image yet, and a phone whose ramdisk links `/etc` or
    `/lib` elsewhere would merge badly with this one.
  - **Kernels older than 5.10.** Phones that launched with Android 11 or
    earlier run 4.x or 5.4 kernels, which systemd does not support at all.
    The flasher reads the kernel's version and refuses them; supporting
    them would mean an older systemd or a kernel built per device.
  - **hardened_malloc.** The PC build preloads it into every process; the GSI
    does not. Its default configuration reserves 32 GiB per size class per
    arena and needs a 48-bit address space, and Android kernels are usually
    built with 39-bit virtual addresses. The GSI needs a hardened_malloc with
    a smaller `CONFIG_CLASS_REGION_SIZE`, and derisk built against it.
  - **Telephony, audio, sensors, camera.** No ofono, no PulseAudio/PipeWire
    droid modules, no sensorfw. Android's init starts the HALs, and nothing
    on the Linux side talks to them yet beyond EGL.
- **Wi-Fi after setup.** First-boot setup joins a network through
  `wpa_supplicant`, which saves it, but nothing in the session lists or
  joins networks afterwards: derisk's Settings has no Wi-Fi page yet.
- **systemd-boot updates.** `systemd-boot-update.service` copies a new
  bootloader from `/usr/lib/systemd/boot`, which NixOS does not have. The
  bootloader on the ESP is the one the image shipped. The same goes for GRUB
  on a BIOS PC: an update never rewrites the MBR or the BIOS boot partition.
- **Two LosOS disks in one BIOS PC.** A BIOS boot finds root by its
  partition label, `root-x86-64`, as no EFI variable names the disk
  ([Boot loaders](boot-loaders.md)). With two LosOS disks in one machine,
  the kernel may mount the other disk's root.
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
- **Nothing has booted yet.** The disk image builds, with the layout in [Where the OS lives](layout.md)
  and a `usrhash=` equal to the root hash repart reported. The VM test
  evaluates and needs KVM, which the machine this was written on did not
  have; so the first-boot repart run, the gpt-auto root, and the installer
  have not been seen working. The installer ISO and its live system evaluate
  for both architectures and `losos-installer`'s tests pass, but the ISO has
  not been built, nor sysupdate seen writing to a disk that is not the one
  running. `nix build .#release` was not completed there either, for
  lack of disk space rather than an error.
- **Danube is unfinished in places** ([Danube](danube.md)). A page's file
  upload button opens nothing, since WebKit's file chooser request is not
  answered yet, and a site's notifications are never shown, even when
  allowed. There are no extensions until the extension runtime lands, no
  bookmarks, history list or find in page, and no way to resume a
  download. Its sandboxes need unprivileged user namespaces, and a phone's
  own kernel may refuse them; Danube then stops rather than running a
  page outside one, so on such a phone it does not open at all.
  `--no-sandbox` skips only the outer one. It has run against WPE WebKit
  in a session, not yet on a desktop or phone with derisk.
- **Ad blocking is DNS-wide only for names.** A filter list's URL and
  element rules apply in Danube alone; other browsers and apps get only
  whole blocked domains ([System-wide ad blocking](adblock.md)), and a
  program with its own DNS over HTTPS skips it entirely.
