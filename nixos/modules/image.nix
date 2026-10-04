# The image: an ESP and one dm-verity protected /usr slot, built by
# systemd-repart. Nothing else is on it. First boot grows it into a whole disk
# (disk.nix), and every later version arrives as a new /usr slot and a new UKI
# (update.nix).
#
# The pm tree drove mkosi for this, and needed plugins/ to get mkosi, xorriso
# and qemu-img past pm's fingerprint table. NixOS drives the same
# systemd-repart through image/repart.nix, so that problem does not exist here.
#
# What comes out of one configuration:
#
#   system.build.image             the OS as a disk: write it to a disk and boot
#   system.build.releaseArtifacts  what systemd-sysupdate downloads, plus the
#                                  disk image and the installer ISO
#                                  (installer.nix), with SHA256SUMS
#
# The installer used to be this image again, with a second UKI that booted
# into an install. installer.nix says why it is a separate live system now.
{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:

let
  cfg = config.losos;
  inherit (cfg) arch version;
  inherit (config.image.repart.verityStore) partitionIds;
  inherit (config.system.boot.loader) ukiFile;

  # nixpkgs hard-codes unshare in repart-image.nix. Add a local per-image
  # switch here so it also applies when the VM test imports this module.
  nixpkgsImageModules = "${modulesPath}/image";
  patchNixpkgsModule =
    name: replacements:
    let
      source = builtins.readFile "${nixpkgsImageModules}/${name}";
      patched = lib.foldl' (
        source: replacement:
        assert lib.assertMsg (lib.hasInfix replacement.from source)
          "Pinned nixpkgs ${name} is missing expected text: ${replacement.from}";
        builtins.replaceStrings [ replacement.from ] [ replacement.to ] source
      ) source replacements;
    in
    builtins.toFile name patched;

  repartImageModule = patchNixpkgsModule "repart-image.nix" [
    {
      from = "  createEmpty ? true,\n";
      to = "  createEmpty ? true,\n  useUnshare ? true,\n";
    }
    {
      from = "in\nstdenvNoCC.mkDerivation (";
      to = "  fakerootCommand = if useUnshare then \"unshare --map-root-user fakeroot\" else \"fakeroot\";\nin\nstdenvNoCC.mkDerivation (";
    }
    {
      from = builtins.concatStringsSep "\n" [
        "    nativeBuildInputs = ["
        "      systemd"
        "      util-linux"
        "      fakeroot"
        "    ]"
        "    ++ lib.optionals (compression.enable) ["
      ];
      to = builtins.concatStringsSep "\n" [
        "    nativeBuildInputs = ["
        "      systemd"
        "      fakeroot"
        "    ]"
        "    ++ lib.optionals useUnshare ["
        "      util-linux"
        "    ]"
        "    ++ lib.optionals (compression.enable) ["
      ];
    }
    {
      from = "      unshare --map-root-user fakeroot systemd-repart \\\n";
      to = "      " + "$" + "{fakerootCommand} systemd-repart \\\n";
    }
    {
      from = "./amend-repart-definitions.py";
      to = "${nixpkgsImageModules}/amend-repart-definitions.py";
    }
  ];
  repartModule = patchNixpkgsModule "repart.nix" [
    {
      from = "./repart-verity-store.nix";
      to = "${nixpkgsImageModules}/repart-verity-store.nix";
    }
    {
      from = "./file-options.nix";
      to = "${nixpkgsImageModules}/file-options.nix";
    }
    {
      from = "pkgs.callPackage ./repart-image.nix {";
      to = "pkgs.callPackage ${repartImageModule} {";
    }
    {
      from = "    package = lib.mkPackageOption pkgs \"systemd-repart\" {";
      to = builtins.concatStringsSep "\n" [
        "    useUnshare = lib.mkOption {"
        "      type = lib.types.bool;"
        "      default = true;"
        "      description = \"Run repart under an unshared user namespace; disable when the build environment prohibits nested user namespaces.\";"
        "    };"
        ""
        "    package = lib.mkPackageOption pkgs \"systemd-repart\" {"
      ];
    }
    {
      from = "                finalPartitions\n                ;";
      to = "                finalPartitions\n                useUnshare\n                ;";
    }
  ];

  id = config.system.image.id;
  systemdBoot = "${config.systemd.package}/lib/systemd/boot/efi/systemd-boot${arch.efi}.efi";

  # systemd-boot offers the newest entry by version, so no default is named on
  # an installed system: after an update the newest UKI is the one to try, and
  # boot counting (update.nix) falls back if it never gets blessed.
  loaderConf = pkgs.writeText "loader.conf" ''
    timeout 3
    editor no
  '';
