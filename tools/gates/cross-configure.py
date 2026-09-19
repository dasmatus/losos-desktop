#!/usr/bin/env python3
"""Every autotools configure must say it is cross-compiling, because it is.

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

This is a template gate rather than a generated-file one on purpose: the
answer has to be in the recipe a human edits.
"""

import re
import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent

# A step that runs a source tree's `configure`. The leading slash keeps this
# off tools/configure, which recipes mention in comments.
CONFIGURE = re.compile(r"/configure(?=\s|$)")

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

    for template in sorted((REPO / "recipes").glob("*/*/build.yaml.in")):
        package = template.parent.name
        doc = yaml.safe_load(template.read_text()) or {}
        for step in doc.get("steps") or []:
            for command in step.get("run") or []:
                if not CONFIGURE.search(command):
                    continue
                if package in EXEMPT:
                    continue
                checked += 1
                if "--host=" not in command:
                    failures.append(
                        f"{template.relative_to(REPO)}:{step.get('name', '?')}: "
                        f"configure with no --host.\n    Add "
                        f"--host=@ARCH_TRIPLE@, or name {package} in EXEMPT here "
                        f"with the reason."
                    )
                continue

    if failures:
        print("cross-configure: FAILED", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1

    print(
        f"cross-configure: {checked} autotools configure call(s) say --host, "
        f"{len(EXEMPT)} exempt"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
