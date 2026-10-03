# Boot the installer from its actual ISO under UEFI, including the initrd
# mounts and the service that owns tty1.
{ self }:

let
  inherit (self.nixosConfigurations."losos-desktop-x86_64".config.system.build) installerSystem;

  # The released ISO has no backdoor shell, which is how the test driver runs
  # every command, so booting it as is leaves the driver waiting on hvc0 until
  # the build times out. The same live system with nixpkgs' test
  # instrumentation added is booted instead: it is still an ISO, still started
  # by the firmware, with the same initrd mounts and the same tty1 service.
  installerIso =
    (installerSystem.extendModules {
      modules = [ "${installerSystem.pkgs.path}/nixos/modules/testing/test-instrumentation.nix" ];
    }).config.system.build.isoImage;
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
