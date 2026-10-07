# The memory allocator

On a PC every dynamically linked process allocates through GrapheneOS's
[hardened_malloc](https://github.com/GrapheneOS/hardened_malloc), nixpkgs'
`graphene-hardened-malloc` in its default (not light) configuration.
`nixos/modules/allocator.nix` names it in `/etc/ld-nix.so.preload`, which
nixpkgs' glibc reads in place of `/etc/ld.so.preload`. derisk is built with
its `hardened-malloc` feature, mcsapi's `mcsapi-hardened-malloc` crate as its
Rust global allocator: the same `libhardened_malloc.so`, mapped once, with
Rust's frees going through `free_sized` so a wrong length aborts.

The module writes the preload itself rather than setting NixOS's
`environment.memoryAllocator.provider`, which preloads a copy of the library
from another store path. The library has no `DT_SONAME`, so `ld.so` would map
that copy and derisk's as two allocators. Halium does not import the module:
see [What is not done](not-done.md).

[Danube](danube.md)'s WebKit keeps its own allocator, libpas, for
JavaScript and DOM objects, with its Gigacage, and allocates everything
else through `malloc`, so it runs under the preload like any other
program. Uranium, the Chromium build before it, could not (PartitionAlloc
replaces `malloc` itself) and ran without the preload; it is gone.
