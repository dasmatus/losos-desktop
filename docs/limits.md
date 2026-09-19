# Limits

What this repository does not do, and what it cannot do without a change
somewhere outside it. Written to be read before trusting anything here.

## The rootfs is not self-contained

**This is the defining limitation.** pm mirrors the build host's `/bin`, `/lib`,
`/lib64`, `/sbin` and `/usr` read-only into the jail (C7), and there is no libc
in the recipe set. So everything here links against the *build host's* glibc and
libstdc++, and `losos-rootfs.tar.xz` contains no loader and no libc at all.
Unpacked on a machine without a compatible glibc at the expected sonames,
nothing in it runs.

Fixing it means building glibc or musl as a package and getting every later
package to use it — which needs a real prefix and a sysroot-aware toolchain.
pm cannot express that today, for a specific reason: a newly built compiler
could never be a step's first word, because pm canonicalises that word on the
host (C3). The new toolchain would be reachable only through native-file
injection, per build system, for every package. That is a change to pm, not to
this tree, and it is the honest ceiling of the current design.

## The resource directory is ours; the headers in it are the build host's

This entry used to read "the CFI runtime we build is not the one that gets
linked", and it was the largest honest limit in this file. It is closed, and
the tombstone is worth more than the removal: clang resolves a sanitizer
runtime out of its own **resource directory** — `clang -print-resource-dir`,
then `lib/linux/libclang_rt.<component>-<arch>.a` — and consults neither
`--sysroot` nor `-L` nor `-B` on the way. So every CFI-built package linked the
build host's glibc-compiled copy into a musl binary, and this file predicted
the failure would be silent.

It was not. The first link the tree ever asked for against the musl sysroot
said so outright:

```
ld.lld: error: undefined symbol: dlvsym
>>> referenced by interception_linux.cpp.o in archive
    /usr/lib/llvm-19/lib/clang/19/lib/linux/libclang_rt.cfi-aarch64.a
>>> defined in: /build/sysroot/usr/lib/libc.so
ld.lld: error: undefined symbol: __confstr_chk
ld.lld: error: undefined symbol: __vsyslog_chk
```

`dlvsym` is a GNU extension musl does not have; `__confstr_chk` and
`__vsyslog_chk` are glibc's `_FORTIFY_SOURCE` symbols. The fix is the one this
entry named as the supported way out: `manifest/toolchain.yaml` sets
`target.resource_dir` to `/build/sysroot/usr/lib/losos-clang`, which is
`recipes/00-toolchain/compiler-rt`'s install prefix, and every compile and link
line now carries `-resource-dir=` pointing at it. `libclang_rt.cfi`,
`libclang_rt.builtins` and `clang_rt.crtbegin`/`crtend` are all resolved out of
that directory, as is `share/cfi_ignorelist.txt`, which `-fsanitize=cfi`
refuses to run without.

**What remains is narrower, and it is a version question rather than a libc
one.** A resource directory has to carry clang's builtin headers — `stddef.h`,
`stdarg.h`, `limits.h`, the per-architecture intrinsics — and those belong to
the compiler rather than to compiler-rt, so nothing in this tree builds them.
The compiler-rt recipe copies them out of the build host's clang, asked for
with `clang -print-resource-dir` rather than spelled. That makes the directory
half ours and half the container's, and it is only coherent while the two
agree: `sources.lock` pins compiler-rt at 18.1.8 and the `Containerfile`
installs clang 19. Headers and runtimes one major version apart is a
combination LLVM supports in practice and does not promise, and closing it
means pinning compiler-rt to the container's clang or the other way round.
That choice is open with Matus, along with the same question on the LLVM pin
itself.

## CFI traps instead of diagnosing, because there is no unwinder

`manifest/toolchain.yaml` sets `cfi.trap: true`, and it is marked temporary
there. The distribution wants the other mode: `-fno-sanitize-trap=cfi` makes a
violation print the call site it happened at, which systemd-coredump can then
record, instead of raising SIGILL with nothing attached.

That mode is unavailable, not unchosen. `-fno-sanitize-trap` puts
`libclang_rt.cfi_diag` on the link line; cfi_diag walks the stack to find the
call site; the walk calls `_Unwind_Backtrace` and `_Unwind_GetIP`:

