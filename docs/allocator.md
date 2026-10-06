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

Uranium is the one exception. Chromium's own allocator, PartitionAlloc, is
its malloc, and under the preload the two free each other's memory and the
browser aborts at start ("fatal allocator error: invalid uninitialized
allocator usage"). glibc has no way to skip the preload file for one program,
so Uranium's launcher runs Chromium in an unprivileged bubblewrap mount
namespace where `/etc/ld-nix.so.preload` is empty. Chromium's own sandbox
still works inside it. PartitionAlloc is hardened in its own right, which is
why GrapheneOS's own browser keeps it too.
