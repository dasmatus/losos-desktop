# Driver choice at boot (hardware.nix, nvidia.nix): nixos-facter probes the
# machine before udev's coldplug, losos-hardware writes the plan, and kmod and
# systemd-modules-load act on it.
#
# A VM has no NVIDIA card, so the last subtest feeds losos-hardware a report
# that has one, an RTX 4090, and checks what follows: nouveau is kept off
# the card and NVIDIA's modules no longer are, NVIDIA's module is asked for
# and, finding no GPU, refuses, and the fallback brings nouveau back. That is the path a real card takes when
# the open module rejects it; the path where it accepts the card needs one.
#
# Only the two modules under test, not the whole image, so it boots the test
# driver's kernel quickly and runs without KVM too.
{ lib, ... }:

{
  name = "losos-desktop-hardware";

  # The modules' packages come from the OS's overlay.
  node.pkgsReadOnly = false;

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [
        ../modules/hardware.nix
        ../modules/nvidia.nix
      ];
      nixpkgs.overlays = [ (import ../pkgs) ];

      # hardware.nix asserts zswap has a swap device, which this VM lacks
      # and which is not what it tests.
      boot.zswap.enable = lib.mkForce false;
      # desktop.nix's, which /run/opengl-driver needs.
      hardware.graphics.enable = true;

      environment.systemPackages = [
        pkgs.jq
        pkgs.losos-hardware
      ];
    };

  testScript = ''
    import json
    import re

    machine.wait_for_unit("multi-user.target")

    with subtest("facter probed the machine and the plan was written before modules-load"):
        machine.succeed("systemctl is-active losos-hardware.service losos-hardware-fallback.service")
        machine.succeed("jq -e '.hardware.graphics_card | length > 0' /run/losos/hardware/facter.json")
        machine.succeed("jq -e '.matched == []' /run/losos/hardware/plan.json")
        done = int(machine.succeed("systemctl show -P ExecMainExitTimestampMonotonic losos-hardware.service"))
        for unit in ["systemd-modules-load.service", "systemd-udev-trigger.service"]:
            started = int(machine.succeed(f"systemctl show -P ExecMainStartTimestampMonotonic {unit}"))
            assert done <= started, f"{unit} started before the plan was written"
        machine.fail("systemctl --failed --no-legend | grep .")

    with subtest("services for hardware the VM lacks were left off"):
        # thermald: the CPU is whatever the host's is, but inside a VM.
        machine.succeed("systemctl show -P ConditionResult thermald.service | grep -qx no")
        machine.fail("test -e /run/losos/hardware/flags/intel-cpu")
        # fprintd: no reader, so a D-Bus start of it is refused.
        machine.fail("systemctl start fprintd.service && systemctl is-active fprintd.service")
        machine.fail("mountpoint -q /run/nvidia-suspend")

    # What udev would load for an RTX 4090, honouring blacklists as coldplug
    # does: a module name in the dry run's insmod lines.
    rtx = "pci:v000010DEd00002684sv00000000sd00000000bc03sc00i00"
    def drivers_for(alias):
        _, out = machine.execute(f"modprobe --dry-run --verbose --use-blacklist {alias} 2>&1")
        return set(re.findall(r"/(nouveau|nvidia[a-z_-]*)\.ko", out))

    with subtest("NVIDIA's modules are in the image and nothing loads them uninvited"):
        machine.succeed("modinfo -n nvidia nvidia_drm nvidia_modeset nvidia_uvm")
        machine.fail("test -d /sys/module/nvidia")
        assert drivers_for(rtx) == {"nouveau"}, drivers_for(rtx)
        # GBM and EGL find NVIDIA's backends where Mesa's loaders look.
        machine.succeed("test -e /run/opengl-driver/lib/gbm/nvidia-drm_gbm.so")
        machine.succeed("test -e /run/opengl-driver/share/glvnd/egl_vendor.d/10_nvidia.json")
        machine.succeed("ls /etc/egl/egl_external_platform.d/ | grep -q gbm")

    with subtest("a supported card goes to the open module, and to nouveau when that refuses it"):
        report = {
            "version": 1,
            "hardware": {
                "graphics_card": [{
                    "vendor": {"hex": "10de", "value": 0x10DE},
                    "device": {"hex": "2684", "value": 0x2684},
                    "sysfs_bus_id": "0000:01:00.0",
                    "model": "nVidia AD102 [GeForce RTX 4090]",
                }],
            },
        }
        machine.succeed(f"echo '{json.dumps(report)}' > /tmp/rtx.json")
        machine.succeed("losos-hardware plan --rules /etc/losos/hardware-rules.json --report /tmp/rtx.json")
        machine.succeed("test -e /run/losos/hardware/flags/nvidia")
        machine.succeed("modprobe --showconfig | grep -qx 'blacklist nouveau'")
        # Coldplug would now pick NVIDIA's module for it, and not nouveau.
        assert "nvidia" in drivers_for(rtx) and "nouveau" not in drivers_for(rtx), drivers_for(rtx)
        # nvidia.ko finds no GPU here and refuses to load.
        machine.fail("systemctl restart systemd-modules-load.service")
        machine.fail("test -d /sys/module/nvidia_drm")
        machine.succeed("losos-hardware fallback")
        machine.succeed("test -d /sys/module/nouveau")
        # Video memory is saved over suspend to RAM, not the root partition.
        machine.succeed("systemctl start /run/nvidia-suspend")
        machine.succeed("findmnt --types tmpfs /run/nvidia-suspend")
  '';
}
