# The handful of settings that differ between one build of this OS and another.
#
# The pm tree carried these as `tools/configure` flags baked into generated
# files, and CLAUDE.md warns that nothing in the generated tree said which
# values it had been given. Here they are module options, so
# `nixos-option losos` or `nix eval .#nixosConfigurations.<name>.config.losos`
# answers the question directly.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  hostPlatform = pkgs.stdenv.hostPlatform;
in
{
  options.losos = {
    version = mkOption {
      type = types.strMatching "[0-9][0-9A-Za-z.~^-]*";
      example = "20260930.162538";
      description = ''
        The release version. It is baked into the image in three places that
        systemd-sysupdate reads back: the UKI's file name on the ESP, the
        partition label of the /usr and /usr-verity partitions, and
        IMAGE_VERSION in os-release. sysupdate orders versions with
        strverscmp(3), so a later build must compare greater.

        The flake sets it from the commit date, which is monotonic along a
        branch. It is not left at a constant default because the failure a
        constant produces is silent: every update would carry the same version
        as the running system, and sysupdate would never install one.
      '';
    };

    channel = mkOption {
      type = types.str;
      default = "nightly";
      description = ''
        The release channel. It is the GitHub release tag CI moves rather than
        creates anew, so the update URL stays the same across releases.
      '';
    };

    update = {
      baseUrl = mkOption {
        type = types.str;
        default = "https://github.com/dasmatus/losos-desktop/releases/download/${config.losos.channel}/";
        defaultText = lib.literalExpression ''"https://github.com/dasmatus/losos-desktop/releases/download/''${config.losos.channel}/"'';
        description = ''
          Where systemd-sysupdate looks for SHA256SUMS and the artifacts it
          lists. Must end in a slash: sysupdate appends file names to it.
        '';
      };

      pubring = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = ''
          The GPG public keyring SHA256SUMS.gpg is verified against. With no
          keyring, verification is turned off and the build says so with a
          warning; sysupdate's default is to refuse an unsigned manifest, and
          an image that shipped with that default and no key would never
          update at all.
        '';
      };
    };

    usrSize = mkOption {
      type = types.str;
      default = "8G";
      description = ''
        The size of each /usr slot once first boot has grown it. An update
        writes a whole new /usr into the slot not currently booted, so this
        is the ceiling on how large the Nix store of any future release may
        be; it cannot be raised later without repartitioning.
      '';
    };

    arch = mkOption {
      type = types.attrsOf types.str;
      readOnly = true;
      internal = true;
      description = ''
        Every per-architecture spelling the image needs. The pm tree kept these
        in manifest/architectures.yaml for the same reason: each is small, each
        is easy to leave at its x86 value, and each such mistake builds cleanly
        and fails on hardware.
      '';
    };
  };

  config.losos.arch =
    {
      x86_64 = {
        name = "x86_64";
        usr = "usr-x86-64";
        usrVerity = "usr-x86-64-verity";
        root = "root-x86-64";
      };
      aarch64 = {
        name = "aarch64";
        usr = "usr-arm64";
        usrVerity = "usr-arm64-verity";
        root = "root-arm64";
      };
    }
    .${hostPlatform.parsed.cpu.name}
      or (throw "losos: unsupported architecture ${hostPlatform.parsed.cpu.name}")
    // {
      # systemd-boot and the stub are named after the EFI machine type, which
      # nixpkgs already knows: x64, aa64.
      efi = hostPlatform.efiArch;
    };
}
