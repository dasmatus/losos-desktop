#!/usr/bin/env python3
"""Every configure must say it is cross-compiling, because it is.

The build host runs glibc and everything above losos-00-toolchain is compiled
against musl. Even when the architectures match, that is a cross build: the
test programs autoconf compiles are musl-dynamic, and their interpreter
(/lib/ld-musl-<arch>.so.1) does not exist in pm's jail, which mirrors the
HOST's /lib read-only (C7). So autoconf's very first act fails:

    configure: error: in '/build/b/xz':
    configure: error: cannot run C compiled programs.
    If you meant to cross compile, use '--host'.

--host=<triple> is what makes autoconf stop trying to run what it builds and
take the compile-only answer instead. Nothing else in the recipe implies it:
--target= on the compile line tells clang what to emit and tells configure
nothing at all, which is why thirty-six recipes were written without it and
all thirty-six would have failed in turn, one build at a time, each looking
like a fresh problem with a different package.

EXEMPT below is the other answer to the same question: a package that is not
cross-compiled at all, and for which --host would be a false statement rather
than a missing one. It is a list of names, not a rule, because every entry on
it had to be argued for.

meson spells the same statement --cross-file, and gets it wrong the same way.
A native build's first act is to compile a test program and RUN it, and meson
skips that run only when a cross file's [host_machine] section has told it the
build is cross -- `is_cross and not has_exe_wrapper()`. So a `meson setup` with
only --native-file dies on its first line with

    Build type: native build
    ERROR: Could not invoke sanity check executable: [Errno 2] No such file
    or directory: '.../meson-private/sanity_check_for_c.exe'

naming the test binary rather than the loader that is actually missing. dbus
found that, being the first meson package this tree ever compiled; the other
forty-seven would have found it one build at a time. share/cross.ini.in is the
file, and a recipe reaches it as @RECIPE@/cross.ini or, for a package with a
toolchain exception, @CROSS_INI_<PKG>@.

There is no meson EXEMPT list and there should not be one: a package that is
not cross-compiled here is a `drops: [target]` host tool, and all three of
those are autotools.

This is a template gate rather than a generated-file one on purpose: the
answer has to be in the recipe a human edits.
"""

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent

# A step that runs a source tree's `configure`. The leading slash keeps this
# off tools/configure, which recipes mention in comments; the end-of-line
# alternative catches a step whose configure takes no arguments at all, which
# is the one shape a lookahead for whitespace alone would wave through.
CONFIGURE = re.compile(r"/configure(?=\s|$)")

# A step that runs `meson setup`. The recipe spells it `python3 <meson.py>
# setup` (C3 -- meson is not on the build hosts, and a step's first word is
# resolved there), so the pair is what identifies it rather than the word
# `meson` alone, which every one of these recipes also says in a comment.
MESON_SETUP = re.compile(r"meson\.py\s+setup(?=\s|$)")

# Tombstone: this gate was briefly rewritten to yaml.safe_load the template and
# walk steps[].run instead of scanning lines, which is the tidier shape and
# cannot work. A build.yaml.in is not YAML -- `@URL_ACL@: '@SHA256_ACL@'` opens
# with a character YAML reserves, and safe_load raises ScannerError on the
# first recipe that downloads anything. It is a template, and only
# tools/configure's substitution makes it parseable. Scanning lines is not a
# shortcut here; it is the only thing that reads what a human edits.

# Packages whose `configure` is not autoconf's and does not take --host.
EXEMPT = {
    # musl's own configure builds the libc that makes everything above this a
    # cross build. It takes --target, not --host, and it is already the one
    # package that drops the target flags entirely (manifest/toolchain.yaml).
    "musl",
    # OpenSSL ships its own Configure/config pair, which take a platform name
    # as a bare argument and reject --host outright.
    "openssl",
    # losos-15-hosttools. These three are not cross builds at all: they carry a
    # `drops: [target]` exception (manifest/toolchain.yaml) because the kernel's
    # build and systemd's RUN them, and a musl-dynamic binary cannot run in
    # pm's jail. --host here would be a lie told to autoconf -- a native glibc
    # compile announcing itself as a cross build to musl -- and autoconf acts
    # on it, disabling the run-time tests whose answers it then has no way to
    # get right. They are listed by name rather than by layer so that a fourth
    # host tool has to be argued for.
    "gperf",
    "flex",
    "gettext",
}


def main():
    failures = []
    checked = 0
    meson_checked = 0

    for template in sorted((REPO / "recipes").glob("*/*/build.yaml.in")):
        package = template.parent.name
        for number, line in enumerate(template.read_text().splitlines(), 1):
            stripped = line.strip()
            if stripped.startswith("#"):
                continue

            if MESON_SETUP.search(line):
                meson_checked += 1
                if "--cross-file" not in line:
                    failures.append(
                        f"{template.relative_to(REPO)}:{number}: meson setup "
                        f"with no --cross-file.\n    Add --cross-file "
                        f"@RECIPE@/cross.ini (or @CROSS_INI_{package.upper().replace('-', '_')}@ "
                        f"if {package} has a toolchain exception); "
                        f"--native-file alone is a native build."
                    )

            if not CONFIGURE.search(line):
                continue
            if package in EXEMPT:
                continue
            checked += 1
            if "--host=" not in line:
                failures.append(
                    f"{template.relative_to(REPO)}:{number}: configure with no "
                    f"--host.\n    Add --host=@ARCH_TRIPLE@, or name {package} "
                    f"in EXEMPT here with the reason."
                )

    if failures:
        print("cross-configure: FAILED", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1

    print(
        f"cross-configure: {checked} autotools configure call(s) say --host, "
        f"{len(EXEMPT)} exempt; {meson_checked} meson setup call(s) say "
        f"--cross-file"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
