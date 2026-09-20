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

It asks a second question of the same bytes, for meson recipes only: does
every -D the recipe passes name an option the pinned source declares, and is
the value the right kind for it? Seventeen did not, across six packages, found
by hand in one evening -- systemd alone had twelve, four of them options
removed or renamed upstream, each a hard error on the first line of `meson
setup`. That is the class the tombstone in this directory says nothing
validates, and the reason it went unvalidated was never that it is hard: the
answer lives in a file this tree downloads and never opens. meson_options.txt
is in the tarball, beside the meson.build this gate already went looking for.

The check is narrow on purpose: a name the source does not declare, a feature
given `true`, a boolean given `enabled`, a combo given something outside its
own choices. It says nothing about whether an option still MEANS what it meant
at the pinned version -- `-Dtests=false` on a project that moved its tests
behind a different name is a passing line and a wrong one -- and nothing about
whether the build then succeeds. A bump that crosses a major version still
needs a human.

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


# Recipes whose entry point is wrong, where the answer is a package this tree
# does not have yet rather than a line in the recipe. These are REPORTED on
# every run, in full, and do not fail the build -- which is the distinction
# this list exists to draw. A gate that blocks every build on a problem four
# layers above where the build currently stops is not finding things earlier;
# it is stopping the work that would reach them. A gate that goes quiet about
# them is the gjs failure again.
#
# An entry earns its place by naming what is actually missing. Neither of these
# is a recipe someone can fix by reading it: both are recipes written against a
# different upstream release than manifest/sources.lock pins, and the pin is
# the constrained end. docs/limits.md carries the reasoning.
DEFERRED = {
    "gnome-keyring": (
        "The recipe is meson and 46.2 is autotools -- and rewriting it "
        "backwards does not help, because 46.2 wants gcr-3, gck-1 and "
        "gcr-base-3 while this tree pins gcr 4.4.1, which ships gcr-4 and "
        "gck-2. The recipe was written for gnome-keyring 48, which is meson "
        "and wants gcr-4. Waiting on that bump, whose bytes have to be "
        "hashed rather than guessed."
    ),
    "xdg-desktop-portal": (
        "The recipe is autotools and 1.18.4 is meson, but the option set is "
        "the smaller half. Its meson.build takes fuse3 and libpipewire-0.3 "
        "as unconditional dependencies and needs bwrap at configure time for "
        "sandboxed image validation, and losos-40-gnome builds none of the "
        "three. Waiting on those packages; a probe that succeeded without "
        "them would have found the build host's, through pm's /usr mirror."
    ),
}

# What a build system looks like from the outside, for the "it has this
# instead" half of a failure. Only the root is reported: a meson.build three
# directories down is a subproject, not the answer to what to run.
MARKERS = (
    "configure", "Configure", "configure.ac", "autogen.sh",
    "meson.build", "CMakeLists.txt", "Makefile", "GNUmakefile", "Makefile.am",
)


# meson renamed its own option file: meson.options is current, meson_options
# the older spelling. A project ships one or the other, so the first found
# wins and finding neither is not a failure -- a project may declare no
# options at all, and the recipe then passes no -D either.
OPTION_FILES = ("meson.options", "meson_options.txt")

# meson's built-in options live in its core rather than in a project's option
# file, so a recipe naming one is not naming a project option and there is
# nothing here to check it against. Listed rather than guessed at from a
# prefix: `python.install_env` and `pkgconfig.relocatable` have dots, `b_pie`
# and `c_args` have neither, and a project is free to declare an option called
# `debug` of its own.
BUILTIN_OPTIONS = frozenset("""
prefix bindir datadir includedir infodir libdir libexecdir licensedir
localedir localstatedir mandir sbindir sharedstatedir sysconfdir
auto_features backend buildtype debug default_library default_both_libraries
errorlogs genvslite install_umask layout optimization prefer_static strip
unity unity_size warning_level werror wrap_mode force_fallback_for vsenv
pkgconfig.relocatable python.install_env python.platlibdir python.purelibdir
python.bytecompile python.allow_limited_api
b_asneeded b_colorout b_coverage b_lto b_lto_threads b_lundef b_ndebug b_pch
b_pgo b_pie b_sanitize b_staticpic b_thinlto_cache b_vscrt
c_args c_link_args cpp_args cpp_link_args c_std cpp_std cpp_eh cpp_rtti
c_winlibs cpp_winlibs
""".split())

FEATURE_VALUES = ("enabled", "disabled", "auto")
BOOLEAN_VALUES = ("true", "false")

