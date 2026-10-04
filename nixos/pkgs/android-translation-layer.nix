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
  makeWrapper,
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
  libGL,
  alsa-lib,
  libbsd,
}:

stdenv.mkDerivation {
  pname = "android-translation-layer";
  version = "0.1.0-unstable-2026-10-04";

  # fetchgit rather than fetchFromGitHub, for the same reason derisk uses it:
  # a git clone reaches github.com from hosts that cannot use its archive
  # endpoint (see the project's proxy notes).
  src = fetchgit {
    url = "https://github.com/dasmatus/android_translation_layer";
    # The ATL branch head carrying the Play-services shims and the theme
    # hook (dasmatus/android_translation_layer#1). The tree has no
    # .gitmodules: thirdparty/ is vendored, so there is nothing for
    # fetchSubmodules to fetch and the hash covers the whole tree as is.
    rev = "cd42b34789db99637472c724c0a41e626326614b";
    hash = "sha256-Nz5oGZ7jNeGXGti3vBbRJ0DJaByhhzre+bPVuDRBDa0=";
  };

  # Upstream assumes a Debian JDK path; use the JDK from nativeBuildInputs.
  # It also installs into a prefix inside the build directory, and meson bakes
  # that prefix into the binary as INSTALL_DATADIR, where it looks for
  # share/atl/system/etc/fonts.xml. Point it at $out so the lookup survives
  # the build directory being deleted.
  postPatch = ''
    substituteInPlace CMakeLists.txt \
      --replace-fail '/usr/lib/jvm/java-21-openjdk-amd64' "$JAVA_HOME" \
      --replace-fail 'set(INSTALL_PREFIX "''${BUILD_DIR}/install")' "set(INSTALL_PREFIX \"$out\")"
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
    makeWrapper
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
    # ATL's meson.build asks for gl and egl and links -lasound; the vendored
    # bionic_translation asks for libbsd. None of the inputs above propagate
    # them.
    libGL
    alsa-lib
    libbsd
  ];

  # The repo drives its multi-component build (wolfSSL, libunwind,
  # bionic_translation, art_standalone, then ATL itself) through a top-level
  # CMake orchestrator, so the stock configure and build phases drive it. It
  # defines no install() rule: installing is its own `install_all` target,
  # which runs `meson install` into the orchestrator's INSTALL_PREFIX,
  # ignoring CMAKE_INSTALL_PREFIX (postPatch makes that $out; the ART build
  # installs there during the build, so lib/art and the framework files under
  # lib/java/dex are already in place). The runtime that ATL dlopens at run
  # time stays under the build directory's lib/ and bionic_build/. So install
  # by hand: run the target, carry those trees under $out and give the binary
  # the LD_LIBRARY_PATH the repo's own launcher assembles, because nothing
  # puts those directories on its RUNPATH. $out/lib/art comes first: ATL finds
  # api-impl.jar and framework-res.apk relative to the libart.so it loaded,
  # and only that copy sits beside them.
  installPhase = ''
    runHook preInstall
    cmake --build . --target install_all
    mkdir -p $out/lib/atl-runtime
    cp -r lib/. $out/lib/atl-runtime/
    if [ -d bionic_build ]; then
      find bionic_build -maxdepth 1 -name '*.so*' -exec cp -t $out/lib/atl-runtime/ {} +
    fi
    wrapProgram $out/bin/android-translation-layer \
      --prefix LD_LIBRARY_PATH : "$out/lib/art:$out/lib:$out/lib/java/dex/android_translation_layer/natives:$out/lib/atl-runtime:$out/lib/atl-runtime/art"
    runHook postInstall
  '';

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
