# The operating-system half of the security report, on the system bus.
#
# Built from src/losos-security. Cargo.lock pins every crate with its SHA-256,
# and importCargoLock turns each pin into a fixed-output fetch, so nix enforces
# the pins that `cargo build --locked` would.
{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "losos-security";
  version = "0.1.0";

  src = ../../src/losos-security;

  cargoLock.lockFile = ../../src/losos-security/Cargo.lock;

  # The suite is ten tests over a fixture tree: no TPM, no root, no network.
  # It is what distinguishes "the checks compile" from "the checks say the right
  # thing", so it runs on every build.
  doCheck = true;

  meta = {
    description = "Operating-system security checks served on the system bus";
    license = lib.licenses.agpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "losos-security";
  };
}