MESON_SETUP = re.compile(r"meson\.py\s+setup(?=\s|$)")
DASH_D = re.compile(r"(?<!\S)-D([A-Za-z0-9_.:+-]+)=(\S*)")
OPTION_CALL = re.compile(r"\boption\s*\(")
OPTION_TYPE = re.compile(r"\btype\s*:\s*'([a-z]+)'")
OPTION_CHOICES = re.compile(r"\bchoices\s*:\s*\[(.*?)\]", re.S)
QUOTED = re.compile(r"'([^']*)'|\"([^\"]*)\"")


def option_bodies(text):
    """Yield the text between each `option(` and its matching `)`.

    Balanced parens with quote tracking, rather than a regex up to the next
    newline-and-paren. That shortcut reads ZERO options out of systemd's file,
    which closes the call at the end of its description line -- and a parser
    that silently finds nothing would report every option in the recipe as
    undeclared, which is a worse failure than the one it looks for.
    """
    for match in OPTION_CALL.finditer(text):
        index = match.end()
        depth, quote, start = 1, None, index
        while index < len(text) and depth:
            char = text[index]
            if quote:
                if text.startswith(quote, index):
                    index += len(quote)
                    quote = None
                    continue
                # A backslash escape only ever appears inside a description,
                # and skipping two characters is what keeps \' from closing it.
                index += 2 if char == "\\" else 1
                continue
            if char in "'\"":
                triple = text[index:index + 3]
                quote = triple if triple in ("'''", '"""') else char
                index += len(quote)
                continue
            if char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            index += 1
        if not depth:
            yield text[start:index - 1]


def parse_options(text):
    """Map an option file to {name: (kind, choices)}.

    A call with no `type:` is meson's own default, which is boolean -- the
    shape a project uses when it writes option('foo', value: true) and nothing
    else.
    """
    declared = {}
    for body in option_bodies(text):
        name = QUOTED.search(body)
        if not name:
            continue
        kind = OPTION_TYPE.search(body)
        choices = []
        listed = OPTION_CHOICES.search(body)
        if listed:
            choices = [q.group(1) if q.group(1) is not None else q.group(2)
                       for q in QUOTED.finditer(listed.group(1))]
        declared[name.group(1) if name.group(1) is not None else name.group(2)] = (
            kind.group(1) if kind else "boolean", choices
        )
    return declared


def option_problem(name, value, declared):
    """Why this -D is wrong at the pinned version, or None.

    The sentence says what the option IS rather than that it is not what was
    passed, because that is the half the reader does not have: every one of
    the seventeen looked right on the line it was written on.
    """
    if name not in declared:
        return "the pinned source declares no such option"
    kind, choices = declared[name]
    if kind == "feature" and value not in FEATURE_VALUES:
        return ("it is a feature, so meson takes only "
                + ", ".join(FEATURE_VALUES))
    if kind == "boolean" and value not in BOOLEAN_VALUES:
        return "it is a boolean, so meson takes only true or false"
    if kind == "combo" and choices and value not in choices:
        return "it is a combo, and its choices are " + ", ".join(choices)
    return None


def read_option_file(tarball, strip):
    """The pinned source's option file, as (name, text), or (None, None)."""
    with tarfile.open(tarball) as archive:
        for member in archive:
            parts = member.name.split("/")
            if len(parts) != strip + 1 or parts[-1] not in OPTION_FILES:
                continue
            handle = archive.extractfile(member)
            if handle is None:
                continue
            return parts[-1], handle.read().decode("utf-8", "replace")
    return None, None


def meson_options_of(text, dest):
    """Every -D on a `meson setup` line configuring `dest`, as (name, value).

    Scoped to the directory this tar line unpacked, so a build file with two
    meson members never checks one member's options against the other's
    tarball.
    """
    passed = []
    for line in text.splitlines():
        if line.strip().startswith("#") or not MESON_SETUP.search(line):
            continue
        if dest not in line.split():
            continue
        for match in DASH_D.finditer(line):
            name, value = match.group(1), match.group(2)
            # A subproject option (`sub:opt`) is the subproject's to declare,
            # and --wrap-mode=nodownload means this tree has no subprojects.
            if ":" in name or name in BUILTIN_OPTIONS:
                continue
            passed.append((name, value))
    return passed


