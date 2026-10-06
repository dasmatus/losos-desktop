# The Android half of the GSI target (nixos/halium/): Halium's generic system
# image, an arm64 Android 14 system-as-root that starts the vendor's HALs and
# nothing a person sees. Treble makes one such image run over any device's
# vendor partition, which is what lets the OS be one image rather than a port
# per device.
#
# Built by UBports from Halium's AOSP tree (halium_arm64) and packaged by
# Droidian, whose repository is where it is published as a file; nixpkgs has
# no AOSP build. The .deb is pinned by the hash Droidian's signed Packages
# index gives for it, and only its one ext4 image is kept.
{
  lib,
  stdenvNoCC,
  fetchurl,
  dpkg,
}:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "halium-gsi";
  version = "14.0.0+r47.20260730.ubports.460";

  src = fetchurl {
    url = "https://production.repo.droidian.org/pool/main/a/android-system-gsi-34-bin/android-system-gsi-34_${finalAttrs.version}+git20260630121416.6478b73.next.production_arm64.deb";
    hash = "sha256-dBoaMc6As6lmpiFyScZ0/n34NoehxCERdgA+JdYFZBo=";
  };

  nativeBuildInputs = [ dpkg ];

  unpackPhase = ''
    dpkg-deb --fsys-tarfile $src | tar -x ./var/lib/lxc/android/android-rootfs.img
  '';

  installPhase = ''
    mkdir -p $out
    mv var/lib/lxc/android/android-rootfs.img $out/system.img
  '';

  # An ext4 image of bionic binaries, read through a loop device; nothing in
  # it is ELF this stdenv could patch or strip.
  dontFixup = true;

  meta = {
    description = "Halium's generic Android system image, for running vendor HALs beside a GNU/Linux system";
    homepage = "https://github.com/droidian/android-system-gsi-34-bin";
    license = lib.licenses.asl20;
    # arm64 code, but to the build only a file; an x86_64 build of the
    # target carries it too, to boot-test everything around it in QEMU.
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
})
