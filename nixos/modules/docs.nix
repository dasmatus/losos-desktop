# This documentation, on the machine: the site built for reading offline
# (pkgs/losos-docs.nix) under /run/current-system/sw/share/doc/losos, and the
# two variables mcsapi's `Docs` reads, so an error dialog's "Learn more" opens
# its section there and on the published site only for a page the copy does
# not have (docs/troubleshooting.md).
#
# sessionVariables reach the user manager through PAM, and so every app derisk
# starts as a unit under it. First-boot setup is a system service with no PAM
# environment and no browser to open a page in, so it gets only the published
# site's address, which its dialogs print for reading on another device; the
# installer's live system does the same (installer/default.nix).
{
  config,
  lib,
  pkgs,
  ...
}:

let
  online = lib.optionalAttrs (config.losos.docs.url != null) {
    MCSAPI_DOCS_URL = config.losos.docs.url;
  };
in
{
  environment.systemPackages = [ pkgs.losos-docs ];
  environment.pathsToLink = [ "/share/doc/losos" ];
  environment.sessionVariables = {
    MCSAPI_DOCS_DIR = "/run/current-system/sw/share/doc/losos";
  }
  // online;
  systemd.services.derisk-setup.environment = online;
}