in
{
  imports = [ repartModule ];

  system.image = {
    id = "losos-desktop";
    inherit version;
  };

  system.nixos = {
    distroId = "losos-desktop";
    distroName = "LosOS Desktop";
  };

  image.repart = {
    # nixpkgs unstable gates the whole repart image module on this switch.
    # Without it image.repart.image is never defined and every output built
    # from the disk fails to evaluate.
    enable = true;

    name = id;

    # The vfat ESP and erofs /usr formatter do not need a nested user namespace,
    # which cannot be created inside the hosted runner's Nix sandbox.
    useUnshare = false;

    # /usr is the Nix store on erofs, with a dm-verity hash tree beside it
    # whose root hash is baked into the UKI's command line as usrhash=. That
    # one value is what makes /usr immutable: a changed block fails
    # verification, and a different /usr needs a different UKI.
    verityStore = {
      enable = true;
      ukiPath = "/EFI/Linux/${ukiFile}";
    };

    partitions = {
      ${partitionIds.esp} = {
        contents = {
          # The removable-media path, which firmware boots with no boot entry
          # registered -- so this image needs no efibootmgr and no install step.
          "/EFI/BOOT/BOOT${lib.toUpper arch.efi}.EFI".source = systemdBoot;
          "/EFI/systemd/systemd-boot${arch.efi}.efi".source = systemdBoot;
          "/loader/loader.conf".source = loaderConf;
        };
        repartConfig = {
          Type = "esp";
          Format = "vfat";
          SizeMinBytes = "512M";
          SizeMaxBytes = "512M";
        };
      };

      # Fixed at 128M rather than minimised: the hash tree of an 8G /usr is
      # about 64M, and slot A's verity partition cannot grow later because
      # slot A's data partition sits directly behind it.
      ${partitionIds.store-verity}.repartConfig = {
        Label = "${id}_${version}";
        # repart defaults verity blocks to the image's 512-byte sector size,
        # which makes the hash tree a sixteenth of the data: 332M for a 4.8G
        # store, measured, and it did not fit. At 4K it is 1/128.
        VerityDataBlockSizeBytes = 4096;
        VerityHashBlockSizeBytes = 4096;
        Minimize = lib.mkForce "off";
        SizeMinBytes = "128M";
        SizeMaxBytes = "128M";
      };

      # The label carries the version because that is where systemd-sysupdate
      # reads a partition's version from. The data partition is minimised here
      # and grown to its slot size by the first boot's repart run.
      ${partitionIds.store}.repartConfig = {
        Label = "${id}_${version}";
        VerityDataBlockSizeBytes = 4096;
        VerityHashBlockSizeBytes = 4096;
      };
    };
  };

  # What a release uploads. The file names are the contract with update.nix:
  # sysupdate matches them with MatchPattern, so the two are written from the
  # same arch table and cannot drift.
  #
  # The /usr halves are cut out of the finished disk image using the offsets
  # repart itself reported, rather than built a second time, so the bytes
  # sysupdate installs are the bytes the image boots. Their names carry the
  # partition UUIDs repart derived from the verity root hash, because that is
  # how the initrd finds /usr from usrhash= on the command line, and
  # sysupdate gives a partition the UUID its source's name carries (@u).
  system.build.releaseArtifacts =
    let
      image = config.system.build.image;
      raw = "${config.image.baseName}.raw";
      prefix = "${id}_${version}";
    in
    pkgs.runCommand "${prefix}-release"
      {
        nativeBuildInputs = with pkgs.buildPackages; [
          jq
          qemu-utils
          xz
        ];
      }
      ''
        mkdir -p $out
        cd $out

        extract() {
          local type=$1 offset size uuid
          field() {
            jq -er --arg t "$type" ".[] | select(.type==\$t) | .$1" ${image}/repart-output.json
          }
          offset=$(field offset)
          size=$(field raw_size)
          uuid=$(field uuid)
          dd if=${image}/${raw} of="${prefix}_''${type}_$uuid.raw" bs=1M iflag=skip_bytes,count_bytes \
            skip="$offset" count="$size" status=none
          xz --threads=$NIX_BUILD_CORES "${prefix}_''${type}_$uuid.raw"
        }

        cp ${config.system.build.uki}/${ukiFile} ${prefix}_${arch.name}.efi
        extract ${arch.usr}
        extract ${arch.usrVerity}

        xz --threads=$NIX_BUILD_CORES -c ${image}/${raw} > ${prefix}_${arch.name}.raw.xz
        cp ${config.system.build.installerIso} ${prefix}_${arch.name}-installer.iso
        qemu-img convert -f raw -O qcow2 ${image}/${raw} ${prefix}_${arch.name}.qcow2

        sha256sum -- * > SHA256SUMS
      '';

  system.build.qcow2 =
    pkgs.runCommand "${id}_${version}.qcow2" { nativeBuildInputs = [ pkgs.buildPackages.qemu-utils ]; }
      ''
        qemu-img convert -f raw -O qcow2 \
          ${config.system.build.image}/${config.image.baseName}.raw $out
      '';
}
