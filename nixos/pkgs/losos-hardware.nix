# Chooses a machine's drivers at boot from its nixos-facter report
# (nixos/modules/hardware.nix, docs/drivers.md).
#
# Built from src/losos-hardware. Cargo.lock pins every crate with its SHA-256,
# and importCargoLock turns each pin into a fixed-output fetch.
{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "losos-hardware";
  version = "0.1.0";

  src = ../../src/losos-hardware;

  cargoLock.lockFile = ../../src/losos-hardware/Cargo.lock;

  # Plans for a hybrid laptop, an unsupported NVIDIA card and a VM, from
  # reports in facter's shape; no hardware or root needed.
  doCheck = true;

  meta = {
    description = "Choose a machine's drivers at boot from its nixos-facter report";
    license = lib.licenses.agpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "losos-hardware";
  };
}
