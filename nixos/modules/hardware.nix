# What the pm tree's kernel layer added that nixpkgs' kernel does not already
# do. nixpkgs' common-config.nix sets what systemd needs from the old kernel
# fragment (cgroups, BPF with BTF, seccomp, PSI, EFI) and builds the drivers it
# listed as modules; these two are what it leaves to the configuration.
{ config, ... }:

{
  # zswap. nixpkgs builds it in with zstd as its compressor but leaves it off,
  # and the pm kernel turned it on by default. A RAM-compressed cache in front
  # of losos-swap's encrypted partition is what keeps a desktop with little
  # memory usable; turning it on here instead of in Kconfig costs one word.
  boot.kernelParams = [ "zswap.enabled=1" ];

  # NVIDIA's open kernel modules, compiled from source against this kernel.
  # This is nixpkgs' derivation of the same tarball the pm recipe built, and
  # like that recipe it is the kernel half only: no proprietary userspace,
  # which is prebuilt against glibc and could not load into a musl process
  # anyway. Mesa's NVK and nouveau drive the GPU from userspace. Turing and
  # later only; nothing older has an open module.
  #
  # hardware.nvidia is deliberately not used: it pulls the proprietary driver
  # package, the thing this OS leaves out.
  boot.extraModulePackages = [ config.boot.kernelPackages.nvidiaPackages.stable.open ];

  # Without modeset=1 there is no DRM device for mutter's native backend to
  # open, and the session falls back to llvmpipe -- a failure that looks like
  # "the desktop is slow" rather than a driver that did not load. fbdev=1 gives
  # the console a framebuffer before the session starts, so a boot failure is
  # visible rather than a black screen.
  boot.extraModprobeConfig = ''
    options nvidia_drm modeset=1 fbdev=1
  '';
}
