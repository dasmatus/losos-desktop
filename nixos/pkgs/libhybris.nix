# libhybris: loads Android's bionic-linked libraries (the vendor's EGL, GLES,
# gralloc and hwcomposer) into ordinary glibc processes, which is how a Halium
# device gets GPU acceleration from drivers that only exist as Android blobs.
#
# Built with GLVND, so its EGL is one vendor among several rather than a
# replacement libEGL.so: halium.nix lists it in hardware.graphics, and
# libglvnd picks it through share/glvnd/egl_vendor.d. Mesa stays installed
# beside it and keeps working wherever a device has a DRM driver of its own.
{
  lib,
  stdenv,
  fetchgit,
  autoreconfHook,
  pkg-config,
  python3,
  wayland,
  wayland-scanner,
  libglvnd,
  libGL,
  android-headers,
}:

let
  # libhybris names architectures its own way and has no autodetection.
  arch =
    {
      aarch64 = "arm64";
      arm = "arm";
      x86_64 = "x86-64";
      i686 = "x86";
    }
    .${stdenv.hostPlatform.parsed.cpu.name}
      or (throw "libhybris: unsupported architecture ${stdenv.hostPlatform.parsed.cpu.name}");
in
stdenv.mkDerivation (finalAttrs: {
  pname = "libhybris";
  version = "0.0.5-unstable-2026-06-24";

  # fetchgit rather than fetchFromGitHub: a git clone reaches github.com from
  # hosts that cannot use its archive endpoint.
  src = fetchgit {
    url = "https://github.com/libhybris/libhybris";
    rev = "7079712a42ea2754adf747e70c6cc75764c8596e";
    hash = "sha256-4SWFmjwDY1f4H0ZzuCeCLwK4+JqjBWrlo12/7rucwLA=";
  };

  sourceRoot = "${finalAttrs.src.name}/hybris";

  nativeBuildInputs = [
    autoreconfHook
    pkg-config
    python3
    wayland-scanner
  ];
  buildInputs = [
    wayland
    libglvnd
    libGL
    android-headers
  ];

  configureFlags = [
    "--enable-arch=${arch}"
    "--enable-wayland"
    "--enable-glvnd"
  ];
  # The default library search path is configure's per-architecture one,
  # /vendor/lib64:/system/lib64:/odm/lib64 on arm64, which is where
  # halium.nix mounts the vendor and Android system images.

  # Android's system/audio.h calls strdup(), which glibc declares only for
  # _GNU_SOURCE or POSIX, and the test programs that include it are built
  # without either. GCC 14 and later make the implicit declaration an error,
  # and on 64-bit the implicit int it returns would truncate the pointer.
  env.NIX_CFLAGS_COMPILE = "-D_GNU_SOURCE";

  enableParallelBuilding = true;

  meta = {
    description = "Run Android's bionic-linked drivers from glibc programs";
    homepage = "https://github.com/libhybris/libhybris";
    license = lib.licenses.asl20;
    platforms = [
      "aarch64-linux"
      "x86_64-linux"
    ];
  };
})