```
ld.lld: error: undefined symbol: _Unwind_Backtrace
>>> referenced by sanitizer_unwind_linux_libcdep.cpp.o
    in archive .../libclang_rt.cfi_diag-x86_64.a
```

musl has no unwinder, `libgcc_s` is not in this sysroot, and LLVM's libunwind
is not built here. `libclang_rt.cfi`, which trap mode links instead, carries no
`_Unwind` reference at all.

**What is lost is the call site, and nothing else.** CFI is still enforced —
every scheme, cross-DSO included — and a violation still stops the process. It
stops with SIGILL, so the report says that a program died rather than which
indirect call was wrong.

The way back is an unwinder for the musl target in the sysroot, and then
`target.unwindlib: libunwind`. `-lunwind` is an ordinary library name and
resolves through `--sysroot`, so unlike the resource-directory problem above it
needs no change to how clang is pointed at anything. It is not one recipe,
though, and the reason is worth writing down because the first attempt looks
like it worked:

- `libunwind/CMakeLists.txt` in 18.1.8 has no `project()` and no
  `enable_language(ASM)` of its own — it is meant to be added as a
  subdirectory, not configured. Configured directly it configures, builds and
  installs without a warning, **silently dropping** `UnwindRegistersSave.S` and
  `UnwindRegistersRestore.S`. The `libunwind.a` that comes out has no
  `__unw_getcontext` in it, and the only symptom is the next link.
- The supported entry point is `runtimes/` with
  `-DLLVM_ENABLE_RUNTIMES=libunwind`. That needs `GetHostTriple.cmake`, which
  ships in neither `llvm-N-dev` nor `cmake-N.src.tar.xz` — only in the full
  LLVM source tree. `AddLLVM.cmake` and `HandleLLVMOptions.cmake` do come from
  `llvm-N-dev`, so it is that one module that would force the pin.

So restoring diagnose mode costs a new pinned source (the LLVM tarball, for one
cmake module), a patch to `runtimes/CMakeLists.txt`, or a libunwind build that
does not go through cmake. That is a pin this tree would carry for years, which
is why it is a decision written down here rather than one taken in passing.

## Cross-DSO CFI is claimed, and a version script can quietly revoke it

`manifest/toolchain.yaml` sets `cfi.cross_dso: true`, which is what makes an
indirect call checked across a shared-library boundary rather than only within
one. The mechanism is that a caller's check falls through to
`__cfi_slowpath`, the runtime finds which DSO the target address lives in, and
it calls **that DSO's own `__cfi_check`** -- which it locates by walking the
library's dynamic symbol table by name.

So `__cfi_check` has to be in `.dynsym`, and in zlib it is not:

```
$ llvm-nm libz.so.1.3.1 | grep __cfi_check
000000000000b000 t __cfi_check          # local, not exported
$ llvm-nm --dynamic libz.so.1.3.1 | grep -i cfi
                 U __cfi_slowpath       # it calls out, nothing can call in
```

The cause is zlib's own version script. `zlib.map` ends in `local: *;`, which
localises every symbol the script does not name, and `__cfi_check` is not a
symbol upstream knows to name. Measured: adding
`-Wl,--export-dynamic-symbol=__cfi_check` does not override it, because the
version script wins.

This is not zlib-specific and it is not fixed by the visibility patch in
`recipes/00-base/zlib`, which is about zlib's own API. Every library in this
tree that ships a version script ending in `local: *` has the same hole, and
that includes libsystemd, glib and mesa. A cross-DSO indirect call into such a
library reaches a DSO the runtime cannot check; in the trap mode
`manifest/toolchain.yaml` currently sets, the honest reading is that it stops
the process rather than silently passing, but that has not been observed here
because nothing in this tree has run yet.

**This is open and it is a design decision, not a patch.** The options are a
per-library version-script patch (which does not scale and has to be redone at
every version bump), dropping version scripts across the tree (which throws
away symbol versioning, an ABI regression), or accepting that cross-DSO CFI
covers only the libraries without one and saying so in
`manifest/toolchain.yaml` instead of claiming the whole set. Nothing here
picks one.

