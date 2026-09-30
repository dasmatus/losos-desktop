# The image: an ESP and one dm-verity protected /usr slot, built by
# systemd-repart. Nothing else is on it. First boot grows it into a whole disk
# (disk.nix), and every later version arrives as a new /usr slot and a new UKI
# (update.nix).
#
# The pm tree drove mkosi for this, and needed plugins/ to get mkosi, xorriso
# and qemu-img past pm's fingerprint table. NixOS drives the same
# systemd-repart through image/repart.nix, so that problem does not exist here.
#
# Three images come out of one configuration:
#
#   system.build.image           the OS as a disk: write it to a disk and boot
#   system.build.installerImage  the same bytes plus an installer UKI, which is
#                                the default boot entry; write it to a USB stick
#   system.build.releaseArtifacts  what systemd-sysupdate downloads, plus both
#                                images, with SHA256SUMS
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

  id = config.system.image.id;
  systemdBoot = "${config.systemd.package}/lib/systemd/boot/efi/systemd-boot${arch.efi}.efi";

  # systemd-boot offers the newest entry by version, so no default is named on
  # an installed system: after an update the newest UKI is the one to try, and
  # boot counting (update.nix) falls back if it never gets blessed.
  loaderConf = pkgs.writeText "loader.conf" ''
    timeout 3
    editor no
  '';

  # The installer medium boots the installer unless someone picks otherwise.
  installerLoaderConf = pkgs.writeText "loader.conf" ''
    timeout 5
    editor no
    default ${id}-installer_*
  '';

  installerUkiFile = "${id}-installer_${version}.efi";

  # The installer UKI is the ordinary one with three words appended. systemd
  # takes the last root= it is given, so root=tmpfs overrides the gpt-auto root
  # the ordinary command line names: the installer must not create partitions
  # on the medium it is about to copy, and a tmpfs root needs none. usrhash is
  # the same value in both, because both boot the same /usr.
  #
  # This is the NixOS verity-store module's own UKI build with a different
  # command line. It reads the roothash out of the intermediate image rather
  # than taking it as an argument, so it cannot be built against a /usr other
  # than the one on the medium.
  installerUki =
    pkgs.runCommand installerUkiFile
      {
        nativeBuildInputs = [
          pkgs.buildPackages.jq
          pkgs.buildPackages.systemdUkify
        ];
      }
      ''
        mkdir -p $out
        usrhash=$(jq -r \
          '.[] | select(.type=="${arch.usrVerity}") | .roothash' \
          ${config.system.build.intermediateImage}/repart-output.json)
        ukify build \
          --config=${config.boot.uki.configFile} \
          --cmdline="init=${config.system.build.toplevel}/init ${toString config.boot.kernelParams} usrhash=$usrhash root=tmpfs losos.install systemd.unit=losos-install.target" \
          --output="$out/${installerUkiFile}"
      '';
in
{
  imports = [ "${modulesPath}/image/repart.nix" ];

  system.image = {
    id = "losos-desktop";
    inherit version;
  };

  system.nixos = {
    distroId = "losos-desktop";
    distroName = "LosOS Desktop";
  };

  image.repart = {
    name = id;

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

  system.build.installerUki = installerUki;

  system.build.installerImage = config.system.build.image.overrideAttrs (
    _: previousAttrs: {
      name = "${id}-installer_${version}";
      finalPartitions = lib.recursiveUpdate previousAttrs.finalPartitions {
        ${partitionIds.esp}.contents = {
          "/EFI/Linux/${installerUkiFile}".source = "${installerUki}/${installerUkiFile}";
          "/loader/loader.conf".source = installerLoaderConf;
        };
      };
    }
  );

  # What a release uploads. The file names are the contract with update.nix:
  # sysupdate matches them with MatchPattern, so the two are written from the
  # same arch table and cannot drift.
  #
  # The /usr halves are cut out of the finished disk image using the offsets
  # repart itself reported, rather than built a second time, so the bytes
  # sysupdate installs are the bytes the image boots.
  system.build.releaseArtifacts =
    let
      image = config.system.build.image;
      installer = config.system.build.installerImage;
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
          local type=$1 dest=$2 offset size
          offset=$(jq -r --arg t "$type" '.[] | select(.type==$t) | .offset' ${image}/repart-output.json)
          size=$(jq -r --arg t "$type" '.[] | select(.type==$t) | .raw_size' ${image}/repart-output.json)
          dd if=${image}/${raw} of="$dest" bs=1M iflag=skip_bytes,count_bytes \
            skip="$offset" count="$size" status=none
          xz --threads=$NIX_BUILD_CORES "$dest"
        }

        cp ${config.system.build.uki}/${ukiFile} ${prefix}_${arch.name}.efi
        extract ${arch.usr} ${prefix}_${arch.usr}.raw
        extract ${arch.usrVerity} ${prefix}_${arch.usrVerity}.raw

        xz --threads=$NIX_BUILD_CORES -c ${image}/${raw} > ${prefix}_${arch.name}.raw.xz
        xz --threads=$NIX_BUILD_CORES -c ${installer}/${raw} > ${prefix}_${arch.name}-installer.raw.xz
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
