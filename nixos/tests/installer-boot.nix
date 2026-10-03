# Boot the installer from its ISO under UEFI, including the initrd mounts and
# the service that owns tty1. The ISO is the shipping one plus the driver's
# backdoor service (installerIsoTest): with the node booted from a CD-ROM
# rather than directly, nothing else puts a shell on hvc0 for the driver.
{ self }:

let
  installerIso = self.nixosConfigurations."losos-desktop-x86_64".config.system.build.installerIsoTest;
in
{
  name = "losos-desktop-installer-boot";

  nodes.machine =
    { ... }:
    {
      virtualisation = {
        directBoot.enable = false;
        mountHostNixStore = false;
        useEFIBoot = true;
        qemu.options = [
          "-cdrom ${installerIso}"
          "-boot order=d"
        ];
        memorySize = 3072;
      };
    };

  testScript = ''
    machine.succeed("findmnt --kernel /iso")
    machine.succeed("findmnt --kernel /nix/store")
    machine.wait_for_unit("losos-installer.service")
    machine.succeed(
        "systemctl show losos-installer.service --property=TTYPath --value "
        "| grep -qx /dev/tty1"
    )
  '';
}
