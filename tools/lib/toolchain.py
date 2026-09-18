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
        flags = []
        if triple:
            flags.append(f"--target={triple}")
        if sysroot:
            flags.append(f"--sysroot={sysroot}")
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
        flags = [] if "target" in drops else list(self.target_flags())
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
            "@TRIPLE@": self.target.get("triple", ""),
            "@SYSROOT@": self.target.get("sysroot", "/build/sysroot"),
        }
        # meson native files want a TOML-ish list, not a shell string.
        subs["@MESON_C_ARGS@"] = _ini_list(self.cflags())
        subs["@MESON_LINK_ARGS@"] = _ini_list(self.ldflags())
        return subs


def _ini_list(flags):
    return "[" + ", ".join(f"'{flag}'" for flag in flags) + "]"
