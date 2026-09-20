#!/usr/bin/env python3
"""Every build system a recipe invokes must be present in the pinned tarball.

Four times now a recipe has driven a build system its source does not have.
libcap and libbpf, then openssl -- which ships `Configure` and `config` and no
`configure` at all, and has since 1998 -- then dbus, which removed autotools in
1.16 and whose recipe was still running `/build/src/dbus/configure`. Each of
those is a one-line mistake that no gate in ./do check can see, because the
answer is not in the tree: it is in bytes the tree only names. So each was
found the expensive way, by a build that ran for an hour and stopped at member
N, and the next one was found by the build after that.

This is the gate that reads the bytes. It cannot live in ./do check -- that
runs with no network and nothing mirrored -- so it runs where the tarballs
already are: `just build`, after the source mirror is up and before pm starts.
The cost is a tar listing per source and the payoff is every one of these
reported at once, in seconds, instead of one per build.

What it checks is narrow on purpose. For each source a recipe extracts, it
reads the tarball's member list, strips --strip-components the way tar will,
and then asks one question of each entry point the recipe names:

    /build/src/X/configure        ->  the tarball must contain `configure`
    cmake -S /build/src/X         ->  ... CMakeLists.txt
    meson.py setup /build/b/X /build/src/X
                                  ->  ... meson.build
    make -C /build/src/X          ->  ... Makefile or GNUmakefile

A path an earlier step creates counts as present: openssl's recipe copies
Configure to configure in Prepare, deliberately and with a comment, and this
gate has to agree that the file exists by the time the Build stage names it.
The same goes for a file a patch adds, so the patch series is read for the
paths it creates.

What it does NOT check is anything about the invocation beyond existence: not
the options, not whether they mean today what they meant at the pinned
version, not whether the build then succeeds. The tombstone in this directory
already says nothing here validates a recipe's -D flags against sources it does
not have, and that is still true; this gate only closes the coarser question of
whether the entry point is there to be invoked at all. A bump that crosses a
major version still needs a human.

It reads the template a human edits rather than the generated recipe, like
cross-configure.py, and for the same reason -- and it scans lines rather than
parsing, for the reason that gate's tombstone gives: a build.yaml.in is not
YAML until tools/configure has substituted it.
"""

import argparse
import io
import re
import sys
import tarfile
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(REPO / "tools" / "lib"))

from fnv import basename  # noqa: E402  -- after the path above, deliberately

# The marker each build system is known by, at the root of the extracted tree.
# GNUmakefile is listed with Makefile because make prefers it and a few
# projects ship only that one.
MAKEFILES = ("GNUmakefile", "makefile", "Makefile")

# `- tar -xf @DL_KEY@ ... -C <dir> --strip-components=N`. Every extraction in
# the tree has this shape today; a new one that does not is reported rather
# than skipped, because a source this gate cannot locate is a source it cannot
# vouch for.
TAR_LINE = re.compile(r"^\s*-\s+tar\s+(.*)$", re.M)
DL_TOKEN = re.compile(r"@DL_([A-Z0-9_]+)@")

# A run item, minus the leading `- `. Comments and keys are left behind.
RUN_ITEM = re.compile(r"^\s+-\s+(\S.*)$", re.M)

# `+++ b/path` in a unified diff, and `--- /dev/null` on the line before it is
# what makes it a file the patch creates.
PATCH_OLD = re.compile(r"^---\s+(\S+)", re.M)


def sources_of(text):
    """The (lock key, destination, strip count) of every tarball a recipe extracts."""
    found = []
    for match in TAR_LINE.finditer(text):
        words = match.group(1).split()
        key = dest = None
        strip = 0
        for index, word in enumerate(words):
            token = DL_TOKEN.fullmatch(word)
            if token:
                key = token.group(1)
            elif word == "-C" and index + 1 < len(words):
                dest = words[index + 1]
            elif word.startswith("--strip-components="):
                strip = int(word.split("=", 1)[1])
        if key and dest:
            found.append((key, dest.rstrip("/"), strip))
    return found


