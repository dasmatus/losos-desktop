# Boot the real image -- UEFI, systemd-boot, the UKI, a disk first boot has to
# lay out -- and check that each systemd piece this OS is built on is doing its
# job, not merely installed.
#
# The pm tree could not do this at all: nothing there had ever booted, and
# docs/boot.md said so. This test is the first thing in the repository that
# can fail because the design is wrong rather than because a build is.
{ self }:

{ lib, ... }:

{
  name = "losos-desktop-boot";

  # The module adds its own overlay, which the test framework's shared,
  # read-only package set would refuse.
  node.pkgsReadOnly = false;

  nodes.machine =
    { config, lib, ... }:
    {
      imports = [ self.nixosModules.default ];

      losos.version = "1";

      # The first-boot wizard waits on the console for a person, which a test
      # has none of. The user is created by the test script instead.
      services.homed.promptOnFirstBoot = lib.mkForce false;

      # Boot the image as firmware would, from its own disk, instead of the
      # test driver's kernel with the host's store mounted in.
      virtualisation = {
        directBoot.enable = false;
        mountHostNixStore = false;
        useEFIBoot = true;
        fileSystems = lib.mkVMOverride { };
        memorySize = 3072;
      };

      # The VM's first-boot disk is /dev/vda. Repart cannot infer it from
      # /sysroot before the root partition exists.
      boot.initrd.systemd.repart.device = "/dev/vda";
      boot.initrd.systemd.services.systemd-repart.after = lib.mkForce [ "dev-vda.device" ];

      # The VM module empties swapDevices with the same override it uses on
      # fileSystems. Put back what disk.nix gives the real image, so the test
      # boots the same fstab and zswap has the backing device it asserts on.
      swapDevices = lib.mkVMOverride [
        {
          device = "/dev/mapper/swap";
          options = [ "nofail" ];
        }
      ];
    };

  testScript =
    { nodes, ... }:
    ''
      import os
      import subprocess
      import tempfile

      # A copy-on-write disk backed by the image, grown so first boot has room
      # for slot B, root, /home and swap.
      tmp = tempfile.NamedTemporaryFile()
      subprocess.run([
        "${nodes.machine.virtualisation.qemu.package}/bin/qemu-img", "create",
        "-f", "qcow2", "-F", "raw",
        "-b", "${nodes.machine.system.build.image}/${nodes.machine.image.filePath}",
        tmp.name, "32G",
      ], check=True)
      os.environ["NIX_DISK_IMAGE"] = tmp.name

      machine.wait_for_unit("multi-user.target")

      with subtest("first boot laid out the disk with systemd-repart"):
          machine.succeed("findmnt --kernel --types ext4 /")
          machine.succeed("findmnt --kernel /home")
          # slot B exists and is waiting for sysupdate
          machine.succeed("sfdisk --json /dev/vda | grep -c '\"name\": \"_empty\"' | grep -qx 2")
          machine.succeed("swapon --show=NAME --noheadings | grep -q dm-")

      with subtest("/usr is dm-verity and the store lives on it"):
          assert "ACTIVE" in machine.succeed("dmsetup info --target verity usr")
          machine.succeed("df --output=source /nix/store | tail -n1 | grep -qx /dev/mapper/usr")

      with subtest("the systemd services the design rests on are running"):
          for unit in [
              "systemd-homed.service",
              "systemd-userdbd.socket",
              "systemd-networkd.service",
              "systemd-resolved.service",
              "systemd-oomd.service",
              "systemd-factory-reset.socket",
              "systemd-sysupdate.timer",
              "display-manager.service",
          ]:
              machine.wait_for_unit(unit)
          machine.fail("systemctl is-active NetworkManager.service")

      with subtest("a user is a homed LUKS volume"):
          # homed's first-boot wizard runs only while no regular user exists,
          # so nothing the image ships -- derisk-greeter included -- may count
          # as one. accounts.nix silences a NixOS warning on that reading.
          # userdbctl also prints "begin/end systemd-homed users" marker rows
          # for the UID ranges a filter covers, and homed's range is regular,
          # so without --boundaries=no the output is never empty.
          machine.fail(
              "userdbctl user --disposition=regular --no-legend --boundaries=no | grep -q ."
          )
          machine.succeed(
              "NEWPASSWORD=correct-horse homectl create alice --storage=luks "
              "--disk-size=256M --member-of=wheel --enforce-password-policy=no"
          )
          machine.succeed("userdbctl user alice | grep -q 'Storage: luks'")
          # glibc's NSS reaches nss-systemd through the configured nscd
          # forwarder, so homed users must be visible to ordinary lookups.
          machine.succeed("getent passwd alice")

      with subtest("sysupdate sees the installed version"):
          # The test VM has no route to the release feed, and without
          # --offline sysupdate fails on the download before listing what is
          # installed.
          machine.succeed(
              "${nodes.machine.systemd.package}/lib/systemd/systemd-sysupdate --offline list | grep -q '1'"
          )

      with subtest("pm is installed as the system manager"):
          machine.succeed("pm --help")
          machine.succeed("pm source-path https://example.org/x-1.0.tar.xz | grep -q x-1.0.tar.xz")

      with subtest("the security report answers on the system bus"):
          machine.succeed(
              "busctl call io.losos.Security1 /io/losos/Security1 "
              "org.freedesktop.DBus.Introspectable Introspect"
          )
    '';
}