## pm has no package store

Dependencies are carried, not consumed (C5): a dependency's archive is copied
into its dependent and nothing unpacks it. `share/sysroot.sh` is this
repository's workaround — recursive untar plus a pkg-config prefix rewrite — and
it lives here rather than in pm because that is what it is. Anything it gets
wrong (a `.la` file, a cmake package file, an RPATH) surfaces as a link against
the wrong library version with no warning. `leak-audit.sh` catches the subset
that leaves a `/build` string behind; it cannot catch a silent link against the
host's copy of a library we also built.

## pm has no build cache

`pm build` rebuilds every node in the graph, every time (C6). That is why the
build graph is seven layer bundles and not ninety package nodes — but it still
means changing one GNOME package rebuilds that entire layer. There is no
incremental path, and adding one means adding content addressing to pm.

**CI cannot work around this, and does not pretend to.** The workflows cache
everything that is an *input* to a build — the pm binary, pm's component
encoder, this repository's plugin components, and the mirror of the
hundred-odd pinned tarballs — so a run that changes one recipe no longer
recompiles pm twice and re-downloads several gigabytes from two dozen
upstreams before it starts. None of that touches the hours. Those are
`pm build` compiling the layer chain, and it compiles all of it because a
dependency in pm is a *build file*, loaded and rebuilt: `Graph::visit_dependency`
rejects a path that is not one. A layer's `.cpkg` from an earlier run is not
something pm can be handed.

Reusing a built layer would therefore mean one of two things, and both are
larger than a cache: teaching pm to accept a built archive where a build file
goes, or generating a stand-in build file per cached layer that unpacks the
archive into `/dest`. The second stays inside this repository and is the one to
be careful with — its key has to cover every input to that layer and everything
below it in the chain, and a key that is subtly wrong ships an image built from
stale binaries with CI green over it.

## Archive duplication

Each layer's archive contains the one below it, whole. With seven layers that is
roughly a sixfold multiplication of intermediate storage, and with a full GNOME
stack the intermediates plausibly run to tens of gigabytes before the final
tarball. `TMPDIR` must be on real disk, not tmpfs.

The image layer adds to that: it holds the OS tree, the root filesystem as a
partition image, a raw disk built around it, the qcow2 and the ISO in the same
workspace at once. The disk images are sparse and the qcow2 leaves zero
clusters unallocated, so the cost is close to the content rather than to the
declared sizes — but it is still several copies of the tree.

## The GNOME layer is the long tail

`recipes/30-gnome` is the minimum that produces a session, and it is a
minimum in the optimistic sense. `gobject-introspection` has to *run* the
libraries it scans. `gjs` carries SpiderMonkey. `mutter` needs a working GL
stack at build time, not just at run time. `gnome-control-center` reaches for
NetworkManager, ModemManager, fwupd, cups, ibus and more; `nautilus` wants
tracker3 and gvfs; the portal wants gnome-remote-desktop for screencast. A first
honest count after real build attempts should be expected to land well above the
27 packages listed, and several panels of the control centre will be inert
because this OS runs `systemd-networkd` rather than NetworkManager. That trade
is deliberate — one network stack, already a systemd unit — but it is a trade.

## Nothing here has booted

There is no VM in the development environment: no KVM, no EFI firmware, no loop
devices. The UKI is checked structurally — it is a PE, it carries the six
sections systemd-stub looks for, and `tools/gates/test-image.py` verifies the
writer's output against an independent parser — and that is the entire claim.
`systemd-repart`, `systemd-firstboot`, homed, verity, sysupdate and gdm are all
wired and none has been exercised. **Anyone who says this OS boots should say on
what machine, once.**

The installation media are subject to the same sentence, and the claim about
them is now weaker rather than stronger. They are built by mkosi, xorriso and
qemu-img, which are mature and are not this repository's to get wrong — but
nothing here has run them: the environment this was developed in has none of
them installed, which is the whole reason `Containerfile` exists. What runs
inside the build is `assert-media.py`, which reads the finished files back and
asserts the qcow2 is a qcow2 with an ESP and a discoverable root in it and that
the ISO's boot catalog and its ESP partition entry point at the same bytes.
`tools/gates/test-media.py` is what makes that claim worth anything: it feeds
`assert-media.py` the shape the image layer produces and nine near-misses, and
requires it to reject all nine. Without it the assertion could be vacuous and
would report so as success.

