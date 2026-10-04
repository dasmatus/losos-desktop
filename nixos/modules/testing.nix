# A unit that exists for CI and does nothing on an installed system: it is
# conditioned on a word on the kernel command line that only a test boot
# carries.
{
  config,
  pkgs,
  ...
}:

{
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

    # losos-ota-test used to be here: on a boot whose command line said
    # `losos.ota-test`, it fetched a release over plain HTTP from 10.0.2.2,
    # installed it with `systemd-sysupdate --verify=no` and rebooted. No test
    # ever booted with that word, and the unit shipped in every image, so the
    # only thing it could do on real hardware was install an unverified /usr
    # and UKI, for whoever controls the command line and that address, past
    # any signing key the image is given. A test that exercises sysupdate
    # should rewrite the transfers from the test driver, not from a unit
    # inside the image it is testing.
  };
}
