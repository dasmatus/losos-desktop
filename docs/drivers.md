# Drivers and NVIDIA

The image is one build for every PC, so it carries every driver nixpkgs'
kernel builds and all of linux-firmware. For nearly all hardware that is the
whole story: the kernel names a module for each device it finds, udev loads
it, and a driver for hardware the machine lacks is never loaded. What udev
cannot do is choose between two drivers that both claim a device. NVIDIA's
GPUs are that case: nouveau and NVIDIA's own open kernel module both claim
every NVIDIA display controller, and which of them should drive a card
depends on which card it is.

## Choosing at boot from facter's report

NixOS has an answer for per-machine hardware, `hardware.facter`, which reads a
[nixos-facter](https://github.com/nix-community/nixos-facter) report while the
configuration is evaluated. That needs a build per machine, and this OS is a
signed image every machine shares, with no Nix on it to build anything. So
the image reads the same report on the machine, at boot, and acts on it with
files kmod and systemd already read from `/run`:

1. `losos-hardware.service` runs early in every boot, before udev's coldplug
   and `systemd-modules-load`. It runs `nixos-facter --hardware pci,usb,cpu`,
   which reads sysfs and takes well under a second, into
   `/run/losos/hardware/facter.json`.
2. `losos-hardware plan` (`src/losos-hardware`) matches the report against
   the rules the configuration defines in `losos.hardware.rules`
   (`nixos/modules/hardware.nix`, readable at `/etc/losos/hardware-rules.json`).
   A rule names a facter class, a vendor and optionally a list of device IDs,
   and what to do when a device matches: modules to load, modules to keep off
   the device, a fallback, and flags.
3. For every match it writes `blacklist` lines to
   `/run/modprobe.d/losos-hardware.conf`, so coldplug does not load a displaced
   driver; the wanted modules to `/run/modules-load.d/losos-hardware.conf`,
   which `systemd-modules-load` loads by name, past any blacklist; a file per
   flag under `/run/losos/hardware/flags/`, for units to test with
   `ConditionPathExists=`; and `/run/losos/hardware/plan.json`, saying what
   matched which device.
4. `losos-hardware-fallback.service` runs after `systemd-modules-load`. If a
   rule's module did not load, it loads the rule's fallback by name, so a
   driver that refuses a card leaves the machine with the driver it would
   have had without the rule rather than none.

Every boot rather than once at install: a card swapped in gets its driver on
the first boot it is there, and nothing has to keep a stored report in step
with the hardware. A rule is the place for any other device two drivers
compete for, or that needs a service only on the machines that have it; most
hardware needs none.

## NVIDIA

`nixos/modules/nvidia.nix` puts NVIDIA's current production driver in the
image: the open kernel module, its GSP firmware and the userspace libraries,
less OpenCL, OptiX, the CUDA debugger, Vulkan SC, the Wine DLLs and the Xorg
GLX module, which nothing in the session loads (about 210 of 784 MB). It
does what NixOS's `hardware.nvidia` module does except the two things that
assume the machine: that module blacklists nouveau and loads `nvidia_uvm` at
every boot. Here the NVIDIA modules are blacklisted instead, so udev never
loads them uninvited, and nouveau keeps every card by default.

The rule `nvidia-open` matches a `graphics_card` from vendor `0x10de` whose
device ID is in the table of supported GPUs NVIDIA ships inside its
installer, cut at build time to the chips the current branch supports. That
branch supports Turing (GTX 16xx, RTX 20xx) and everything newer, all of it
on the open module. On a match, nouveau is blacklisted, `nvidia`,
`nvidia_modeset`, `nvidia_drm` and `nvidia_uvm` are loaded, and nouveau is
the fallback.

| Card | Driver | Vulkan |
| --- | --- | --- |
| Turing and newer (GTX 16xx, RTX 20xx and later) | NVIDIA's open module | NVIDIA's |
| Maxwell, Pascal, Volta (GTX 7xx/9xx/10xx, Titan V) | nouveau | Mesa's NVK |
| Older | nouveau | none |
| A card newer than the image's driver | nouveau | Mesa's NVK |

Maxwell to Volta are in NVIDIA's 580 legacy branch and are not offered its
closed module. In a laptop with an Intel or AMD GPU beside the NVIDIA one,
the rule matches only the NVIDIA card; the other keeps its driver and stays
the boot display, which derisk's compositor draws on, and the NVIDIA GPU
powers down while unused (the udev rule in `nvidia.nix` lets the kernel
suspend it).

`nvidia_drm` runs with `modeset=1 fbdev=1`, which the compositor's GBM
scanout needs. Suspend and hibernation use the driver's kernel suspend
notifiers, with video memory saved to `/var/tmp`. VA-API decodes on the card
through `nvidia-vaapi-driver`. Flatpak apps do not use these libraries: they
take Flathub's NVIDIA GL extension of the same driver version.

## The compositor on NVIDIA

derisk's compositor (mcsapi's KMS backend) opens the seat's boot GPU, makes a
GBM device on it, and renders with EGL on that device; on NVIDIA, Mesa's
`libgbm` loads NVIDIA's backend (`/run/opengl-driver/lib/gbm/nvidia-drm_gbm.so`)
and NVIDIA's EGL takes the GBM platform through `egl-gbm`. Frames are queued
for scanout with the fence of their rendering, so the display never flips to a
buffer NVIDIA has not finished drawing. Clients share GPU buffers through
`zwp_linux_dmabuf_v1`, and `wp_linux_drm_syncobj_v1` (explicit sync) is
offered when the GPU's DRM driver supports syncobj eventfds, which NVIDIA's
does from 560 on: NVIDIA does not do implicit sync, so without it a client's
frame could be shown before NVIDIA finished drawing it.

## What is not tested

There is no NVIDIA card where this was built. `nixos/tests/hardware.nix`
boots a VM and checks the probe runs before coldplug and module loading, that
NVIDIA's modules are installed and that udev would load nouveau, not them,
for an RTX 4090's ID; then it hands `losos-hardware` a report with that card
and checks nouveau is blacklisted, NVIDIA's module is loaded and, finding no
GPU, refuses, and the fallback loads nouveau. A real card accepting the
module, the compositor on it, suspend and hibernation, VA-API and
hardened_malloc under NVIDIA's libraries are untested ([What is not
done](not-done.md)).
