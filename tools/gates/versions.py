#!/usr/bin/env python3
"""Check that a recipe's `version:` matches the source it downloads.

Every recipe states its upstream version twice: once as pm's `version:` list,
and once implicitly through the sources.lock key it pulls from. Nothing ties
them together, so a version bump done in sources.lock alone produces a package
that builds the new sources and calls itself the old version -- in its archive
name, in `pm explain`, and in every dependent's carried copy. Nothing fails;
the tree just quietly starts lying about what it contains.

That is not hypothetical: the bump from 6.12.1 to 7.2.6 changed sources.lock
and left both kernel recipes claiming '6','12','1'.

Fixing it with a placeholder would mean splicing a YAML list into every
template, which is a change to ninety files to catch a mistake in one. This
checks instead, which is cheaper and catches the same thing.

A recipe with no dl_urls has no upstream version to agree with and is skipped.
"""

import re
import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent

# @URL_<KEY>@ inside a dl_urls block is what names the source.
URL_TOKEN = re.compile(r"@URL_([A-Z0-9_]+)@")
# `version:` followed by its list items, in the template's raw text -- the
# template is not valid YAML before substitution, so this is read as text.
VERSION_BLOCK = re.compile(r"^version:\n((?:- .*\n)+)", re.M)


def recipe_version(text):
    match = VERSION_BLOCK.search(text)
    if not match:
        return None
    return [
        item.strip().lstrip("- ").strip("'\"")
        for item in match.group(1).splitlines()
    ]


def main():
    lock = yaml.safe_load((REPO / "manifest" / "sources.lock").read_text()) or {}
    failures = []
    checked = 0

    for template in sorted((REPO / "recipes").glob("*/*/build.yaml.in")):
        text = template.read_text()
        keys = set(URL_TOKEN.findall(text))
        if not keys:
            continue
        if len(keys) > 1:
            # A recipe that downloads several sources has no single upstream
            # version to agree with; the kernel's bpftool is the shape this
            # would be, if it were not built from the kernel's own tree.
            continue

        key = keys.pop()
        entry = lock.get(key)
        if entry is None:
            failures.append(f"{template.relative_to(REPO)}: @URL_{key}@ is not in sources.lock")
            continue

        declared = recipe_version(text)
        if declared is None:
            failures.append(f"{template.relative_to(REPO)}: no `version:` list")
            continue

        upstream = str(entry.get("version", "")).split(".")
        if declared != upstream:
            failures.append(
                f"{template.relative_to(REPO)}: version {'.'.join(declared)} "
                f"but {key} is {entry.get('version')}"
            )
            continue

        checked += 1

    if failures:
        print("versions: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        print(
            "\n  Each recipe's `version:` list is its source's version split on\n"
            "  dots. Bumping one without the other ships the new sources under\n"
            "  the old name.",
            file=sys.stderr,
        )
        return 1

    print(f"versions: {checked} recipe(s) agree with sources.lock")
    return 0


if __name__ == "__main__":
    sys.exit(main())
