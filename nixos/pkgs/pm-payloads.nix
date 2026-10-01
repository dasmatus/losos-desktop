# Any nixpkgs package, as what a pm package installs.
#
# `pm-payloads.<name>` is nixpkgs' `<name>` from the pinned tree, linked
# statically from nixpkgs' pkgsStatic and laid out as a pm package's /dest:
# usr/bin, usr/sbin, usr/libexec and usr/share, every output merged. A pm
# build file reaches it through losos-nix (`nix build .#pm-payloads.<name>`)
# and copies the result into /dest (docs/nixos.md, "nixpkgs in a pm build").
#
# Static, because pm extracts a package at /pkg and runs it in a jail whose
# /nix/store is the host's own, or nothing at all: a dynamically linked program
# names its loader and every library by store path, and those paths exist only
# on a machine that happens to have built the same closure. pkgsStatic rather
# than relinking afterwards, because nixpkgs then builds every dependency static
# as well, with the flags each one's maintainers chose for that.
{
  lib,
  pkgsStatic,
  runCommand,
}:

let
  payload =
    name: package:
    runCommand "pm-payload-${package.pname or name}-${package.version or "0"}"
      {
        # The check that makes this safe to hand to pm rather than a hope: Nix
        # refuses the output if any file in it still names a store path, and
        # the error says which. A package that embeds its own store path (a
        # data directory compiled in, a script's shebang) cannot run from /pkg,
        # and finding that out here is better than finding it out at run time.
        allowedReferences = [ ];
        passthru = { inherit package; };
      }
      ''
        mkdir -p $out/usr
        for output in ${
          lib.concatMapStringsSep " " (o: "${package.${o}}") (package.outputs or [ "out" ])
        }; do
          for dir in bin sbin libexec share; do
            if [ -d "$output/$dir" ]; then
              # -L: a symlink into another store path becomes the file itself.
              cp -rL "$output/$dir" $out/usr/
              # Store files are read-only, and the next output merges into
              # the same directories.
              chmod -R u+w $out/usr
            fi
          done
        done
      '';
in
# Lazy in its values, so naming one package builds that one and evaluates
# nothing else.
lib.mapAttrs (
  name: package:
  lib.throwIfNot (lib.isDerivation package) "pm-payloads.${name} is not a package" (
    payload name package
  )
) pkgsStatic
