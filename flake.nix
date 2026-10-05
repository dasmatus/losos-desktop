# LosOS Desktop as NixOS.
#
# A derisk desktop that uses systemd for everything it can, installs as an
# image, updates with systemd-sysupdate and makes every user a systemd-homed
# LUKS volume, written as a NixOS configuration. It replaced a from-source
# distribution of pm recipes. docs/nixos.md maps the old pieces to the new
# ones, and nixos/ holds the modules.
#
#   nix build                      the disk image (.#image)
#   nix build .#installer          the installer ISO: Wi-Fi, a disk, sysupdate
#   nix build .#release            the files CI publishes through the proxy
#   nix run .#vm                   boot the image in QEMU with UEFI firmware
#   nix build .#pm-payloads.<pkg>  any nixpkgs package, static, for a pm build file
#   nix flake check                evaluate both architectures, test losos-security
#                                  and losos-installer, and (with KVM) boot the image
{
  description = "LosOS Desktop: a systemd-native, image-based derisk desktop on NixOS";

  inputs = {
    # The channel tarball rather than github:NixOS/nixpkgs. It is the same
    # tree at the revision the channel's Hydra jobset tested, and resolves to
    # an immutable releases.nixos.org URL that flake.lock pins by narHash
    # exactly as it would a git revision. It is also the form that
    # fetches without api.github.com, which rate-limits anonymous clients and
    # which some build hosts cannot reach at all. `nix flake update nixpkgs`
    # moves it.
    #
    # nixos-unstable, the channel rather than nixpkgs master: Hydra has built
    # and tested every revision it points at, so the stock closures still
    # substitute from cache.nixos.org. An update to this OS is a whole new
    # image that CI builds and the VM test boots before it is published, so
    # a breaking change upstream stops at CI rather than on a machine, and
    # tracking unstable buys the newer kernel, Mesa and systemd that the
    # desktop and the Halium target (nixos/halium/) want.
    nixpkgs.url = "https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz";
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

      proxyUrl = builtins.getEnv "LOSOS_PROXY_URL";

      requireProxy =
        value:
        if proxyUrl == "" then
          throw "Set LOSOS_PROXY_URL and pass --impure to build image artifacts."
        else
          value;

      mkSystem =
        system:
        lib.nixosSystem {
          modules = [
            self.nixosModules.default
            {
              # Use nixpkgs' standard glibc platform so its stock package
              # closures can substitute from cache.nixos.org.
              nixpkgs.hostPlatform = system;
              losos.version = lib.mkDefault version;
            }
            (
              { config, ... }:
              lib.optionalAttrs (proxyUrl != "") {
                # The image's update source and the artifact published to the
                # proxy must agree; the URL is a repository variable in CI.
                losos.update.baseUrl = lib.mkDefault "${lib.removeSuffix "/" proxyUrl}/updates/${config.losos.channel}/${archOf system}/";
                losos.ping.url = lib.mkDefault "${lib.removeSuffix "/" proxyUrl}/ping";
              }
            )
          ];
        };

      # The Halium build: the same OS on an Android phone's hardware layer
      # (nixos/halium/). arm64 only, since that is what Halium devices are.
      # This configuration has no device kernel and boots no device; it is
      # what CI evaluates and builds, and a device port is this plus its
      # losos.halium settings.
      haliumSystem = lib.nixosSystem {
        modules = [
          self.nixosModules.halium
          {
            nixpkgs.hostPlatform = "aarch64-linux";
            losos.version = lib.mkDefault version;
          }
        ];
      };

      archOf = system: lib.head (lib.splitString "-" system);
      configOf = system: self.nixosConfigurations."losos-desktop-${archOf system}".config;
    in
    {
      nixosModules.default = ./nixos/modules;
      nixosModules.halium = ./nixos/halium;

      overlays.default = import ./nixos/pkgs;

      nixosConfigurations =
        lib.listToAttrs (
          map (system: lib.nameValuePair "losos-desktop-${archOf system}" (mkSystem system)) systems
        )
        // {
          losos-desktop-halium-aarch64 = haliumSystem;
        };

      packages = forAllSystems (
        system: pkgs:
        let
          build = (configOf system).system.build;
          # The system's package set, so these are the stock glibc builds the
          # image carries rather than packages built for the build host.
          ours = self.nixosConfigurations."losos-desktop-${archOf system}".pkgs;
        in
        {
          image = requireProxy build.image;
          installer = requireProxy build.installerIso;
          qcow2 = requireProxy build.qcow2;
          release = requireProxy build.releaseArtifacts;
          uki = requireProxy build.uki;
          toplevel = build.toplevel;
          inherit (ours)
            derisk
            pm
            pm-plugins
            losos-installer
            losos-security
            losos-swap
            ;
          # The image carries no Nix (an update is a new /usr, not a switch);
          # this package is for a build host that wants matching Nix.
          nix = ours.nix;
          default = requireProxy build.image;
        }
        // lib.optionalAttrs (system == "aarch64-linux") {
          # boot.img and rootfs.img.xz, with SHA256SUMS. No update URL is
          # baked into them, so this needs no LOSOS_PROXY_URL.
          halium = haliumSystem.config.system.build.haliumImages;
          inherit (ours) libhybris;
        }
      );

      # The whole of nixpkgs at the pinned revision, as this OS builds it:
      # glibc, with this repository's overlay. `nix build .#<package>`
      # reaches any of it, and `.#pm-payloads.<package>` is the same package
      # static and laid out for a pm build file to install (docs/nixos.md,
      # "nixpkgs in a pm build"). It is the system's own package set, so a
      # package here and the same package in the image are one store path.
      legacyPackages = forAllSystems (
        system: _: self.nixosConfigurations."losos-desktop-${archOf system}".pkgs
      );

      checks = forAllSystems (
        system: pkgs:
        {
          # Building the toplevel evaluates every module and every assertion in
          # them, which is most of what can go wrong in a configuration.
          toplevel = self.packages.${system}.toplevel;
          # Run each program's own test suite as part of its build.
          losos-installer = self.packages.${system}.losos-installer;
          losos-security = self.packages.${system}.losos-security;
          losos-swap = self.packages.${system}.losos-swap;
          derisk = self.packages.${system}.derisk;
          pm = self.packages.${system}.pm;
          pm-plugins = self.packages.${system}.pm-plugins;
        }
        // lib.optionalAttrs (system == "aarch64-linux") {
          # Every Halium module and assertion, libhybris's build, and the
          # images themselves: mkbootimg, the ext4 rootfs and its
          # compression only run when the images are built.
          halium = self.packages.${system}.halium;
          libhybris = self.packages.${system}.libhybris;
        }
        # Boots the image under QEMU and checks the systemd pieces are actually
        # in place. Needs KVM, which the test driver asks for, so a builder
        # without it refuses rather than running for hours.
        // lib.optionalAttrs (system == "x86_64-linux") {
          boot = pkgs.testers.runNixOSTest (import ./nixos/tests/boot.nix { inherit self; });
          installer-boot = pkgs.testers.runNixOSTest (
            import ./nixos/tests/installer-boot.nix { inherit self; }
          );
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
            program = lib.getExe (requireProxy vm);
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
