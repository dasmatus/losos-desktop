# Android Translation Layer: runs Android apps on the Linux desktop by
# translating Android's APIs onto GTK4, Vulkan and the host's libraries, rather
# than booting an Android userspace the way an emulator or Waydroid does.
#
# WORK IN PROGRESS — this recipe evaluates, but it has not yet built to
# completion and is not referenced by the default configuration (the
# losos.android module defaults off, so `nix flake check` never realises it).
# It captures the dependency set and build shape; the open items before it
# builds are called out below. Pinned to a commit and moved by hand, like pm
# and derisk, because the fork moves fast.
#
# Open items (why it is not wired into the image yet):
#   - src/cargoHash: fakeHash placeholders, to be filled by a real build.
#   - Android build-tools: the meson build shells out to `dx` and `aapt` to
#     dex and package the bundled framework. Those come from the Android SDK
#     build-tools, which nixpkgs exposes only through androidenv; this recipe
#     does not yet provide them, so the art_standalone/framework-res steps
#     will fail until it does.
#   - The bundled thirdparty (wolfSSL, libunwind, bionic_translation,
#     art_standalone) builds through a CMake orchestrator; whether every one
#     of those substitutes or has to build from the vendored copy needs a
#     build pass to confirm.
{
  lib,
  stdenv,
  fetchgit,
  cmake,
  meson,
  ninja,
  pkg-config,
  autoconf,
  automake,
  libtool,
  jdk21,
  ant,
  python3,
  gtk4,
  gtk4-layer-shell,
  vulkan-loader,
  vulkan-headers,
  openxr-loader,
  wayland,
  wayland-protocols,
  libportal,
  sqlite,
  ffmpeg,
  libdrm,
  libgudev,
  webkitgtk_6_0,
}:

stdenv.mkDerivation {
  pname = "android-translation-layer";
  version = "0.1.0-unstable-2026-10-04";

  # fetchgit rather than fetchFromGitHub, for the same reason derisk uses it:
  # a git clone reaches github.com from hosts that cannot use its archive
  # endpoint (see the project's proxy notes).
  src = fetchgit {
    url = "https://github.com/dasmatus/android_translation_layer";
    rev = "6af8f226"; # the branch head this packaging was written against
    hash = lib.fakeHash; # TODO: fill from a real fetch
    fetchSubmodules = true;
  };

  # Upstream assumes a Debian JDK path; use the JDK from nativeBuildInputs.
  postPatch = ''
    substituteInPlace CMakeLists.txt \
      --replace-fail '/usr/lib/jvm/java-21-openjdk-amd64' "$JAVA_HOME"
  '';

  nativeBuildInputs = [
    cmake
    meson
    ninja
    pkg-config
    autoconf
    automake
    libtool
    jdk21
    ant
    python3
  ];

  buildInputs = [
    gtk4
    gtk4-layer-shell
    vulkan-loader
    vulkan-headers
    openxr-loader
    wayland
    wayland-protocols
    libportal
    sqlite
    ffmpeg
    libdrm
    libgudev
    webkitgtk_6_0
  ];

  # The repo drives its multi-component build (wolfSSL, libunwind,
  # bionic_translation, art_standalone, then ATL itself) through a top-level
  # CMake orchestrator, so use it rather than invoking meson directly.
  dontUseCmakeConfigure = false;

  meta = {
    description = "Translation layer that runs Android apps on a Linux desktop";
    homepage = "https://github.com/dasmatus/android_translation_layer";
    # Upstream is on GitLab under GPL-3.0; the bundled thirdparty carries its
    # own licences (wolfSSL is GPL-2.0+).
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "android-translation-layer";
  };
}
