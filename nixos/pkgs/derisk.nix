# derisk, the desktop: an adaptive, agent-first Wayland shell on mcsapi's
# Smithay compositor, with its core apps (Files, Settings, Text Editor, System
# Monitor, Calculator) built into the same binary.
#
# Pinned to a commit and moved by hand, like pm, because both derisk and the
# mcsapi it builds against are moving fast. mcsapi comes in through
# Cargo.lock's git entries, which the cargo vendor step fetches and cargoHash
# covers.
{
  lib,
  rustPlatform,
  fetchgit,
  pkg-config,
  libxkbcommon,
  wayland,
  libGL,
  libinput,
  udev,
  libgbm,
  seatd,
  libdrm,
  linux-pam,
  graphene-hardened-malloc,
  # mcsapi-hardened-malloc as the global allocator (nixos/modules/allocator.nix
  # turns it on where the system preloads hardened_malloc).
  withHardenedMalloc ? false,
}:

rustPlatform.buildRustPackage {
  pname = "derisk";
  version = "0.1.0-unstable-2026-10-05";

  # fetchgit rather than fetchFromGitHub: a git clone reaches github.com from
  # hosts that cannot use its archive endpoint.
  src = fetchgit {
    url = "https://github.com/dasmatus/derisk";
    # derisk main: #22 (the xdg-desktop-portal backend), #23, #24, #21
    # (icons from the set icon theme, Papirus by default), #31 and #33
    # (the hardened-malloc feature, which allocator.nix turns on) and #32
    # (text fields in GTK and Qt apps open the on-screen keyboard, over
    # text-input-v3), and #34 (the browser and search engine choice screens,
    # which turn on from the policy file losos-ping writes).
    rev = "729ca0b6abdc8e17cba4b8070e414c74082e4c2f";
    hash = "sha256-TQcoG6NlE44I9CheQTToXiAQCz67umkhKvg1EKOf8ZI=";
  };

  cargoHash = "sha256-cPxWZOQV2dMTKMABxiBsdjYySO96iptzvTsdTHFQ6/w=";

  # derisk and its portal backend, xdg-desktop-portal-derisk. `host` is the
  # compositor; without it the binary has only the headless commands. It is
  # named through its package because cargo refuses a bare feature name when
  # more than one package is built.
  cargoBuildFlags = [
    "-p"
    "derisk"
    "-p"
    "derisk-portal"
  ];
  buildFeatures = [ "derisk/host" ] ++ lib.optional withHardenedMalloc "derisk/hardened-malloc";

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [
    libxkbcommon
    wayland
    libGL
    libinput
    udev
    libgbm
    seatd
    libdrm
    # The lock screen checks passwords through PAM.
    linux-pam
  ]
  # Linked, and put on the RUNPATH, by the cc wrapper.
  ++ lib.optional withHardenedMalloc graphene-hardened-malloc;

  # winit and glutin dlopen libwayland-client, libxkbcommon and libEGL at run
  # time rather than linking them, so nothing puts them on the RUNPATH and the
  # session would die looking for them.
  postFixup = ''
    patchelf --add-rpath ${
      lib.makeLibraryPath [
        wayland
        libxkbcommon
        libGL
      ]
    } $out/bin/derisk
  '';

  # The units call plain `derisk`; give them the store path so they work
  # without the user's PATH.
  postInstall = ''
    install -Dm644 -t $out/lib/systemd/user data/systemd/user/*
    substituteInPlace $out/lib/systemd/user/derisk-agent.service \
      --replace-fail "ExecStart=derisk " "ExecStart=$out/bin/derisk "

    # The portal backend. xdg.portal links share/xdg-desktop-portal from
    # extraPortals and configPackages, and D-Bus wants an absolute Exec=.
    install -Dm644 -t $out/share/xdg-desktop-portal/portals data/portal/derisk.portal
    install -Dm644 -t $out/share/xdg-desktop-portal data/portal/derisk-portals.conf
    install -Dm644 -t $out/share/dbus-1/services data/dbus-1/services/*
    substituteInPlace $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.derisk.service \
      --replace-fail "Exec=xdg-desktop-portal-derisk" "Exec=$out/bin/xdg-desktop-portal-derisk"
    substituteInPlace $out/lib/systemd/user/xdg-desktop-portal-derisk.service \
      --replace-fail "ExecStart=xdg-desktop-portal-derisk" "ExecStart=$out/bin/xdg-desktop-portal-derisk"
  '';

  meta = {
    description = "Adaptive, agent-first Wayland desktop environment built on mcsapi";
    homepage = "https://github.com/dasmatus/derisk";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "derisk";
  };
}
