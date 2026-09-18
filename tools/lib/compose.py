"""Compose per-package recipe fragments into one layer bundle.

Why bundles at all, when a package DAG is the obvious shape: pm has no build
cache and copies each dependency's whole archive into its dependent
(C5, C6 in docs/pm-constraints.md). A 90-node DAG would rebuild every package
on every attempt, and a package that twelve others depend on would have its
bytes duplicated twelve times over. A short chain of layer bundles keeps the
nesting depth at the number of layers and rebuilds a layer, not the world.

Per-package recipes remain the authored unit -- one directory, one upstream,
reviewable on its own. This module is what turns them into the six build files
pm actually sees.

The one rule that is not obvious: **every member step lands in the `Build`
stage.** pm sorts steps by stage and keeps authored order only *within* a
stage (`BuildFile::execute_steps`), so a member that kept an `Install` step
would have it float past every later member and run against a sysroot those
members had not populated yet. Stages are a pm-level ordering mechanism and
cannot also be a package-level one; inside a bundle, order is the list.
"""

from collections import OrderedDict

# Step names a member may use that the bundle provides once, for all members,
# and therefore drops when composing.
SHARED_STEPS = {"sysroot"}


def compose(layer, members, depends_on):
    """Build one bundle build file from a list of rendered member build files.

    `members` is a list of (package_name, build_file_dict) in build order.
    `depends_on` is the path of the layer below, or None for the bottom.
    """
    downloads = OrderedDict()
    steps = []

    for package, build in members:
        for step in build.get("steps") or []:
            urls = step.get("dl_urls") or {}
            for url, digest in urls.items():
                previous = downloads.get(url)
                if previous is not None and previous != digest:
                    raise ValueError(
                        f"{layer}: {url} is pinned to two different hashes"
                    )
                downloads[url] = digest

            if step.get("name") in SHARED_STEPS:
                continue

            steps.append(
                OrderedDict(
                    stage="Build",
                    dl_urls=None,
                    name=f"{package}/{step['name']}",
                    run=list(step["run"]),
                )
            )

    # One Prepare step for the whole layer: the downloads, the directory
    # skeleton every member assumes, and the dependency-closure unpack.
    #
    # Hoisting the downloads here is presentation, not policy: pm unions
    # capabilities across the whole build file, so the layer is networked
    # either way (C8). Collecting them makes `pm explain` readable and makes a
    # duplicate pin an error rather than a silent last-one-wins.
    prepare = OrderedDict(
        stage="Prepare",
        dl_urls=dict(downloads) or None,
        name="prepare",
        run=[
            "mkdir -p /build/src /build/b /build/sysroot",
            "/bin/sh @RECIPE@/sysroot.sh /dest/deps /build/sysroot",
        ],
    )

    return OrderedDict(
        name=layer,
        version=["0", "1", "0"],
        dependencies=[depends_on] if depends_on else [],
        steps=[prepare] + steps,
    )
