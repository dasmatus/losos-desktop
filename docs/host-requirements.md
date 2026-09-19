# What the build host must provide

pm mirrors the host's `/usr` into the build jail read-only and resolves every
step's first word *on the host* (C3 in `docs/pm-constraints.md`). So a program a
recipe invokes directly is a host requirement, not something this tree can
supply — there is no bootstrap step that could install it, because installing it
would itself be a step whose first word had to resolve on the host.

`Containerfile` is this list, executable — `./do container check` runs the gate
inside an image that has all of it, pinned, and CI uses the same image. See
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
| **clang, lld, llvm-ar/nm/objcopy/strip** | The whole tree is compiled with them; `manifest/toolchain.yaml` names the exact binaries. | `tools/gates/toolchain-report.py` |
| **python3** | Every meson invocation is `python3 …/meson.py`, because meson is vendored rather than installed. | `./do check` |
| **python3 `jinja2`** | fwupd's meson runs `python3 -c 'import jinja2'` and errors out without it; systemd generates sources with it too. A step cannot set `PYTHONPATH` — no shell, and `env` is banned — so this one cannot be vendored into the sysroot the way meson is. | fails at fwupd's configure step |
| **ninja** | Every meson build. | `./do check` |
| **cargo**, plus `rustup target add <arch>-unknown-linux-musl` | `losos-security` is a cargo build and everything above the toolchain layer is musl; without the target's std, `--target` fails. | fails at `losos-05-core` |
| **pkgconf** or pkg-config, **make**, **tar**, **xz**, **zstd**, **cpio**, **patch** | Named directly by recipes; all are in pm's fingerprint table. | `tools/gates/fingerprint-lint.py` |

## Not required

Not meson (vendored by `losos-01-meson`), not gperf, flex, gettext or bpftool
(built by `losos-15-hosttools` and reached through a native file), not nix, not
KVM, not root, and not network for `./do check`.

## Deliberately a host requirement

The two entries above that could in principle be vendored — jinja2 and the Rust
musl target — are not, and both for the same reason: reaching a vendored copy
needs an environment variable, and a pm step has no way to set one. Writing that
down is better than a half-working vendoring that fails on someone else's
machine with a confusing error.
