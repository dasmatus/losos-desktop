#!/usr/bin/env python3
"""Check the plugins against the contract they were compiled with.

**Contract drift** is the failure this exists for. `plugins/wit/plugin.wit` is a
copy of pm's. If pm's changes and this one does not, a component still builds
and still loads, and then answers with a record the host reads differently --
or, once a field is added, stops loading at all with "expected record of 7
fields, found 6" and takes every recipe in the tree down with it. Comparing the
bytes is cheap and turns that into one line. It is not something pm can catch:
from pm's side a plugin built against an older contract is simply a plugin.

There is deliberately **no staleness check**, because there is nothing to go
stale. `plugins/dist/` is build output and is not tracked: `./do plugins`
produces it from the sources beside it, and the sign-all recipe installs whatever
is there into pm's trust store. An earlier version of this file compared the
component's mtime against `src/lib.rs`'s, which was wrong twice over -- git
does not record mtimes, so in a fresh clone the comparison decided by whichever
order the checkout wrote the two files in, and it only ever looked at
`lib.rs`. Both bugs existed only because a compiled artifact was committed.
"""

import hashlib
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
                f"{upstream}\n    re-copy it and run ./do plugins"
            )
        else:
            print(f"  contract    matches {upstream}")
    else:
        # Not a failure: the sibling checkout is a convention, not a guarantee.
        print(f"  contract    {upstream} absent; drift not checked")

    crates = sorted(p.parent.parent.name for p in PLUGINS.glob("*/src/lib.rs"))
    for crate in crates:
        component = PLUGINS / "dist" / f"{crate}.wasm"
        if component.exists():
            print(f"  {crate:15} {component.stat().st_size:>7} bytes, built")
        else:
            # Informational, not a failure. `./do check` is meant to run on a
            # machine with no wasm toolchain and no network; requiring a built
            # component here would take that away for no gain, since pm simply
            # runs without a plugin it does not have.
            print(f"  {crate:15} not built (./do plugins)")

    if failures:
        print("plugins: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"plugins: {len(crates)} plugin(s) against the current contract")
    return 0


if __name__ == "__main__":
    sys.exit(main())
