# Updates replace; they do not patch.
#
# systemd-sysupdate keeps two of each thing it manages: two UKIs on the ESP,
# two /usr slots, two verity hash trees. An update downloads the new version
# into whichever slot is not booted and touches neither the running system
# nor /home. There is no nixos-rebuild here and no Nix on the machine to run
# one with.
#
# Rollback needs no failure detection. systemd-boot offers the newest UKI;
# a UKI installed by sysupdate carries a boot counter in its file name;
# systemd-bless-boot removes the counter once boot-complete.target is reached;
# an entry that never gets blessed runs out of tries and systemd-boot falls
# back to the older one by itself. The older UKI's usrhash still names the
# older /usr, which is still there.
{ config, lib, ... }:

let
  cfg = config.losos;
  inherit (cfg) arch;
  id = config.system.image.id;

  # The same partition-label contract image.nix writes: "<id>_<version>".
  #
  # The source name carries the partition's UUID (@u), and sysupdate gives the
  # partition it writes that UUID. It has to: the UKI finds /usr by the UUIDs
  # repart derived from usrhash=, so a slot that kept the random UUID repart
  # created it with would hold the right bytes and never be found.
  partition = type: {
    Transfer.ProtectVersion = "%A";
    Source = {
      Type = "url-file";
      Path = cfg.update.baseUrl;
      MatchPattern = "${id}_@v_${type}_@u.raw.xz";
    };
    Target = {
      Type = "partition";
      Path = "auto";
      MatchPattern = "${id}_@v";
      MatchPartitionType = type;
      ReadOnly = true;
      InstancesMax = 2;
    };
  };
in
{
  warnings = lib.optional (cfg.update.pubring == null) ''
    losos.update.pubring is unset, so systemd-sysupdate will install updates
    without verifying SHA256SUMS.gpg. Set it to the release signing key's
    public keyring before shipping an image to anyone.
  '';

  environment.etc."systemd/import-pubring.gpg" = lib.mkIf (cfg.update.pubring != null) {
    source = cfg.update.pubring;
  };

  systemd.sysupdate = {
    enable = true;

    # The timer is how updates appear without anyone asking. It downloads and
    # installs; it does not reboot, because a desktop that restarts itself
    # under its user loses their work. The next boot picks the new entry.
    reboot.enable = false;

    transfers = {
      # All three belong to one version and sysupdate installs them as a set.
      # A /usr slot no UKI names is inert, so the UKI is the half that decides
      # whether a new version boots at all.
      "10-usr-verity" = lib.recursiveUpdate (partition arch.usrVerity) {
        Transfer.Verify = cfg.update.pubring != null;
      };
      "20-usr" = lib.recursiveUpdate (partition arch.usr) {
        Transfer.Verify = cfg.update.pubring != null;
      };

      "30-uki" = {
        Transfer = {
          ProtectVersion = "%A";
          Verify = cfg.update.pubring != null;
        };
        Source = {
          Type = "url-file";
          Path = cfg.update.baseUrl;
          MatchPattern = "${id}_@v_${arch.name}.efi";
        };
        Target = {
          Type = "regular-file";
          Path = "/EFI/Linux";
          PathRelativeTo = "esp";
          # Every name a UKI of this OS can have on the ESP: the image's own
          # (uncounted), a fresh install (+tries left), one mid-assessment
          # (+left-done), and one already blessed (counter removed again).
          MatchPattern = [
            "${id}_@v+@l-@d.efi"
            "${id}_@v+@l.efi"
            "${id}_@v.efi"
          ];
          Mode = "0444";
          TriesLeft = 3;
          TriesDone = 0;
          # Two kernels kept: the one running and the one to fall back to.
          InstancesMax = 2;
        };
      };
    };
  };
}
