# The web browser: Uranium, Chromium under the OS's own name, which turns
# into Chrome for Android on a phone (nixos/pkgs/uranium.nix, docs/nixos.md,
# "Uranium"). It is what opens a link or an HTML file unless the user picks
# something else.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos.uranium;
  uranium = if cfg.patched then pkgs.uranium-patched else pkgs.uranium;
in
{
  options.losos.uranium.patched = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Ship the Uranium compiled from Chromium's source with
      `nixos/pkgs/patches/chromium`, which says Uranium inside the browser
      too and on a phone sends Chrome for Android's client hints and lays
      pages out with Android's viewport. Off, Uranium wraps nixpkgs' own
      Chromium from cache.nixos.org with the same name, icon, profile and
      Android User-Agent string. Off by default because CI's runners cannot
      compile Chromium inside their time budget: turn it on where the build
      host can, or once the patched build is in the project's cache.
    '';
  };

  config = {
    environment.systemPackages = [ uranium ];

    # The default for web pages and links, which xdg-open, the portal's
    # OpenURI and derisk's own launcher all read from mimeapps.list.
    xdg.mime.defaultApplications = lib.genAttrs [
      "text/html"
      "application/xhtml+xml"
      "x-scheme-handler/http"
      "x-scheme-handler/https"
    ] (_: "uranium.desktop");
  };
}
