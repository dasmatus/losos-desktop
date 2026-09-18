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
"""

import hashlib
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
PLUGINS = REPO / "plugins"
PM_ROOT = Path(__import__("os").environ.get("PM_ROOT", REPO.parent / "pm"))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
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

    crates = sorted(p.parent.parent.name for p in PLUGINS.glob("*/src/lib.rs"))
    for crate in crates:
        component = PLUGINS / "dist" / f"{crate}.wasm"
        if not component.exists():
            failures.append(f"{crate}: not built -- run plugins/build.sh")
            continue
        source = PLUGINS / crate / "src" / "lib.rs"
        if source.stat().st_mtime > component.stat().st_mtime:
            failures.append(
                f"{crate}: {source.relative_to(REPO)} is newer than its "
                f"component -- run plugins/build.sh"
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
