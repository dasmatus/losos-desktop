#!/usr/bin/env python3
"""Check that every patch series is well-formed and actually applied.

Two silent failures this catches, both of which produce a green build and a
wrong system:

  * **A series nobody applies.** Dropping a .patch into files/<pkg>/patches/ and
    forgetting the apply step in the recipe leaves the patch tracked, reviewed,
    and doing nothing. The build passes; the feature is missing. For the
    factory-reset patch that means a Settings panel with no reset button.
  * **A patch that is not a patch.** A file saved with the wrong extension, or a
    diff truncated in a copy, fails at build time inside a jail rather than here.

It does NOT check that a patch still applies to its upstream -- that needs the
tarball, which means the network and a download per package. `apply-patches.sh`
uses --fuzz=0 precisely so that check happens for real at build time, loudly,
rather than being approximated here.
"""

import re
import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent

# A unified diff has to have at least one file header pair and one hunk.
FILE_HEADER = re.compile(r"^--- \S", re.M)
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+\d+(?:,\d+)? @@", re.M)


def main():
    failures = []
    series = sorted((REPO / "recipes").glob("*/*/files/patches/*.patch"))

    # Group by the package directory that owns them.
    by_package = {}
    for patch in series:
        package = patch.parent.parent.parent
        by_package.setdefault(package, []).append(patch)

    for package, patches in sorted(by_package.items()):
        name = package.name
        template = package / "build.yaml.in"
        if not template.exists():
            failures.append(f"{name}: has patches but no build.yaml.in")
            continue

        body = template.read_text()
        # The apply step has to name THIS package's patch directory. A recipe
        # that applies some other package's series is not a thing that happens
        # by accident, but a copied recipe that still names the original is.
        expected = f"files/{name}/patches"
        if "apply-patches.sh" not in body or expected not in body:
            failures.append(
                f"{name}: {len(patches)} patch(es) in files/patches/ that no "
                f"step applies.\n    Add an apply-patches.sh step naming "
                f"{expected}, or delete them."
            )

        for patch in patches:
            text = patch.read_text()
            if not FILE_HEADER.search(text):
                failures.append(f"{patch.relative_to(REPO)}: no `--- <file>` header")
            elif not HUNK.search(text):
                failures.append(f"{patch.relative_to(REPO)}: no @@ hunk")
            else:
                print(f"  {name:24} {patch.name}")

    if failures:
        print("patches: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"patches: {len(series)} patch(es) across {len(by_package)} package(s), "
          "each applied by its recipe")
    return 0


if __name__ == "__main__":
    sys.exit(main())
