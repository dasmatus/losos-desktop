# LosOS Desktop as NixOS.
#
# The same OS the pm recipes under recipes/ describe -- a GNOME desktop that is
# systemd end to end, image-based, updated by systemd-sysupdate and with every
# user a systemd-homed LUKS volume -- expressed as a NixOS configuration
# instead of a from-source distribution. docs/nixos.md says what maps to what
# and what changed on the way; nixos/ holds the modules.
#
#   nix build                      the disk image (.#image)
#   nix build .#installer          the same image, booting the installer
#   nix build .#release            what a GitHub release uploads, with SHA256SUMS
#   nix run .#vm                   boot the image in QEMU with UEFI firmware
#   nix flake check                evaluate both architectures, test losos-security,
#                                  and (with KVM) boot the image
{
  description = "LosOS Desktop: a systemd-native, image-based GNOME desktop on NixOS";

  inputs = {
    # The channel tarball rather than github:NixOS/nixpkgs. It is the same
    # tree at the revision the channel's Hydra jobset tested -- its sources,
    # not its binaries: the system below is musl and built from them -- and
    # it resolves to an immutable releases.nixos.org URL that flake.lock pins
    # by narHash exactly as it would a git revision. It is also the form that
    # fetches without api.github.com, which rate-limits anonymous clients and
    # which some build hosts cannot reach at all. `nix flake update nixpkgs`
    # moves it.
    #
    # 26.05 is the stable release: this is an OS people install, and an
    # update to it is a new image, so unstable's churn buys nothing.
    nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";
  };

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems = f: lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});

      # strverscmp(3) orders this the way time does, which is what
      # systemd-sysupdate needs from a version: a later commit compares greater.
      # A dirty tree reuses the last commit's date, so a local build never
      # claims to be newer than the release it was built from.
      version =
        let
          d = self.lastModifiedDate or "19700101000000";
        in
        "${lib.substring 0 8 d}.${lib.substring 8 6 d}";

      # musl, not glibc: the libc the pm tree builds everything against, and
      # the one this OS is meant to run on. Only the host platform changes, so
      # the build is native rather than cross, and runs on any builder of the
      # same architecture: on x86_64 nixpkgs grows its musl compiler from
      # source (the minimal bootstrap), on aarch64 from pinned musl bootstrap
      # tools. Every package is compiled by the builder; CI's only binary
      # cache is this project's own (docs/nixos.md, "Binary cache").
      musl = {
        x86_64-linux = lib.systems.examples.musl64;
        aarch64-linux = lib.systems.examples.aarch64-multiplatform-musl;
      };

      mkSystem =
        system:
        lib.nixosSystem {
          modules = [
            self.nixosModules.default
            {
              nixpkgs.hostPlatform = musl.${system};
              losos.version = lib.mkDefault version;
            }
          ];
        };

      archOf = system: lib.head (lib.splitString "-" system);
      configOf = system: self.nixosConfigurations."losos-desktop-${archOf system}".config;
    in
    {
      nixosModules.default = ./nixos/modules;

      overlays.default = import ./nixos/pkgs;

      nixosConfigurations = lib.listToAttrs (
        map (system: lib.nameValuePair "losos-desktop-${archOf system}" (mkSystem system)) systems
      );

      packages = forAllSystems (
        system: pkgs:
        let
          build = (configOf system).system.build;
          # The system's own package set, so these are the musl builds the
          # image carries rather than glibc ones built for the build host.
          ours = self.nixosConfigurations."losos-desktop-${archOf system}".pkgs;
        in
        {
          image = build.image;
          installer = build.installerImage;
          qcow2 = build.qcow2;
          release = build.releaseArtifacts;
          uki = build.uki;
          toplevel = build.toplevel;
          inherit (ours)
            pm
            pm-plugins
            losos-security
            losos-swap
            ;
          # Nix itself, from source against musl. The image carries none (an
          # update is a new /usr, not a switch); this is for a build host that
          # wants its Nix to be the same libc as what it builds.
          nix = ours.nix;
          default = build.image;
        }
      );

      checks = forAllSystems (
        system: pkgs:
        {
          # Building the toplevel evaluates every module and every assertion in
          # them, which is most of what can go wrong in a configuration.
          toplevel = self.packages.${system}.toplevel;
          # Runs losos-security's own test suite as part of the build.
          losos-security = self.packages.${system}.losos-security;
          losos-swap = self.packages.${system}.losos-swap;
          pm = self.packages.${system}.pm;
          pm-plugins = self.packages.${system}.pm-plugins;
        }
        # Boots the image under QEMU and checks the systemd pieces are actually
        # in place. Needs KVM, which the test driver asks for, so a builder
        # without it refuses rather than running for hours.
        // lib.optionalAttrs (system == "x86_64-linux") {
          boot = pkgs.testers.runNixOSTest (import ./nixos/tests/boot.nix { inherit self; });
        }
      );

      apps = forAllSystems (
        system: pkgs:
        let
          config = configOf system;
          machine =
            {
              x86_64-linux = "q35,accel=kvm:tcg";
              aarch64-linux = "virt,accel=kvm:tcg -cpu max";
            }
            .${system};
          vm = pkgs.writeShellApplication {
            name = "losos-desktop-vm";
            runtimeInputs = [
              pkgs.qemu
              pkgs.coreutils
            ];
            # A writable copy, grown to a size first boot can lay a whole disk
            # out on: repart needs room for slot B, root, /home and swap.
            text = ''
              disk=''${LOSOS_DISK:-losos-desktop.qcow2}
              vars=''${disk%.qcow2}-efivars.fd
              if [ ! -e "$disk" ]; then
                qemu-img create -f qcow2 -F qcow2 -b ${config.system.build.qcow2} "$disk" 40G
              fi
              # The firmware's variable store is per machine and writable:
              # systemd-boot records its boot counting and the loader's
              # partition UUID there.
              if [ ! -e "$vars" ]; then
                install -m 0644 ${pkgs.OVMF.variables} "$vars"
              fi
              exec qemu-system-${config.losos.arch.name} -machine ${machine} \
                -drive if=pflash,format=raw,readonly=on,file=${pkgs.OVMF.firmware} \
                -drive if=pflash,format=raw,file="$vars" \
                -m 4096 -smp 4 \
                -device virtio-vga -device virtio-net-pci,netdev=n0 -netdev user,id=n0 \
                -drive if=virtio,file="$disk" "$@"
            '';
          };
        in
        {
          vm = {
            type = "app";
            program = lib.getExe vm;
            meta.description = "Boot the LosOS Desktop image in QEMU with UEFI firmware";
          };
        }
      );

      devShells = forAllSystems (
        system: pkgs: {
          default = pkgs.mkShell {
            packages = with pkgs; [
              nixfmt
              qemu
              jq
              xz
              # repart, ukify and sysupdate, to inspect what an image contains
              # with the same tools that built it.
              systemd
              systemdUkify
            ];
          };
        }
      );

      formatter = forAllSystems (system: pkgs: pkgs.nixfmt-tree);
    };
}
