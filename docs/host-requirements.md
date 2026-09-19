# What the build host must provide

pm mirrors the host's `/usr` into the build jail read-only and resolves every
step's first word *on the host* (C3 in `docs/pm-constraints.md`). So a program a
recipe invokes directly is a host requirement, not something this tree can
supply — there is no bootstrap step that could install it, because installing it
would itself be a step whose first word had to resolve on the host.

`Containerfile` is this list, executable — `just container-check` runs the gate
inside an image that has all of it, and CI uses the same image. The image is
not pinned to a set of package versions and deliberately so; see
[`container.md`](container.md). What follows is still the definition; the
Containerfile is one way of satisfying it, not a replacement for knowing what
it asks for.

That makes the list below part of the build definition rather than a
convenience, and it is short by design: everything else a build needs is
compiled here and reached through a meson native file, an explicit `make`
variable, or an absolute path.

## Required

| What | Why | Checked by |
|---|---|---|
| A kernel with **unprivileged user namespaces** | pm's jail is one. On Ubuntu 24.04 and derivatives `kernel.apparmor_restrict_unprivileged_userns=1` denies the `uid_map` write and nothing builds. | `tools/check-digest` says so by name |
| **just** | The repository's entrypoint is the Justfile; the legacy wrapper only forwards to it for compatibility. | `just check` |
| **clang, llvm-ar/nm/objcopy/strip** | The whole tree is compiled with them; `manifest/toolchain.yaml` names the exact binaries, unversioned, so a distribution that installs them under versioned names needs links made. | `tools/gates/toolchain-report.py` |
| **clang's compiler runtime** (Arch `compiler-rt`, Debian `libclang-rt-N-dev`) | Not the one the tree builds for the musl target — the host's own. Without it clang errors out on `-fsanitize=cfi` with a missing ignorelist, so the toolchain report cannot tell whether CFI works from a flag that never compiled. | `tools/gates/toolchain-report.py` |
| **mold** | The linker `manifest/toolchain.yaml` names, reached as `-fuse-ld=mold`, so clang must find `ld.mold` on the `PATH` pm gives a step. | `tools/gates/toolchain-report.py` links its probe with the real `LDFLAGS` |
| **lld** | Still required although mold is the default, and not as a fallback: the kernel and its headers are built with `LLVM=1`, which is the kernel's own switch for the whole LLVM toolchain and takes `ld.lld` with it. The kernel supports `ld.bfd` and `ld.lld`, not mold. | fails at `losos-00-toolchain` |
| **`LLVMgold.so`** | mold does LTO through the GNU linker-plugin interface, so clang hands it `-plugin .../LLVMgold.so`; lld needed none of this because its LTO is built in. Where it lives is a packaging decision and not a fixed path: Arch ships it inside `llvm`, Debian splits it into `llvm-N-linker-tools`, which `llvm-N-dev` does **not** depend on. Every link in the tree is an LTO link, so without it the first one stops with `mold: fatal: could not open plugin file`, naming a path inside clang's own directory. Ask for it the way clang does — `clang -print-file-name=LLVMgold.so`, which prints the name back unchanged when there is nothing there. | `tools/gates/toolchain-report.py` |
| **python3** | Every meson invocation is `python3 …/meson.py`, because meson is vendored rather than installed. | `just check` |
| **python3 `jinja2`** | fwupd's meson runs `python3 -c 'import jinja2'` and errors out without it; systemd generates sources with it too. A step cannot set `PYTHONPATH` — no shell, and `env` is banned — so this one cannot be vendored into the sysroot the way meson is. | fails at fwupd's configure step |
| **ninja** | Every meson build. | `just check` |
| **cargo**, plus `rustup target add <arch>-unknown-linux-musl` | `losos-security` is a cargo build and everything above the toolchain layer is musl; without the target's std, `--target` fails. **A rustup install needs one change:** rustup ships `cargo` as a symlink to `rustup`, and the binary decides which tool it is from the name it was invoked under. pm resolves that symlink (C3), so the recipe reaches the jail as `rustup` and rustup rejects `--release` as an argument of its own. Put links to the real binaries ahead of rustup's bin on `PATH` — `for t in cargo rustc rustdoc; do ln -sf "$(rustup which $t)" /usr/local/bin/$t; done` — and include `rustc`: without the cargo shim nothing sets the toolchain up for cargo's children, so cargo looks `rustc` up on `PATH`, lands back on a shim once per crate, and rustup tries to install a component. `/usr/local/bin` rather than `~/.local/bin`, and that is not a preference: pm gives the step a `PATH` of its own, fixed at `/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin`, so links under `$HOME` satisfy pm's own resolution of `cargo` and are then invisible to the `rustc` cargo goes looking for. A distribution `cargo` package needs none of this. | fails at `losos-05-core` |
| **pkgconf** or pkg-config, **make**, **tar**, **xz**, **zstd**, **cpio**, **patch** | Named directly by recipes; all are in pm's fingerprint table. | `tools/gates/fingerprint-lint.py` |
| **LLVM's cmake package** (Arch `llvm`, Debian `llvm-N-dev`) | Not the headers, the cmake files. compiler-rt configures standalone and calls `find_package(LLVM)`; when that finds nothing it falls back to `CompilerRTMockLLVMCMakeConfig`, which wants `AddLLVM.cmake` from the LLVM *source* tree and hard-errors that `LLVM_CMAKE_DIR` does not exist. The same version as the clang above, since the runtime is version-locked to it. Invisible to `just check` for the same reason as `rsync`. | fails at `losos-00-toolchain` |
| **rsync** | No recipe names it. The kernel's `headers_install` copies the sanitised headers with it, so the first step of the toolchain layer exits 127 without it. Nothing in `just check` can see this, because the gates never run a build. | fails at `losos-00-toolchain` |

## Not required

Not a GNU toolchain of any kind — no gcc, no binutils, no `ld.bfd`; the
only linkers here are mold and lld. Not meson (vendored by `losos-01-meson`),
not gperf, flex, gettext or bpftool
(built by `losos-15-hosttools` and reached through a native file), not nix, not
KVM, not root, and not network for `just check`.

## Deliberately a host requirement

The two entries above that could in principle be vendored — jinja2 and the Rust
musl target — are not, and both for the same reason: reaching a vendored copy
needs an environment variable, and a pm step has no way to set one. Writing that
down is better than a half-working vendoring that fails on someone else's
machine with a confusing error.
