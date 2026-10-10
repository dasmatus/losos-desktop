# Hibernate the real image on a laptop and resume it.
#
# Booted as boot.nix boots it, with a TPM and a machine that says it is a
# laptop. The test checks the swap key is sealed to the TPM rather than
# random, hibernates, starts the machine again, and checks it came back to the
# same boot: the boot ID, a process and a file that was only ever in RAM are
# all still there. A second start without hibernating must boot afresh.
{ self }:

{ lib, ... }:

{
  name = "losos-desktop-hibernate";

  node.pkgsReadOnly = false;

  nodes.machine =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    {
      imports = [ self.nixosModules.default ];

      losos.version = "1";

      services.homed.promptOnFirstBoot = lib.mkForce false;
      systemd.services.derisk-setup.enable = false;

      # QEMU's SMBIOS says "Other". hostnamed, and hibernate.nix, take
      # /etc/machine-info's word over the firmware's.
      environment.etc."machine-info".text = ''
        CHASSIS=laptop
      '';

      # The test driver's shell is a virtio console, and its connection to
      # the driver closes when this image resumes: a shell started again
      # afterwards, as nixpkgs' own hibernation test does, talks to nobody.
      # So the shell stops before the image is written, and the resumed
      # system reports what the test checks on the serial console instead.
      powerManagement.powerDownCommands = "${config.systemd.package}/bin/systemctl --no-block stop backdoor.service";
      powerManagement.resumeCommands = ''
        PATH=${
          lib.makeBinPath [
            pkgs.coreutils
            pkgs.gnugrep
            config.systemd.package
          ]
        }
        # GRUB's request to start this version once, in grubenv (grub.cfg).
        env="$(bootctl --print-esp-path)/EFI/losos/grubenv"
        if grep -q '^losos_oneshot=.' "$env"; then oneshot=set; else oneshot=spent; fi
        echo "hibernate-test: boot $(cat /proc/sys/kernel/random/boot_id)," \
          "$(cat /run/hibernate-test/marker)," \
          "pid $(systemctl show -P MainPID hibernate-test-sleeper.service)," \
          "one-shot entry $oneshot" >/dev/ttyS0
      '';

      virtualisation = {
        directBoot.enable = false;
        mountHostNixStore = false;
        # The driver's shared directories are virtio-fs devices, and
        # virtio-fs refuses to freeze, so the kernel abandons the
        # hibernation with EOPNOTSUPP. Nothing here mounts them anyway.
        sharedDirectories = lib.mkForce { };
        useEFIBoot = true;
        fileSystems = lib.mkVMOverride { };
        # The image the kernel writes is the RAM in use, so less RAM is a
        # shorter test. losos-swap sizes swap to match.
        memorySize = 2048;
        tpm.enable = true;
      };

      boot.initrd.systemd.repart.device = "/dev/vda";
      boot.initrd.systemd.services.systemd-repart.after = lib.mkForce [ "dev-vda.device" ];

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
      import re
      import subprocess
      import tempfile

      tmp = tempfile.NamedTemporaryFile()
      subprocess.run([
        "${nodes.machine.virtualisation.qemu.package}/bin/qemu-img", "create",
        "-f", "qcow2", "-F", "raw",
        "-b", "${nodes.machine.system.build.disk}/${nodes.machine.image.filePath}",
        tmp.name, "32G",
      ], check=True)
      os.environ["NIX_DISK_IMAGE"] = tmp.name

      machine.wait_for_unit("multi-user.target")

      # The firmware's first boot is not like the ones after it: EDK II
      # records how much memory of each type that boot used and reserves
      # that much from the next boot on, which moves its ACPI tables. Linux
      # refuses an image whose memory map differs from the resuming boot's
      # ("Hibernate inconsistent memory map detected"), so a machine that
      # hibernated on the firmware's very first boot boots afresh instead.
      # A laptop's firmware is long past that, so start this one once more.
      # It also makes the second boot seal a second key, as every boot will.
      machine.shutdown()
      machine.start()
      machine.wait_for_unit("multi-user.target")

      with subtest("the swap key is sealed to the TPM"):
          machine.succeed("systemctl is-active losos-hibernate-swap.service")
          machine.succeed("cryptsetup isLuks /dev/disk/by-partlabel/losos-swap")
          machine.succeed("cryptsetup luksDump /dev/disk/by-partlabel/losos-swap | grep -q systemd-tpm2")
          # The key it was formatted with is gone: the TPM is the only way in.
          machine.succeed(
              "test \"$(cryptsetup luksDump /dev/disk/by-partlabel/losos-swap | grep -cE '^  [0-9]+: luks2$')\" = 1"
          )
          machine.succeed("swapon --show=NAME --noheadings | grep -q dm-")
          machine.succeed("test -e /dev/disk/by-designator/swap-luks")
          assert "yes" in machine.succeed(
              "busctl call org.freedesktop.login1 /org/freedesktop/login1 "
              "org.freedesktop.login1.Manager CanHibernate"
          )

      with subtest("the lid suspends, then hibernates"):
          machine.succeed("systemd-analyze cat-config systemd/logind.conf | grep -qx 'HandleLidSwitch=suspend-then-hibernate'")
          assert "yes" in machine.succeed(
              "busctl call org.freedesktop.login1 /org/freedesktop/login1 "
              "org.freedesktop.login1.Manager CanSuspendThenHibernate"
          )

      boot_id = machine.succeed("cat /proc/sys/kernel/random/boot_id").strip()
      machine.succeed(
          "mkdir /run/hibernate-test",
          "mount -t ramfs ramfs /run/hibernate-test",
          "echo only in RAM > /run/hibernate-test/marker",
          "systemd-run --unit=hibernate-test-sleeper sleep infinity",
      )
      sleeper = machine.succeed("systemctl show -P MainPID hibernate-test-sleeper.service").strip()

      with subtest("hibernate and resume into the same boot"):
          machine.execute("systemctl hibernate >&2 &", check_return=False)
          machine.wait_for_shutdown()
          machine.start()
          # The same boot, with a file and a process that only ever lived in
          # RAM, and GRUB's request to start this version once spent.
          machine.wait_for_console_text(
              re.escape(f"hibernate-test: boot {boot_id}, only in RAM, pid {sleeper}, one-shot entry spent")
          )

      with subtest("a cold boot does not resume, and seals a new key"):
          machine.crash()
          machine.start()
          machine.wait_for_unit("multi-user.target")
          assert machine.succeed("cat /proc/sys/kernel/random/boot_id").strip() != boot_id
          machine.fail("test -e /run/hibernate-test/marker")
          machine.succeed("systemctl is-active losos-hibernate-swap.service")
          machine.succeed("cryptsetup luksDump /dev/disk/by-partlabel/losos-swap | grep -q systemd-tpm2")
    '';
}
