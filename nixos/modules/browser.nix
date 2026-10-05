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
  flatpak = config.services.flatpak.package;
  desktopFile = if cfg.flatpak then "org.losos.Uranium.desktop" else "uranium.desktop";
in
{
  options.losos.uranium.flatpakRemote = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    example = "https://proxy.example/flatpak/";
    description = ''
      Where the Uranium Flatpak is served from: the proxy's /flatpak/, an
      OCI remote whose index and images CI's uranium workflow publishes.
      The flake sets it from LOSOS_PROXY_URL. The OS adds it as the
      Flatpak remote `losos`, so Bazaar lists Uranium beside Flathub's
      apps; null adds nothing.
    '';
  };

  options.losos.uranium.flatpak = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Install Uranium as the Flatpak from `flatpakRemote` at boot, and keep
      it updated there, in place of the build in the image. Off by default:
      the image's own works offline from the first boot and is the one the
      patched build replaces.
    '';
  };

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
    assertions = [
      {
        assertion = cfg.flatpak -> cfg.flatpakRemote != null;
        message = "losos.uranium.flatpak needs losos.uranium.flatpakRemote, which LOSOS_PROXY_URL sets.";
      }
    ];

    environment.systemPackages = lib.optional (!cfg.flatpak) uranium;

    # The proxy's remote, added once at boot as flathub is (flatpak.nix).
    # An OCI remote has no OSTree signature to check: each image is fetched
    # by the digest in an index served over HTTPS by the project's own
    # proxy, which is what is trusted.
    systemd.services.flatpak-losos = lib.mkIf (cfg.flatpakRemote != null) {
      description = "Add the losos Flatpak remote";
      wantedBy = [ "multi-user.target" ];
      after = [ "systemd-tmpfiles-setup.service" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${flatpak}/bin/flatpak remote-add --system --if-not-exists --no-gpg-verify --title=losos losos oci+${cfg.flatpakRemote}";
      };
    };

    # Uranium from that remote, installed or updated at each boot once the
    # network is up; a boot without network keeps the one it has.
    systemd.services.uranium-flatpak = lib.mkIf cfg.flatpak {
      description = "Install or update the Uranium Flatpak";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [
        "network-online.target"
        "flatpak-losos.service"
      ];
      requires = [ "flatpak-losos.service" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${flatpak}/bin/flatpak install --system --noninteractive --or-update losos org.losos.Uranium";
      };
    };

    # Its policies, first preferences and tabs host, which
    # nixos/pkgs/uranium/config.nix lists and explains. The Flatpak carries
    # its own copy under /app/chromium and cannot see the host's /etc.
    environment.etc = lib.mkIf (!cfg.flatpak) (
      lib.mapAttrs' (
        name: value: lib.nameValuePair "chromium/${name}" { text = builtins.toJSON value; }
      ) uranium.chromiumConfig
    );

    # The default for web pages and links, which xdg-open, the portal's
    # OpenURI and derisk's own launcher all read from mimeapps.list.
    xdg.mime.defaultApplications = lib.genAttrs [
      "text/html"
      "application/xhtml+xml"
      "x-scheme-handler/http"
      "x-scheme-handler/https"
    ] (_: desktopFile);
  };
}
