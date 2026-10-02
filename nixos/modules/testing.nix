# Two units that exist for CI and do nothing on an installed system: each is
# conditioned on a word on the kernel command line that only a test boot
# carries.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos;
in
{
  options.losos.test.otaPort = lib.mkOption {
    type = lib.types.port;
    default = 8730;
    description = ''
      The port on the QEMU host that serves a release directory to a guest
      booted with `losos.ota-test`.
    '';
  };

  config.systemd.services = {
    # The one thing a VM test can observe on a machine with no shell and no
    # SSH. Ordered after boot-complete.target, which systemd reaches only when
    # nothing essential failed, so the marker asserts that startup finished
    # clean rather than merely that the kernel started.
    losos-selftest = {
      description = "Report that this boot reached a good state";
      documentation = [ "man:systemd.special(7)" ];
      after = [ "boot-complete.target" ];
      requires = [ "boot-complete.target" ];
      wantedBy = [ "multi-user.target" ];
      unitConfig.ConditionKernelCommandLine = "losos.selftest";
      path = [
        config.systemd.package
        pkgs.gnugrep
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        StandardOutput = "journal+console";
      };
      # `is-system-running --wait` exits non-zero on a degraded boot, so this
      # is an assertion: a boot with a failed unit prints no marker.
      script = ''
        systemctl is-system-running --wait
        systemd-analyze --no-pager
        echo "LOSOS-SELFTEST-OK $(grep ^IMAGE_VERSION= /etc/os-release)"
        systemctl poweroff
      '';
    };

    # Drive systemd-sysupdate from inside the VM. The update has to happen in
    # the running system's own context so that sysupdate sees the real ESP and
    # the real partition table. The shipped definitions point at GitHub; under
    # QEMU user networking the host is always 10.0.2.2, so a copy with the
    # source rewritten goes to /run -- not /etc, because it must not survive
    # the reboot it is about to trigger.
    losos-ota-test = {
      description = "Apply a sysupdate from the CI host, then reboot";
      documentation = [ "man:systemd-sysupdate(8)" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      before = [ "losos-selftest.service" ];
      wantedBy = [ "multi-user.target" ];
      unitConfig.ConditionKernelCommandLine = "losos.ota-test";
      path = [
        config.systemd.package
        pkgs.gnused
        pkgs.coreutils
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        StandardOutput = "journal+console";
      };
      script = ''
        mkdir -p /run/losos-ota.d
        for f in /etc/sysupdate.d/*.transfer; do
          sed 's|^Path=https://.*|Path=http://10.0.2.2:${toString cfg.test.otaPort}/|' "$f" \
            > "/run/losos-ota.d/$(basename "$f")"
        done
        ${config.systemd.package}/lib/systemd/systemd-sysupdate \
          --definitions=/run/losos-ota.d --verify=no update
        systemctl reboot
      '';
    };
  };
}
