# The installer's TUI: Wi-Fi through wpa_supplicant, a disk, then
# systemd-repart and systemd-sysupdate.
#
# Built from src/losos-installer, pinned by its Cargo.lock as losos-security is.
# The repart definitions and sysupdate transfers it runs with are not in here:
# they are generated from the OS's own configuration (installer.nix) and
# passed on its command line, so this package knows nothing about the layout.
{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "losos-installer";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ../../src/losos-installer;
    # A local `cargo build` leaves target/ beside the sources; it is gitignored,
    # but a path source would still copy all of it into the store.
    filter = path: _type: baseNameOf path != "target";
  };

  cargoLock.lockFile = ../../src/losos-installer/Cargo.lock;

  # Fixture trees and a fake wpa_supplicant socket: no disk, no radio, no
  # root. The disk filter is what stands between someone and the wrong drive,
  # so it is tested on every build.
  doCheck = true;

  meta = {
    description = "Text-mode installer for LosOS Desktop";
    license = lib.licenses.agpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "losos-installer";
  };
}
