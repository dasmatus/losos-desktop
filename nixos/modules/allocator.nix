# GrapheneOS's hardened_malloc as the allocator of every process on a PC.
#
# nixpkgs' glibc reads /etc/ld-nix.so.preload where upstream glibc reads
# /etc/ld.so.preload, and loads what it names into every dynamically linked
# process ahead of libc, so malloc and free in all of them, C, C++ and Rust,
# are hardened_malloc's: slab canaries, zero-on-free, guard slabs, randomized
# slot reuse and quarantines, with metadata kept away from the heap it
# describes. Statically linked programs keep their own allocator.
#
# derisk is built with mcsapi-hardened-malloc as its Rust global allocator,
# which links the same libhardened_malloc.so and frees through free_sized, so
# a free with a corrupted length aborts too.
#
# PCs only: nixos/halium/ does not import this. hardened_malloc reserves 32
# GiB per size class per arena up front and needs a 48-bit address space,
# and most Android kernels are built with 39-bit virtual addresses, where its
# first allocation fails and with it every process.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  lib' = "${pkgs.graphene-hardened-malloc}/lib/libhardened_malloc.so";
in
{
  # Not environment.memoryAllocator.provider = "graphene-hardened": that
  # preloads a copy of the library in a store path of its own. The library
  # has no DT_SONAME, so ld.so tells loaded objects apart by file, and derisk,
  # linking the package's copy, would map and initialize a second
  # hardened_malloc beside the preloaded one. Naming the package's own file
  # here makes the two one mapping.
  environment.etc."ld-nix.so.preload".text = ''
    ${lib'}
  '';

  # After base.nix's overlay, which defines derisk.
  nixpkgs.overlays = lib.mkAfter [
    (final: prev: {
      derisk = prev.derisk.override { withHardenedMalloc = true; };
    })
  ];

  # hardened_malloc puts a guard mapping between slabs, and a process with a
  # large heap needs far more mappings than the kernel's default 65530.
  # NixOS already raises vm.max_map_count to 1048576, upstream's
  # recommendation; this fails the build if that default ever goes away.
  assertions = [
    {
      assertion = (config.boot.kernel.sysctl."vm.max_map_count" or 0) >= 1048576;
      message = "hardened_malloc needs vm.max_map_count of at least 1048576.";
    }
  ];
}
