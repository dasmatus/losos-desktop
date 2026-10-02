# The RAM-sized swap definition for systemd-repart.
#
# repart can size a partition from a number and not from the machine it is
# running on, so this writes one 25-swap.conf into a directory repart also
# reads. The source is the pm tree's own, built here by the same meson file:
# one C file, no dependencies, and no reason to have two copies of it.
{
  lib,
  stdenv,
  meson,
  ninja,
}:

stdenv.mkDerivation {
  pname = "losos-swap";
  version = "0.1.0";

  src = ../../recipes/10-core/losos-swap/files;

  nativeBuildInputs = [
    meson
    ninja
  ];

  meta = {
    description = "Write a RAM-sized swap partition definition for systemd-repart";
    license = lib.licenses.agpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "losos-swap";
  };
}