That proves the structures are the structures they claim to be. It proves
nothing about firmware, and **nobody has put this ISO in front of any.**

`tools/vm-test disk` is the test that would settle it: it hands the qcow2 to
QEMU with UEFI firmware and requires the guest to say it came up, and CI runs
it. It needs KVM and OVMF, which is exactly what this environment does not
have.

## The upstream sources have not been fetched

`manifest/sources.lock` ships with `sha256: TODO` for every entry except
`MESON`, because the environment this was developed in cannot reach
kernel.org, freedesktop.org, gnome.org or ftp.gnu.org. `tools/fetch-sources
--update` fills them in on a machine that can. Nothing here invents a hash: pm
verifies before it parses, so a wrong value presents as a compromised mirror,
which is strictly worse than a missing one.

Consequently **no upstream package in this tree has been compiled**. What has
been built end to end, by pm, in the jail: `losos-00-hosttools` (meson, from a
pinned sdist) and `losos-05-core` (`losos-release`, compiled from this
repository's own C by that meson, and run out of its own archive). The recipe
tree above that is validated by `just check` — schema, URL form, fingerprints,
and `pm explain` over every command — which proves it is executable-in-principle
and proves nothing about whether each package configures.

## Two named fingerprint compromises

pm's fingerprint check reads the first word only (C2), so a wrapper hides what
it wraps.

- **`share/in-dir.sh`** is the one wrapper this repo permits, because
  `./configure` has no `-C` and there is no `cd`. It is listed in
  `tools/gates/allowed-wrappers`, and `fingerprint-lint.py` re-applies pm's own
  table to whatever follows it. That lint has already caught one real case.
- **`env` is banned outright**, not merely discouraged — it hides the program
  *and* silently rewrites the derived capability set.

Something genuinely unclassifiable — `veritysetup`, say — would need
`pm build --permissive`, and should be its own recipe saying so in its header
rather than smuggled through a wrapper. That is also why the disk images are
written by format writers in Python rather than by e2fsprogs, mtools and
libisoburn built into the sysroot and driven from a shell script: the script
would have been a third wrapper, hiding four more programs. See
[`images.md`](images.md).

## The download-path derivation is undocumented behaviour

`tools/configure` computes where pm will put a download by reimplementing an
FNV-1a-64 hash from pm's `Step::url_digest`. Nothing in pm promises that layout
and no test in pm pins it. If it changes, every recipe fails with `ENOENT` on a
file that was downloaded and verified moments earlier. `tools/check-digest`
exists to turn that into one clear message, and it proves the agreement against
a real `pm build` rather than against itself — but it is a guard on a
dependency that was never promised.

## NVIDIA: kernel module yes, proprietary userspace no

`recipes/10-systemd/nvidia-open` builds NVIDIA's open kernel modules from
source, against this kernel tree, so they pick up the same `CONFIG_CFI_CLANG`
and LTO settings the kernel was built with. An out-of-tree module whose CFI
settings disagree with the kernel's does not load, and reports only that the
module format is invalid.

The userspace is NVK and nouveau in Mesa. The proprietary userspace is absent
and will stay absent, for a reason that is structural rather than political:
`libcuda` and `libnvidia-glcore` are prebuilt blobs linked against **glibc**.
Our toolchain never compiles them, so they receive neither CFI nor LTO, and
they cannot load into a musl process at all — two C libraries cannot coexist in
one address space. Shipping them would mean carrying a second libc inside a
bundle and running every accelerated application inside that bundle, which is a
parallel userspace maintained for one vendor.

What this costs: **CUDA is not available**, and NVK is behind the proprietary
driver on raw performance. What it buys: one libc, one toolchain, and no
component of the graphics stack outside the CFI scheme.

Open kernel modules cover Turing and later. Nothing older has one, and this
tree has nothing to offer those cards beyond nouveau's reverse-engineered
support.
