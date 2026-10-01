# The two things the pm tree's kernel layer did that nixpkgs leaves to the
# configuration. nixpkgs' common-config.nix already sets what systemd needs
# from the old kernel fragment, cgroups, BPF with BTF, seccomp, PSI and EFI
# among it, and builds the drivers that fragment listed as modules.
{ config, ... }:

{
  # zswap compresses pages in RAM before they reach losos-swap's encrypted
  # partition, which keeps a machine with little memory usable under load.
  # nixpkgs builds it into the kernel and leaves it off. The NixOS module sets
  # it on the command line and in sysfs, with zstd as the compressor.
  boot.zswap.enable = true;

  # NVIDIA's open kernel modules, compiled from source against this kernel.
  # It is the same tarball the pm recipe built, and the kernel half only.
  # NVIDIA's userspace is prebuilt against glibc and cannot load into a musl
  # process, so Mesa's NVK and nouveau drive the GPU instead. The open modules
  # cover Turing and later GPUs only.
  #
  # hardware.nvidia is not used because it installs that proprietary
  # userspace.
  boot.extraModulePackages = [ config.boot.kernelPackages.nvidiaPackages.stable.open ];

  # modeset=1 creates the DRM device mutter's native backend opens. Without it
  # the session falls back to llvmpipe, which looks like a slow desktop rather
  # than a missing driver. fbdev=1 gives the console a framebuffer before the
  # session starts, so a failed boot shows its error instead of a black screen.
  boot.extraModprobeConfig = ''
    options nvidia_drm modeset=1 fbdev=1
  '';
}
