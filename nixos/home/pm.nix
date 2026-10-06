# pm's plugins through Home Manager, instead of copying them by hand.
#
# pm loads plugins only from the user's own <config>/pm/plugins, and only those
# signed by a key in <config>/pm/trusted (docs/pm.md, "pm's plugins"). That is
# per-user state, which is what Home Manager manages, so a user who runs it
# names the plugins they want here and their Home Manager generation installs
# them. Removing one from the list uninstalls it at the next switch, and
# rolling back a generation rolls back the plugin set with it, which a copy in
# ~/.config could not do.
#
# Trust stays the user's to give. Nothing here trusts a key on anyone's
# behalf: a plugin is either signed at activation with the user's own pm key,
# the same `pm sign` docs/pm.md has them run by hand, or carries a publisher's
# signature whose key the user lists in trustedKeys themselves.
#
# The image has no Nix and its users are systemd-homed records, so this is
# for a pm a user runs from their own Home Manager configuration, on LosOS's
# build hosts or any other machine; the image still ships the components
# unsigned under share/pm/plugins (nixos/modules/pm.nix).
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.pm;
  inherit (lib) types;

  system = pkgs.stdenv.hostPlatform.system;

  # Every plugin this generation installs, name -> component file.
  installed =
    lib.genAttrs cfg.plugins (name: "${cfg.pluginPackage}/share/pm/plugins/${name}.wasm")
    // cfg.extraPlugins;

  # The ones the user signs at activation: everything without a publisher's
  # signature.
  selfSigned = lib.filter (name: !(cfg.signatures ? ${name})) (lib.attrNames installed);

  pluginDir = "${config.xdg.configHome}/pm/plugins";

  # One script for the activation and the flake check that tests it, so what
  # the check proves is what a switch runs.
  sign = pkgs.writeShellApplication {
    name = "pm-sign-plugins";
    runtimeInputs = [
      cfg.package
      pkgs.coreutils
    ];
    text = ''
      dir=$1
      shift
      # A self-made signature is a regular file; one Home Manager links from
      # `signatures` is a symlink and is not ours to remove. Without its
      # plugin, a self-made one is left from a plugin taken off the list.
      for sig in "$dir"/*.wasm.sig; do
        [ -e "$sig" ] || continue
        if [ ! -L "$sig" ] && [ ! -e "''${sig%.sig}" ]; then
          rm -f -- "$sig"
        fi
      done
      # pm signs the file's bytes, which are the store's, and writes the
      # signature beside the link. Ed25519 is deterministic, so re-signing an
      # unchanged plugin writes the same file. The first `pm sign` on a
      # machine with no key makes one and trusts it, as it does by hand.
      for name in "$@"; do
        pm sign "$dir/$name.wasm" >/dev/null
      done
    '';
  };
in
{
  options.programs.pm = {
    enable = lib.mkEnableOption "pm, with its plugins installed by Home Manager";

    package = lib.mkOption {
      type = types.package;
      default = self.packages.${system}.pm;
      defaultText = lib.literalExpression "losos-desktop.packages.\${system}.pm";
      description = "pm itself, which also signs the plugins at activation.";
    };

    pluginPackage = lib.mkOption {
      type = types.package;
      default = self.packages.${system}.pm-plugins;
      defaultText = lib.literalExpression "losos-desktop.packages.\${system}.pm-plugins";
      description = "The plugin components `plugins` names, under share/pm/plugins.";
    };

    plugins = lib.mkOption {
      # Checked against the package's own list, so a typo fails evaluation
      # rather than linking a file that does not exist.
      type = types.listOf (types.enum (cfg.pluginPackage.plugins or [ ]));
      default = cfg.pluginPackage.plugins or [ ];
      defaultText = lib.literalMD "every plugin in `pluginPackage`";
      example = [
        "losos-nix"
        "systemd"
      ];
      description = ''
        Plugins from `pluginPackage` to install into pm's plugin directory.
        All seven by default: LosOS's four and pm's own `sysext`, `sysupdate`
        and `systemd`.
      '';
    };

    extraPlugins = lib.mkOption {
      type = types.attrsOf types.path;
      default = { };
      example = lib.literalExpression "{ zig = ./zig.wasm; }";
      description = ''
        Other plugin components, by the name pm will find them under. Signed
        at activation like the rest unless `signatures` has one for them.
      '';
    };

    signatures = lib.mkOption {
      type = types.attrsOf types.path;
      default = { };
      description = ''
        A publisher's `.sig` for a plugin, by plugin name. Such a plugin is not
        signed at activation, so the publisher's key has to be in
        `trustedKeys` or pm refuses to load it.
      '';
    };

    trustedKeys = lib.mkOption {
      type = types.listOf (types.strMatching "[0-9a-fA-F]{64}");
      default = [ ];
      description = ''
        Ed25519 public keys, in hex, added to pm's trust store. The store
        governs build files as well as plugins, so a key here is trusted for
        both. Keys added with `pm trust` stay beside these.
      '';
    };

    signScript = lib.mkOption {
      type = types.package;
      internal = true;
      readOnly = true;
      description = "The activation's signing step, for the flake check to run.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.all (name: installed ? ${name}) (lib.attrNames cfg.signatures);
        message = "programs.pm.signatures names plugins that are not installed: ${lib.concatStringsSep ", " (lib.attrNames (lib.removeAttrs cfg.signatures (lib.attrNames installed)))}";
      }
    ];

    home.packages = [ cfg.package ];

    # One link per file, so the directories stay real and pm can write
    # signatures and trusted keys into them beside what is linked.
    xdg.configFile =
      lib.mapAttrs' (name: file: lib.nameValuePair "pm/plugins/${name}.wasm" { source = file; }) installed
      // lib.mapAttrs' (
        name: file: lib.nameValuePair "pm/plugins/${name}.wasm.sig" { source = file; }
      ) cfg.signatures
      # pm's own file name and content for a key, as `pm trust` writes them.
      // lib.listToAttrs (
        map (
          key:
          lib.nameValuePair "pm/trusted/${lib.toLower key}.pub" {
            text = "${lib.toLower key}\n";
          }
        ) cfg.trustedKeys
      );

    home.activation = {
      # A signature the user made at an earlier activation is a regular file,
      # and Home Manager will not replace a file it did not link. When the same
      # plugin now comes with a publisher's signature, that file goes first.
      pmDropSelfSignatures = lib.hm.dag.entryBefore [ "checkLinkTargets" ] (
        lib.concatMapStrings (name: ''
          sig=${lib.escapeShellArg "${pluginDir}/${name}.wasm.sig"}
          if [ -f "$sig" ] && [ ! -L "$sig" ]; then
            run rm -f -- "$sig"
          fi
        '') (lib.attrNames cfg.signatures)
      );

      # After the links exist. XDG_CONFIG_HOME is passed because pm finds its
      # key and trust store through it, and Home Manager's xdg.configHome need
      # not be the default the activation's environment would give pm.
      pmSignPlugins = lib.hm.dag.entryAfter [ "linkGeneration" ] ''
        run env XDG_CONFIG_HOME=${lib.escapeShellArg config.xdg.configHome} \
          ${lib.getExe sign} ${lib.escapeShellArg pluginDir} ${lib.escapeShellArgs selfSigned}
      '';
    };

    programs.pm.signScript = sign;
  };
}
