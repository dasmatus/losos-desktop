# What the pm tree's kernel layer did that nixpkgs leaves to the
# configuration. nixpkgs' common-config.nix already sets what systemd needs
# from the old kernel fragment, cgroups, BPF with BTF, seccomp, PSI and EFI
# among it, and builds the drivers that fragment listed as modules.
{ ... }:

{
  # zswap compresses pages in RAM before they reach losos-swap's encrypted
  # partition, which keeps a machine with little memory usable under load.
  # nixpkgs builds it into the kernel and leaves it off. The NixOS module sets
  # it on the command line and in sysfs, with zstd as the compressor.
  boot.zswap.enable = true;

  # GPU firmware from linux-firmware. amdgpu needs it to start any AMD GPU,
  # and nouveau needs NVIDIA's GSP firmware for Turing and newer. Without it
  # both fall back to llvmpipe. These are prebuilt vendor blobs, because no
  # source for them exists (docs/nixos.md, "What this trusts from outside").
  hardware.enableRedistributableFirmware = true;

  # No NVIDIA kernel module. nvidia.ko, open or not, serves NVIDIA's own
  # userspace, which is prebuilt against glibc and cannot load into a musl
  # process. Mesa's NVK drives NVIDIA GPUs through the in-tree nouveau driver
  # instead. Shipping nvidia.ko as well would let it claim the GPU first and
  # leave GNOME with no driver it can use. The pm tree built the open modules
  # anyway, which was the same mistake.
}