def resolve(wanted, tarball, strip):
    """Stream the tarball, ticking off what was wanted; return what was not.

    Streaming with an early exit rather than getnames(): the passing case is
    every entry point found in the first handful of members, and the kernel's
    tarball is a gigabyte of xz that nothing here needs decompressed in full.
    Only a real miss pays for the whole listing, which is the right way round.

    A miss also collects the build systems the archive DOES have at its root.
    Saying only what is absent leaves the reader to go and find the tarball,
    which is the work this gate exists to save -- and the two findings that
    first came out of it were a meson recipe over an autotools source and an
    autotools recipe over a meson one, where the answer was in the listing
    already read.
    """
    outstanding = dict(wanted)
    found = set()
    with tarfile.open(tarball) as archive:
        for member in archive:
            parts = member.name.split("/")
            if len(parts) <= strip:
                continue
            if len(parts) == strip + 1 and parts[-1] in MARKERS:
                found.add(parts[-1])
            # rstrip("/") because tar names a directory member with a trailing
            # slash, and one of the things a recipe copies IS a directory:
            # meson's own `mesonbuild` package. Matching the prefix as well
            # covers the archives that carry no directory entries at all, where
            # `mesonbuild` exists only as the parent of its files.
            relative = "/".join(parts[strip:]).rstrip("/")
            for names in list(outstanding):
                if any(relative == name or relative.startswith(name + "/")
                       for name in names):
                    del outstanding[names]
            # No early exit once something is missing: the rest of the
            # listing is where the "instead" comes from.
            if not outstanding:
                break
    return outstanding, sorted(found)


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
        # meson's own recipe copies a DIRECTORY, mesonbuild, and an archive may
        # name it with a trailing slash, or not name it at all and carry only
        # the files under it. Both are the directory being there.
        ("a copied directory, present as its own entry",
         ["cp -a /build/src/x/mesonbuild /dest/usr/lib/meson/"],
         ["mesonbuild/", "meson.py"], 1, 0),
        ("a copied directory, present only as its contents",
         ["cp -a /build/src/x/mesonbuild /dest/usr/lib/meson/"],
         ["mesonbuild/__init__.py", "meson.py"], 1, 0),
        ("a copied directory that is genuinely not there",
         ["cp -a /build/src/x/mesonbuild /dest/usr/lib/meson/"],
         ["meson.py"], 1, 1),
    ]

    failures = []
    for name in sorted(DEFERRED):
        if not list(REPO.glob(f"recipes/*/{name}/build.yaml.in")):
            failures.append(
                f"DEFERRED names {name}, which has no recipe -- it was renamed "
                f"or removed and the entry was left behind"
            )

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

            missing, _ = resolve(wanted, tarball, 1)
            if len(missing) != expect_missing:
                failures.append(
                    f"{label}: {len(missing)} missing, expected "
                    f"{expect_missing}: {list(missing)}"
                )

    # The option half, exercised on text rather than on tarballs: what it
    # reads is a file, and the tar plumbing above is already covered. Every
    # case here is a shape that was actually met -- systemd's closing paren,
    # GNOME's triple-quoted descriptions, fwupd's name on its own line.
    systemd_shaped = (
        "option('nspawn', type : 'feature', value : 'auto',\n"
        "       description : 'install systemd-nspawn')\n"
        "option('status-unit-format-default', type : 'combo',\n"
        "       choices : ['name', 'description', 'combined'],\n"
        "       description : 'use unit name or description by default')\n"
        "option('machined', type : 'boolean', value : true,\n"
        "       description : 'install systemd-machined')\n"
    )
    declared = parse_options(systemd_shaped)
    option_cases = [
        ("three options read from a systemd-shaped file",
         len(declared) == 3),
        ("a feature is read as a feature",
         declared.get("nspawn") == ("feature", [])),
        ("a combo keeps its choices",
         declared.get("status-unit-format-default")
         == ("combo", ["name", "description", "combined"])),
        ("a boolean beside them is still a boolean",
         declared.get("machined")[0] == "boolean"),
        # The regression this parser exists for: a description ending the line
        # the call closes on. A regex to the next newline-and-paren reads zero
        # options here and would then call every -D in the recipe undeclared.
        ("a call closing on its description line is not skipped",
         "machined" in declared),
        ("a feature given true is caught",
         option_problem("nspawn", "true", declared) is not None),
        ("a feature given enabled passes",
         option_problem("nspawn", "enabled", declared) is None),
        ("a boolean given enabled is caught",
         option_problem("machined", "enabled", declared) is not None),
        ("a combo outside its choices is caught",
         option_problem("status-unit-format-default", "combined2", declared)
         is not None),
        ("a combo inside its choices passes",
         option_problem("status-unit-format-default", "combined", declared)
         is None),
        ("an option the source does not declare is caught",
         option_problem("nscd", "false", declared) is not None),
    ]
    triple = parse_options(
        "option('docs', type: 'feature',\n"
        "  description: '''build the documentation,\n"
        "which needs gi-docgen (see README)''')\n"
        "option('tests', value: false)\n"
    )
    option_cases += [
        ("a triple-quoted description does not swallow the next call",
         set(triple) == {"docs", "tests"}),
        ("a call with no type: is boolean, which is meson's default",
         triple.get("tests") == ("boolean", [])),
    ]
    line = ("  - python3 /x/meson.py setup /build/b/p /build/src/p "
            "--cross-file @RECIPE@/cross.ini --prefix=/usr "
            "-Dtests=false -Dintrospection=disabled -Dc_args=-O2 "
            "-Dglib:werror=false")
    passed = dict(meson_options_of(line, "/build/src/p"))
    option_cases += [
        ("the -D options are read off the setup line",
         set(passed) == {"tests", "introspection"}),
        # c_args is meson's own and glib:werror is a subproject's; neither is
        # declared in this project's option file, so checking them against it
        # would report two failures that are not there.
        ("a built-in and a subproject option are left alone",
         "c_args" not in passed and "glib:werror" not in passed),
        ("a setup line for another member's directory is not read",
         meson_options_of(line, "/build/src/other") == []),
        ("a commented-out setup line is not read",
         meson_options_of("  # " + line.strip(), "/build/src/p") == []),
    ]
    for label, ok in option_cases:
        if not ok:
            failures.append(f"{label}: expected this to hold and it does not")

    if failures:
        print("recipe-entrypoints --self-test: FAILED", file=sys.stderr)
        for failure in failures:
            print("  " + failure, file=sys.stderr)
        return 1
    print(f"recipe-entrypoints: self-test green, {len(cases)} entry-point "
          f"case(s) and {len(option_cases)} option case(s)")
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

    failures, deferred, missing_bytes = [], [], []
    checked, options_checked = 0, 0
    # Which deferred recipes were actually looked at, and which still failed.
    # An entry that stops failing has to say so: a deferred finding nobody
    # removes is indistinguishable from a gate that was quietly switched off.
    seen_deferred, still_failing = set(), set()

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

            if name in DEFERRED:
                seen_deferred.add(name)

            # The second question of the same bytes, before the entry-point
            # one because the `continue`s below are for a recipe with nothing
            # left to look for -- and a meson recipe whose meson.build an
            # earlier step created still has sixteen options to get wrong.
            #
            # Skipped for a deferred recipe: those are recipes driving the
            # wrong build system entirely, so their options belong to a file
            # that is not in this tarball and every one of them would be
            # reported as undeclared.
            if name not in DEFERRED:
                passed = meson_options_of(text, dest)
                if passed:
                    where, declaration = read_option_file(tarball, strip)
                    declared = parse_options(declaration) if where else {}
                    if not declared:
                        had = (f"ships {where} and this gate read no option "
                               f"out of it" if where else
                               f"has neither {' nor '.join(OPTION_FILES)}")
                        failures.append(
                            f"{template.relative_to(REPO)}: the recipe passes "
                            f"{len(passed)} -D option(s), and the pinned "
                            f"tarball {had}.\n    {entry['url']}"
                        )
                    else:
                        options_checked += len(passed)
                        for option, value in passed:
                            why = option_problem(option, value, declared)
                            if why:
                                failures.append(
                                    f"{template.relative_to(REPO)}: "
                                    f"-D{option}={value} -- {why}.\n"
                                    f"    Read {where} in {entry['url']}"
                                )

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
            missing, found = resolve(wanted, tarball, strip)
            for names, (verb, why) in missing.items():
                spelling = " or ".join(names)
                instead = (
                    f"    It does ship, at its root: {', '.join(found)}\n"
                    if found else
                    "    It ships no build system this gate recognises at its "
                    "root, so read the listing yourself.\n"
                )
                report = (
                    f"{template.relative_to(REPO)}: the recipe {verb} `{why}`, "
                    f"and the pinned tarball has no {spelling}.\n"
                    f"    {entry['url']}\n"
                    + instead
                )
                if name in DEFERRED:
                    still_failing.add(name)
                    deferred.append(report + f"    {DEFERRED[name]}")
                else:
                    failures.append(
                        report +
                        f"    Either the source moved to another build system "
                        f"or it never had this one. Translate the options "
                        f"rather than transcribing them."
                    )

    # Printed before the verdict either way, so a deferred finding is read
    # rather than scrolled past on a green run.
    if deferred:
        print("recipe-entrypoints: deferred, waiting on a package this tree "
              "does not build yet:")
        for item in deferred:
            print("  " + item)

    for name in sorted(seen_deferred - still_failing):
        failures.append(
            f"{name}: deferred in tools/gates/recipe-entrypoints.py, and its "
            f"entry point is now present.\n"
            f"    Whatever it was waiting for has landed. Delete the DEFERRED "
            f"entry and the paragraph in docs/limits.md that goes with it -- a "
            f"hole this tree no longer has is one it must stop claiming."
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
    held = f", {len(deferred)} deferred" if deferred else ""
    print(f"recipe-entrypoints: {checked} build-system entry point(s) "
          f"present, {options_checked} meson option(s) declared at the pinned "
          f"version{held}{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
