"""Turn manifest/toolchain.yaml into the placeholders recipes use.

Every flag in the tree comes from here. That is the point: CFI is worth very
little if it is applied to most of a system, because an indirect call is only
checked when the *caller* was built with it and a cross-DSO call only when both
sides were. Letting a recipe spell its own -O2 is how a package quietly ends up
outside the scheme with nothing reporting it.

`tools/gates/toolchain-report.py` re-derives these from the same file and
checks that no recipe has gone its own way.
"""

from pathlib import Path

import yaml


class Toolchain:
    def __init__(self, doc):
        self.doc = doc or {}
        self.compiler = self.doc.get("compiler", {})
        self.target = self.doc.get("target", {})
        self.lto = self.doc.get("lto", {})
        self.cfi = self.doc.get("cfi", {})
        self.hardening = self.doc.get("hardening", {})
        self.rust = self.doc.get("rust", {})
        self.exceptions = {
            entry["package"]: entry for entry in (self.doc.get("exceptions") or [])
        }

    @classmethod
    def load(cls, path):
        if not Path(path).exists():
            return cls({})
        return cls(yaml.safe_load(Path(path).read_text()))

    # -- the pieces -------------------------------------------------------

    def lto_flags(self):
        mode = self.lto.get("mode")
        return [f"-flto={mode}"] if mode else []

    def cfi_flags(self):
        """The CFI flag set, or nothing if it is switched off.

        Order matters to a reader, not to clang: visibility first because it is
        the precondition, then the schemes, then how a violation is reported.
        """
        if not self.cfi.get("enable"):
            return []
        flags = [f"-fvisibility={self.cfi.get('visibility', 'hidden')}"]
        for scheme in self.cfi.get("schemes") or []:
            flags.append(f"-fsanitize={scheme}")
        if self.cfi.get("cross_dso"):
            flags.append("-fsanitize-cfi-cross-dso")
        if not self.cfi.get("trap"):
            # Diagnose: the violation names its call site instead of raising
            # SIGILL with nothing attached.
            flags.append("-fno-sanitize-trap=cfi")
            flags.append("-fsanitize-recover=cfi")
        return flags

    def target_flags(self):
        triple = self.target.get("triple")
        sysroot = self.target.get("sysroot")
        resource_dir = self.target.get("resource_dir")
        flags = []
        if triple:
            flags.append(f"--target={triple}")
        if sysroot:
            flags.append(f"--sysroot={sysroot}")
        if resource_dir:
            # Compile and link both: the resource directory is where clang's
            # builtin headers come from as well as its runtimes, so dropping it
            # from the compile line loses stddef.h.
            flags.append(f"-resource-dir={resource_dir}")
        return flags

    def runtime_flags(self, drops=()):
        """Which runtime library and unwinder clang links against.

        Link-time only, deliberately. They are accepted on a compile line and
        do nothing there, and clang then warns about an unused argument for
        every translation unit in the distribution.

        `unwindlib` is droppable on its own, which no other flag here is. The
        unwinder cannot be handed itself: libunwind's own shared library links
        with --rtlib=compiler-rt like everything else, and a --unwindlib
        naming the library being linked resolves to nothing.
        """
        flags = []
        rtlib = self.target.get("rtlib")
        if rtlib:
            flags.append(f"--rtlib={rtlib}")
        unwindlib = self.target.get("unwindlib")
        if unwindlib and "unwindlib" not in drops:
            flags.append(f"--unwindlib={unwindlib}")
        return flags

    def cflags(self, package=None):
        """The compile flags, minus whatever `package` is exempt from."""
        drops = self._drops(package)
        flags = []
        if "target" not in drops:
            flags += self.target_flags()
        if "hardening" not in drops:
            flags += list(self.hardening.get("cflags") or [])
        if "lto" not in drops:
            flags += self.lto_flags()
        flags += self._cfi_for(drops)
        return flags

    def _drops(self, package):
        entry = self.exceptions.get(package) if package else None
        return set(entry.get("drops") or []) if entry else set()

    def _cfi_for(self, drops):
        """CFI flags after removing whole-feature and per-scheme exemptions."""
        if "cfi" in drops:
            # Dropping CFI entirely still leaves -fvisibility=hidden off: it is
            # only here as CFI's precondition, and forcing it on a libc changes
            # which symbols are exported.
            return []
        if not self.cfi.get("enable"):
            return []
        kept = [s for s in (self.cfi.get("schemes") or []) if s not in drops]
        if not kept:
            return []
        flags = [f"-fvisibility={self.cfi.get('visibility', 'hidden')}"]
        flags += [f"-fsanitize={scheme}" for scheme in kept]
        if self.cfi.get("cross_dso"):
            flags.append("-fsanitize-cfi-cross-dso")
        if not self.cfi.get("trap"):
            flags.append("-fno-sanitize-trap=cfi")
            flags.append("-fsanitize-recover=cfi")
        return flags

    def ldflags(self, package=None):
        drops = self._drops(package)
        linker = self.compiler.get("linker")
        # The runtime flags ride with `target` rather than with `hardening`:
        # they name what the target's libc is built alongside, and the two
        # packages that drop `target` -- musl and the kernel -- are exactly the
        # two that must not be handed compiler-rt. musl is building the sysroot
        # these would resolve out of, and the kernel supplies its own.
        flags = (
            [] if "target" in drops
            else self.target_flags() + self.runtime_flags(drops)
        )
        if linker:
            # CFI needs a linker that understands the bitcode it is merging.
            flags.append(f"-fuse-ld={linker}")
        if "lto" not in drops:
            flags += self.lto_flags()
        flags += self._cfi_for(drops)
        if "hardening" not in drops:
            flags += list(self.hardening.get("ldflags") or [])
        return flags

    def rustflags(self, nightly=False):
        flags = list(self.rust.get("flags") or [])
        if nightly:
            flags += list(self.rust.get("nightly_flags") or [])
        return flags

    # -- what the generator consumes --------------------------------------

    def resource_dir_prefix(self):
        """`target.resource_dir` as a path inside the sysroot rather than under it.

        The manifest spells the resource directory the way clang is handed it,
        which is absolute and already inside the jail's sysroot. A recipe that
        installs into it needs the same location twice -- staged at /dest<tail>
        and live at /build/sysroot<tail> -- so what a recipe can use is the
        tail. Derived here rather than spelled a second time in
        tools/configure: a copy of a manifest value that nothing compares back
        is a copy that drifts.
        """
        resource_dir = self.target.get("resource_dir", "")
        sysroot = self.target.get("sysroot", "/build/sysroot")
        if resource_dir.startswith(sysroot):
            return resource_dir[len(sysroot):]
        return resource_dir

    def placeholders(self, resolve):
        """The @TOKEN@ map. `resolve` turns a program name into a host path."""
        compiler = self.compiler
        subs = {
            "@CC@": resolve(compiler.get("cc", "cc"), "cc"),
            "@CXX@": resolve(compiler.get("cxx", "c++"), "c++"),
            "@AR@": resolve(compiler.get("ar", "ar"), "ar"),
            "@NM@": resolve(compiler.get("nm", "nm"), "nm"),
            "@RANLIB@": resolve(compiler.get("ranlib", "ranlib"), "ranlib"),
            "@STRIP@": resolve(compiler.get("strip", "strip"), "strip"),
            "@OBJCOPY@": resolve(compiler.get("objcopy", "objcopy"), "objcopy"),
            "@CFLAGS@": " ".join(self.cflags()),
            "@LDFLAGS@": " ".join(self.ldflags()),
            "@RUSTFLAGS@": " ".join(self.rustflags()),
            # The same flags as a TOML array, for `cargo --config
            # build.rustflags=...`. cargo's own way of taking them is the
            # RUSTFLAGS environment variable, and a build step cannot set one:
            # `env` is banned here because pm reads only a command's first word
            # (C3), so `env RUSTFLAGS=... cargo build` is classified coreutils
            # and loses cargo's network grant with it. Quoted and comma-joined
            # with no spaces, because a step's command line is whitespace-split
            # and execve'd -- a space would make this several arguments.
            "@RUSTFLAGS_CARGO@": _toml_list(self.rustflags()),
            "@TRIPLE@": self.target.get("triple", ""),
            "@SYSROOT@": self.target.get("sysroot", "/build/sysroot"),
            "@RESOURCE_DIR_PREFIX@": self.resource_dir_prefix(),
        }
        # meson native files want a TOML-ish list, not a shell string.
        subs["@MESON_C_ARGS@"] = _ini_list(self.cflags())
        subs["@MESON_LINK_ARGS@"] = _ini_list(self.ldflags())
        return subs


def _ini_list(flags):
    return "[" + ", ".join(f"'{flag}'" for flag in flags) + "]"


def _toml_list(flags):
    """A TOML array with no whitespace anywhere in it."""
    return "[" + ",".join(f'"{flag}"' for flag in flags) + "]"
