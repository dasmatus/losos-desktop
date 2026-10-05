# x2mcsapi, mcsapi's adapter that styles apps not built on mcsapi so they
# match derisk: a GTK 3 and GTK 4 theme, a Qt style sheet and web CSS, all
# generated from the same mcsapi theme derisk draws itself with. Uranium's
# launcher runs it to style the browser's own interface (uranium.nix).
#
# The same mcsapi commit derisk's Cargo.lock pins, so both read a theme the
# same way; move the two together.
{
  lib,
  rustPlatform,
  fetchgit,
  libxkbcommon,
}:

rustPlatform.buildRustPackage {
  pname = "x2mcsapi";
  version = "0.1.0-unstable-2026-10-05";

  src = fetchgit {
    url = "https://github.com/dasmatus/mcsapi";
    rev = "b027f20964291b9bc0204502fc5a49ee3b361906";
    hash = "sha256-QfQuuhRwOuP1c5rqxHXyxCstsFPgnJe77AhIbKre/ZQ=";
  };

  cargoHash = "sha256-pt+mYQnrTDj9rjslTe7psyGyq8EqpPksoZPxL1X1fFY=";

  cargoBuildFlags = [
    "-p"
    "x2mcsapi"
  ];
  # The mcsapi crate it builds on links xkbcommon for keyboard handling.
  buildInputs = [ libxkbcommon ];

  cargoTestFlags = [
    "-p"
    "x2mcsapi"
  ];

  meta = {
    description = "Styles GTK, Qt and web apps to match an mcsapi desktop such as derisk";
    homepage = "https://github.com/dasmatus/mcsapi";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "x2mcsapi";
  };
}