def names_tool(words, index, tool):
    """Whether `tool` is the command this flag belongs to.

    A flag means what its program says it means, and two programs in this tree
    spell the same flag differently: -C is make's directory and tar's
    destination, -S is cmake's source tree and nothing else's. Looking left
    from the flag is enough, because the one permitted wrapper
    (share/in-dir.sh) puts the real program on the same line ahead of its
    arguments.
    """
    return any(w == tool or w.endswith("/" + tool) for w in words[:index])


def wanted_from(text, dest):
    """The entry points a recipe names under one extracted source tree.

    Returns {relative path: what named it}, where several spellings may map to
    the same file -- `make -C` accepts any of three names, so that one is
    carried as a tuple and satisfied by any member of it.
    """
    wanted = {}
    prefix = dest + "/"

    def relative(path):
        """The path under the extracted tree, or None if it is somewhere else.

        The empty string is a real answer and the common one: `cmake -S` and
        `meson setup` are handed the source directory itself, which is `dest`
        exactly and not `dest/` -- so a plain startswith(prefix) test says no
        to every recipe this gate most needs to read.
        """
        if path == dest:
            return ""
        return path[len(prefix):] if path.startswith(prefix) else None

    for match in RUN_ITEM.finditer(text):
        words = match.group(1).split()
        if not words:
            continue

        copied_here = words[0] in ("cp", "mv", "ln")
        if copied_here and len(words) >= 3:
            rel = relative(words[-2])
            if rel:
                wanted.setdefault((rel,), ("copies", words[-2]))

        for index, word in enumerate(words):
            # A configure script, wherever on the line it sits: it is the
            # program word for a native build and in-dir.sh's argument for an
            # out-of-tree one.
            if word.endswith("/configure") and not copied_here:
                rel = relative(word)
                if rel:
                    wanted.setdefault((rel,), ("runs", word))

            # cmake -S <source tree>
            if word == "-S" and index + 1 < len(words) and names_tool(words, index, "cmake"):
                rel = relative(words[index + 1].rstrip("/"))
                if rel is not None:
                    joined = f"{rel}/CMakeLists.txt" if rel else "CMakeLists.txt"
                    wanted.setdefault((joined,), ("runs", " ".join(words[:2])))

            # make -C <directory>, and only make: -C is tar's destination flag
            # too, and every fetch step in the tree spells `tar -C /build/src/X`
            # -- which would otherwise demand a Makefile of every source here,
            # including the meson and cmake ones. ninja -C and meson -C name a
            # directory under /build/b, outside the extracted tree, so they fall
            # out on their own.
            if word == "-C" and index + 1 < len(words) and names_tool(words, index, "make"):
                rel = relative(words[index + 1].rstrip("/"))
                if rel is not None:
                    base = f"{rel}/" if rel else ""
                    wanted.setdefault(
                        tuple(base + name for name in MAKEFILES),
                        ("runs", " ".join(words[:2])),
                    )

            # meson setup <build dir> <source dir>: the source is the argument
            # that lands inside the extracted tree, whichever position it is in.
            if word == "setup" and "meson" in " ".join(words[:index + 1]):
                for candidate in words[index + 1:]:
                    if candidate.startswith("-"):
                        break
                    rel = relative(candidate.rstrip("/"))
                    if rel is not None:
                        joined = f"{rel}/meson.build" if rel else "meson.build"
                        wanted.setdefault((joined,), ("runs", "meson setup"))

    return wanted


def created_by(text, dest):
    """Paths under `dest` that the recipe itself makes before the build stage.

    openssl is the reason: it copies Configure to configure on purpose, because
    pm canonicalises a step's first word on the host and canonicalising would
    resolve a symlink back to the name Configure (C3). The copy is real by the
    time anything names it, and a gate that called it missing would be telling
    the recipe to undo a fix.
    """
    made = set()
    prefix = dest + "/"
    for match in RUN_ITEM.finditer(text):
        words = match.group(1).split()
        if words and words[0] in ("cp", "mv", "ln") and len(words) >= 3:
            target = words[-1]
            if target.startswith(prefix):
                made.add(target[len(prefix):])
    return made


