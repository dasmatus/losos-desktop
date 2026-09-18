#!/usr/bin/env python3
"""Refuse to build a layer whose dependency chain has an unpinned source.

`sha256: TODO` in manifest/sources.lock is a sentinel meaning "these bytes have
not been fetched and hashed yet". It reaches the generated build file as the
literal hash `TODO`, and pm would fail on it -- but late, after downloading,
with a hash-mismatch error that reads like a compromised mirror rather than
like a missing entry in a lock file.

So it is caught here instead, and only for the layers the requested build will
actually walk. A tree where the upper layers are not yet fetched is the normal
state while a distribution is being brought up, and it should not stop the
lower ones being built.
"""

import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent
RECIPES = REPO / "out" / "recipes"


def chain(target):
    """The target layer and everything below it, bottom first."""
    seen, order = set(), []
    stack = [target]
    while stack:
        name = stack.pop()
        if name in seen:
            continue
        seen.add(name)
        build = RECIPES / name / "build.yaml"
        if not build.exists():
            sys.exit(f"chain-pinned: no generated recipe for {name}")
        doc = yaml.safe_load(build.read_text()) or {}
        order.append((name, doc))
        for dep in doc.get("dependencies") or []:
            # Dependencies are spelled relative to out/pkgs (C11).
            stack.append(Path(dep).parent.name)
    return list(reversed(order))


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: chain-pinned.py <layer>")

    unpinned = []
    layers = chain(sys.argv[1])
    for name, doc in layers:
        for step in doc.get("steps") or []:
            for url, digest in (step.get("dl_urls") or {}).items():
                if str(digest).strip() == "TODO":
                    unpinned.append((name, url))

    if unpinned:
        print(
            f"chain-pinned: {len(unpinned)} source(s) in this chain are not pinned:",
            file=sys.stderr,
        )
        for name, url in unpinned:
            print(f"  {name}: {url}", file=sys.stderr)
        print(
            "\nchain-pinned: run `./do fetch --update` on a machine that can reach\n"
            "              these hosts, then commit manifest/sources.lock.",
            file=sys.stderr,
        )
        return 1

    print(f"chain-pinned: {len(layers)} layer(s) fully pinned")
    return 0


if __name__ == "__main__":
    sys.exit(main())
