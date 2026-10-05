# Uranium's tabs in derisk's command palette: the native messaging host
# Uranium's tabs extension (uranium/tabs) talks to, which registers each
# window's tabs with derisk as its menus.
#
# Built from src/uranium-tabs, with its Cargo.lock's pins as fixed-output
# fetches, as losos-security is.
{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "uranium-tabs";
  version = "0.1.0";

  src = ../../src/uranium-tabs;

  cargoLock.lockFile = ../../src/uranium-tabs/Cargo.lock;

  # The window pairing and the menu ids, which nothing else would notice
  # breaking until a tab picked in the palette did nothing.
  doCheck = true;

  meta = {
    description = "Lists Uranium's tabs in derisk's command palette";
    license = lib.licenses.agpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "uranium-tabs";
  };
}