def created_by_patches(recipe_dir):
    """Paths a patch series adds, read from the diffs themselves."""
    made = set()
    for patch in sorted(recipe_dir.glob("files/patches/*.patch")):
        lines = patch.read_text(errors="replace").splitlines()
        for index, line in enumerate(lines):
            if line.startswith("--- /dev/null") and index + 1 < len(lines):
                nxt = lines[index + 1]
                if nxt.startswith("+++ "):
                    path = nxt[4:].split("\t")[0].strip()
                    if path.startswith("b/"):
                        made.add(path[2:])
    return made


def resolve(wanted, tarball, strip):
    """Stream the tarball, ticking off what was wanted; return what was not.

    Streaming with an early exit rather than getnames(): the passing case is
    every entry point found in the first handful of members, and the kernel's
    tarball is a gigabyte of xz that nothing here needs decompressed in full.
    Only a real miss pays for the whole listing, which is the right way round.
    """
    outstanding = dict(wanted)
    with tarfile.open(tarball) as archive:
        for member in archive:
            parts = member.name.split("/")
            if len(parts) <= strip:
                continue
            relative = "/".join(parts[strip:])
            for names in list(outstanding):
                if relative in names:
                    del outstanding[names]
            if not outstanding:
                break
    return outstanding


def self_test():
    """Exercise the reader on recipes and tarballs built for the purpose.

    This gate is the only one here whose real input is bytes ./do check does not
    have, so without this its logic is never run except during a real build --
    and a gate that is never run is a gate that can be refactored into agreeing
    with everything, which is what happened to gjs's toolchain exception. The
    cases below are the four failures that motivated it, reduced to the smallest
    recipe that shows each.
    """
    import tempfile

    def recipe(key, dest, *runs):
        body = [
            "steps:", "", "- stage: Prepare", "  name: fetch", "  run:",
            f"  - tar -xf @DL_{key}@ --no-same-owner -C {dest} --strip-components=1",
            "", "- stage: Build", "  name: configure", "  run:",
        ]
        body += [f"  - {run}" for run in runs]
        return "\n".join(body) + "\n"

    cases = [
        # (name, recipe runs, tarball members, how many entry points survive
        #  the created-by pass, how many of those the tarball is missing)
        ("dbus as it was: autotools against a meson-only tarball",
         ["/bin/sh in-dir.sh /build/b/x /build/src/x/configure --host=y"],
         ["meson.build", "CMakeLists.txt"], 1, 1),
        ("dbus as it is: meson against the same tarball",
         ["python3 meson.py setup /build/b/x /build/src/x --prefix=/usr"],
         ["meson.build", "CMakeLists.txt"], 1, 0),
        ("openssl: configure named, and made by the recipe first",
         ["cp /build/src/x/Configure /build/src/x/configure",
          "/bin/sh in-dir.sh /build/b/x /build/src/x/configure linux-x86_64"],
         ["Configure", "config"], 1, 0),
        ("openssl without that copy, which is the bug it fixed",
         ["/bin/sh in-dir.sh /build/b/x /build/src/x/configure linux-x86_64"],
         ["Configure", "config"], 1, 1),
        ("libbpf: make -C a subdirectory that has the Makefile",
         ["make -C /build/src/x/src -j 4"],
         ["src/Makefile", "README.md"], 1, 0),
        ("libbpf as it was: meson against a tarball with none",
         ["python3 meson.py setup /build/b/x /build/src/x"],
         ["src/Makefile"], 1, 1),
        ("cmake -S the source tree itself",
         ["cmake -S /build/src/x -B /build/b/x -DBUILD_SHARED_LIBS=ON"],
         ["CMakeLists.txt"], 1, 0),
        # The regression that made every recipe fail at once: tar's -C is a
        # destination, not a Makefile directory, and it is on every fetch step.
        ("tar's own -C must demand nothing",
         ["python3 meson.py setup /build/b/x /build/src/x"],
         ["meson.build"], 1, 0),
    ]

    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        for label, runs, members, expect_wanted, expect_missing in cases:
            text = recipe("X", "/build/src/x", *runs)
            sources = sources_of(text)
            if sources != [("X", "/build/src/x", 1)]:
                failures.append(f"{label}: read the tar line as {sources}")
                continue

            tarball = Path(tmp) / "t.tar"
            with tarfile.open(tarball, "w") as archive:
                for member in members:
                    info = tarfile.TarInfo(f"top/{member}")
                    info.size = 0
                    archive.addfile(info, io.BytesIO(b""))

            wanted = wanted_from(text, "/build/src/x")
            made = created_by(text, "/build/src/x")
            wanted = {
                names: why for names, why in wanted.items()
                if not any(candidate in made for candidate in names)
            }
            if len(wanted) != expect_wanted:
                failures.append(
                    f"{label}: wanted {len(wanted)} entry point(s), "
                    f"expected {expect_wanted}: {list(wanted)}"
                )
                continue

            missing = resolve(wanted, tarball, 1)
            if len(missing) != expect_missing:
                failures.append(
                    f"{label}: {len(missing)} missing, expected "
                    f"{expect_missing}: {list(missing)}"
                )

    if failures:
        print("recipe-entrypoints --self-test: FAILED", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1
    print(f"recipe-entrypoints: self-test green, {len(cases)} case(s)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--mirror", default=str(REPO / "out" / "sources"),
                        help="where tools/fetch-sources staged the tarballs")
    parser.add_argument("--self-test", action="store_true",
                        help="run the reader against recipes and tarballs "
                             "built for the purpose, with no mirror")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    lock = yaml.safe_load((REPO / "manifest" / "sources.lock").read_text()) or {}
    mirror = Path(args.mirror)

    failures, missing_bytes, checked = [], [], 0

    for template in sorted(REPO.glob("recipes/*/*/build.yaml.in")):
        text = template.read_text()
        name = template.parent.name
        patched = created_by_patches(template.parent)

        for key, dest, strip in sources_of(text):
            entry = lock.get(key)
            if not entry:
                failures.append(
                    f"{template.relative_to(REPO)}: extracts @DL_{key}@, "
                    f"which manifest/sources.lock does not define."
                )
                continue

            digest = str(entry.get("sha256", "TODO"))
            if digest == "TODO":
                # An unresolved pin is sources.lock's own sentinel and
                # tools/configure already refuses to generate on it. Saying so
                # twice would only bury this gate's own findings.
                continue

            # Same layout tools/fetch-sources writes and tools/serve-sources
            # serves: content-addressed by the pinned hash, so a changed pin
            # never finds the old bytes.
            tarball = mirror / digest / basename(entry["url"])
            if not tarball.exists():
                missing_bytes.append(f"{name} ({key})")
                continue

            wanted = wanted_from(text, dest)
            if not wanted:
                continue

            made = created_by(text, dest) | patched
            wanted = {
                names: why for names, why in wanted.items()
                if not any(candidate in made for candidate in names)
            }
            if not wanted:
                continue

            checked += len(wanted)
            for names, (verb, why) in resolve(wanted, tarball, strip).items():
                spelling = " or ".join(names)
                failures.append(
                    f"{template.relative_to(REPO)}: the recipe {verb} `{why}`, "
                    f"and the pinned tarball has no {spelling}.\n"
                    f"    {entry['url']}\n"
                    f"    Either the source moved to another build system or "
                    f"it never had this one. Read what the tarball does ship "
                    f"before translating the options."
                )

    if failures:
        print("recipe-entrypoints: FAILED", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1

    note = ""
    if missing_bytes:
        # Not a failure: this gate runs before a partial build too, and a
        # source that is not mirrored yet is a source it has nothing to say
        # about. Counting them out loud is what keeps a green line from
        # reading as more coverage than it is.
        note = f", {len(missing_bytes)} source(s) not mirrored and not checked"
    print(f"recipe-entrypoints: {checked} build-system entry point(s) present{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
