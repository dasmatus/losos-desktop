#!/usr/bin/env python3
"""Check the plugins against the contract they were compiled with.

Two failure modes, both silent:

  * **Contract drift.** `plugins/wit/plugin.wit` is a copy of pm's. If pm's
    changes and this one does not, the plugins still build and still load, and
    then answer with a layout the host reads differently. Comparing the bytes
    is cheap and turns that into one line.
  * **A stale component.** `dist/*.wasm` is a build artifact of `plugins/`. If
    the source moved on and nobody re-ran build.sh, pm loads yesterday's
    behaviour while the source says otherwise.

Neither is something pm can catch: from pm's side a stale plugin is simply a
plugin.

Staleness is decided by a CONTENT HASH, not by mtimes. This gate compared
`src/lib.rs`'s mtime against the component's until CI went red on a tree nobody
had touched: git does not record mtimes, so every file in a fresh clone is
stamped at checkout time and the comparison decides by whatever order the
checkout happened to write them in. It was also blind to every source file but
`lib.rs`. `plugins/build.sh` now writes `dist/<crate>.srchash` alongside the
component, using the same function below -- called through `--write-hashes`, so
there is one implementation rather than two that can disagree.
"""

import argparse
import hashlib
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
PLUGINS = REPO / "plugins"
PM_ROOT = Path(__import__("os").environ.get("PM_ROOT", REPO.parent / "pm"))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


# What a crate's hash covers: its sources, its manifest, and the contract it is
# compiled against. target/ is excluded because it is the build output, and
# dist/ because that is what the hash is being compared to.
def source_digest(crate):
    """A hash of everything that decides what `crate`'s component contains."""
    files = sorted(
        path
        for path in (PLUGINS / crate).rglob("*")
        if path.is_file() and "target" not in path.relative_to(PLUGINS).parts
    )
    # Plus the shared inputs: one crate's component changes when the workspace
    # lock or the contract changes, even though nothing under its own directory
    # did.
    for shared in ("Cargo.lock", "wit/plugin.wit"):
        path = PLUGINS / shared
        if path.exists():
            files.append(path)

    # The path goes into the hash as well as the bytes, so moving a file is a
    # change. Relative to plugins/, so the hash does not depend on where the
    # repository is checked out.
    running = hashlib.sha256()
    for path in files:
        running.update(str(path.relative_to(PLUGINS)).encode())
        running.update(b"\0")
        running.update(digest(path).encode())
        running.update(b"\n")
    return running.hexdigest()


def crate_names():
    return sorted(p.parent.parent.name for p in PLUGINS.glob("*/src/lib.rs"))


def write_hashes():
    """Record each built component's source hash. Called by plugins/build.sh."""
    for crate in crate_names():
        (PLUGINS / "dist" / f"{crate}.srchash").write_text(source_digest(crate) + "\n")
        print(f"  {crate:15} source hash recorded")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--write-hashes",
        action="store_true",
        help="record each component's source hash (plugins/build.sh does this)",
    )
    args = parser.parse_args()
    if args.write_hashes:
        return write_hashes()

    failures = []

    vendored = PLUGINS / "wit" / "plugin.wit"
    if not vendored.exists():
        sys.exit("plugins: no vendored wit/plugin.wit")

    upstream = PM_ROOT / "wit" / "plugin.wit"
    if upstream.exists():
        if digest(vendored) != digest(upstream):
            failures.append(
                f"contract drift: {vendored.relative_to(REPO)} differs from "
                f"{upstream}\n    re-copy it and rebuild the plugins"
            )
        else:
            print(f"  contract    matches {upstream}")
    else:
        # Not a failure: the sibling checkout is a convention, not a guarantee.
        print(f"  contract    {upstream} absent; drift not checked")

    crates = crate_names()
    for crate in crates:
        component = PLUGINS / "dist" / f"{crate}.wasm"
        if not component.exists():
            failures.append(f"{crate}: not built -- run plugins/build.sh")
            continue

        recorded = PLUGINS / "dist" / f"{crate}.srchash"
        if not recorded.exists():
            failures.append(
                f"{crate}: {component.relative_to(REPO)} has no recorded source "
                f"hash -- run plugins/build.sh"
            )
            continue

        if recorded.read_text().strip() != source_digest(crate):
            failures.append(
                f"{crate}: sources have changed since the component was built "
                f"-- run plugins/build.sh"
            )
            continue

        print(f"  {crate:15} {component.stat().st_size:>7} bytes, current")

    if failures:
        print("plugins: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"plugins: {len(crates)} plugin(s) built against the current contract")
    return 0


if __name__ == "__main__":
    sys.exit(main())
