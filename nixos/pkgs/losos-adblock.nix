# The system-wide ad blocker (docs/adblock.md): fetches EasyList-style
# filter lists, answers DNS for the domains they block in front of
# systemd-resolved, and compiles the rest into WebKit content blockers for
# Danube.
#
# Built from src/losos-adblock; Cargo.lock pins every crate, as for
# losos-security.
{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "losos-adblock";
  version = "0.1.0";

  src = ../../src/losos-adblock;

  cargoLock.lockFile = ../../src/losos-adblock/Cargo.lock;

  # The tests parse lists and DNS packets and compile from a temporary
  # directory: no network, no root.
  doCheck = true;

  meta = {
    description = "System-wide ad and tracker blocking from filter lists";
    license = lib.licenses.agpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "losos-adblock";
  };
}
