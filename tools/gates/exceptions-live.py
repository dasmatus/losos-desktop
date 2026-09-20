#!/usr/bin/env python3
"""Every toolchain exception must be named by the recipe it is written for.

manifest/toolchain.yaml's `exceptions:` list is the honest part of this tree's
CFI claim, and tools/gates/toolchain-report.py prints it on every run so the
holes are counted rather than implied away. That is worth nothing if an entry
can be written, reviewed, argued for in a `why:` paragraph -- and then apply to
nothing at all.

Which is exactly what happened. tools/configure emits cflags-<pkg>.rsp and
ldflags-<pkg>.rsp for each exempt package and defines @CFLAGS_RSP_<PKG>@ for
them, and a recipe gets its exemption only by NAMING that token; left spelling
the shared @CFLAGS_RSP@, the files are written every run and read by nothing.
gjs carried a four-scheme exemption for SpiderMonkey's JIT that way -- the
manifest said four schemes were off, the build had all five on, and neither the
gate output nor the recipe said so. The exemption was inert for as long as it
existed and nothing anywhere could have told you.

meson is the reason that is easy to do by accident rather than careless. Every
other build system here takes flags as a VAR=value argument or a -D, so the
token has somewhere obvious to go; meson takes them only from a native file,
and the native file is per LAYER. tools/configure now writes native-<pkg>.ini
for an exempt package too, and @NATIVE_INI_<PKG>@ is the third way a recipe can
name its exemption.

EXEMPT is for a package that takes none of this tree's flags in the first
place, where there is no token to name because there is no flag line to name it
on. It is a list of names rather than a rule, because an entry on it is a claim
that has to be argued.

This reads the template a human edits, not the generated tree: the answer has
to be in the file someone will change next.
"""

import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent

# The kernel. `drops: [cfi, lto, hardening]` is a true statement about how it is
# built and not one this tree's response files implement: recipes/*/linux builds
# with LLVM=1 and the kernel's own Kbuild flags, and passes neither @CFLAGS_RSP@
# nor a native file. There is no line in that recipe on which a per-package
# token would mean anything, so requiring one would only produce a decorative
# argument nothing reads. The manifest entry stays because toolchain-report.py
# counting the kernel among the packages outside the full scheme set is correct
# and is the point of the list.
EXEMPT = {
    "linux",
}


def main():
    manifest = yaml.safe_load((REPO / "manifest" / "toolchain.yaml").read_text())
    exceptions = manifest.get("exceptions") or []

    failures = []
    checked = 0

    for entry in exceptions:
        package = entry.get("package")
        if not package:
            failures.append("an exception with no `package:` -- nothing to check")
            continue
        if package in EXEMPT:
            continue

        templates = sorted((REPO / "recipes").glob(f"*/{package}/build.yaml.in"))
        if not templates:
            failures.append(
                f"{package}: an exception for a package with no recipe.\n"
                f"    Either it was renamed and this entry was left behind, or "
                f"the entry is aspirational. Both are worth failing on."
            )
            continue

        token = package.upper().replace("-", "_")
        names = (
            f"@CFLAGS_RSP_{token}@",
            f"@LDFLAGS_RSP_{token}@",
            f"@NATIVE_INI_{token}@",
        )

        for template in templates:
            checked += 1
            text = template.read_text()
            if any(name in text for name in names):
                continue
            drops = ", ".join(entry.get("drops") or []) or "nothing"
            failures.append(
                f"{template.relative_to(REPO)}: {package} has an exception "
                f"dropping {drops}, and names none of it.\n"
                f"    The recipe spells the shared flags, so the exemption is "
                f"written to disk every run and read by nothing.\n"
                f"    Name {names[0]} and {names[1]} where the recipe passes "
                f"CFLAGS and LDFLAGS, or {names[2]} as its --native-file if it "
                f"is a meson build."
            )

    if failures:
        print("exceptions-live: FAILED", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1

    print(
        f"exceptions-live: {checked} toolchain exception(s) reach the recipe "
        f"they were written for, {len(EXEMPT)} exempt"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
