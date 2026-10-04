# derisk, the desktop: an adaptive, agent-first Wayland shell on mcsapi's
# Smithay compositor, with its core apps (Files, Settings, Text Editor, System
# Monitor, Calculator) built into the same binary.
#
# Pinned to a commit and moved by hand, like pm, because both derisk and the
# mcsapi branch it builds against are moving fast. mcsapi comes in through
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
}:

rustPlatform.buildRustPackage {
  pname = "derisk";
  version = "0.1.0-unstable-2026-10-04";

  # fetchgit rather than fetchFromGitHub: a git clone reaches github.com from
  # hosts that cannot use its archive endpoint.
  src = fetchgit {
    url = "https://github.com/dasmatus/derisk";
    # dasmatus/derisk#11, the lock screen, until it merges.
    rev = "aad30d7f015c452ee8adbc5a0d8c80685181aad7";
    hash = "sha256-HGiQ4lrkQZXWjN8c/9apsQYMWUOQz8M+FwEK+E4EPAg=";
  };

  cargoHash = "sha256-Lz+3FxRVq6GObEwUuNZvC+MnILnPDZjugdk6L2gCTpE=";

  # `host` is the compositor; without it the binary has only the headless
  # commands.
  buildFeatures = [ "host" ];

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
  ];

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
  '';

  meta = {
    description = "Adaptive, agent-first Wayland desktop environment built on mcsapi";
    homepage = "https://github.com/dasmatus/derisk";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "derisk";
  };
}
