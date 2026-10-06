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
  still asks polkit on the terminal. The gnome-control-center patches above
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
- **Wi-Fi after setup.** First-boot setup joins a network through
  `wpa_supplicant`, which saves it, but nothing in the session lists or
  joins networks afterwards: derisk's Settings has no Wi-Fi page yet.
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
- **Nothing has booted yet.** The disk image builds, with the layout in [Where the OS lives](layout.md)
  and a `usrhash=` equal to the root hash repart reported. The VM test
  evaluates and needs KVM, which the machine this was written on did not
  have; so the first-boot repart run, the gpt-auto root, and the installer
  have not been seen working. The installer ISO and its live system evaluate
  for both architectures and `losos-installer`'s tests pass, but the ISO has
  not been built, nor sysupdate seen writing to a disk that is not the one
  running. `nix build .#release` was not completed there either, for
  lack of disk space rather than an error.
