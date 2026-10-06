# The Home Manager module (nixos/home/pm.nix) built into a real Home Manager
# generation, and its activation's signing step run against that
# generation's files. Passing means pm, with no flags, loads every plugin the
# generation installed: each is linked where pm looks, signed by the key the
# first `pm sign` made, and that key is trusted.
{
  self,
  home-manager,
  pkgs,
}:

let
  # Ed25519's base point: a valid public key nobody signs with, to show a
  # listed key lands in the trust store beside the one pm makes.
  listedKey = "5866666666666666666666666666666666666666666666666666666666666666";

  generation = home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      self.homeModules.pm
      {
        home.username = "losos";
        home.homeDirectory = "/home/losos";
        home.stateVersion = "25.05";
        programs.pm.enable = true;
        programs.pm.trustedKeys = [ listedKey ];
      }
    ];
  };

  inherit (generation.config.programs) pm;
in
pkgs.runCommand "home-manager-pm"
  {
    nativeBuildInputs = [ pm.package ];
  }
  ''
    export HOME=$TMPDIR/home
    # The generation's files as linkGeneration leaves them: real directories,
    # each file a link into the store.
    mkdir -p "$HOME"
    cp -rP ${generation.config.home-files}/.config "$HOME/"
    chmod -R u+w "$HOME/.config"

    # A stale self-made signature, for a plugin no longer on the list.
    echo stale > "$HOME/.config/pm/plugins/gone.wasm.sig"

    ${pkgs.lib.getExe pm.signScript} "$HOME/.config/pm/plugins" ${pkgs.lib.escapeShellArgs pm.plugins}

    test ! -e "$HOME/.config/pm/plugins/gone.wasm.sig"
    test -f "$HOME/.config/pm/trusted/${listedKey}.pub"
    test -f "$HOME/.config/pm/signing.key"

    pm plugins | tee plugins.txt
    for name in ${pkgs.lib.escapeShellArgs pm.plugins}; do
      # Loaded as signed, not merely listed: pm refuses an unsigned or
      # untrusted plugin outright, but the line says which it was.
      grep -q "^$name [^ ]* (signed)$" plugins.txt
    done
    touch $out
  ''
