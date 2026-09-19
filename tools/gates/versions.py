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

One thing this gate deliberately does not check, and is not going to, is the
options a recipe passes to its build system. A version bump moves those too,
and an option name or value kind can only be checked against the
`meson_options.txt` of the tarball itself -- which the gates, running with no
sources and no network, do not have. That gap is real and has bitten once:
fwupd's recipe passed `-Dintrospection=false`, but fwupd 2.0.3 declares that
option `type: 'feature'`, which accepts only `enabled`, `disabled` or `auto`.
`false` is valid syntax for a boolean option, so the mistake looked right next
to the thirty others on the same line, and nothing said otherwise until
`meson setup` ran in a real build and died before one object compiled.

So a wrong option is invisible to `./do check`, and therefore invisible to the
per-candidate legs in .github/workflows/update-sources.yml, which run exactly
that. A bump crossing a major version needs a human and a release note; the
matrix proves the tarball exists and the tree still lints, and claims no more.
"""

import pathlib
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


def check(repo):
    """Return (failures, checked) for the tree rooted at `repo`."""
    lock = yaml.safe_load((repo / "manifest" / "sources.lock").read_text()) or {}
    failures = []
    checked = 0

    for template in sorted((repo / "recipes").glob("*/*/build.yaml.in")):
        text = template.read_text()
        keys = set(URL_TOKEN.findall(text))
        if not keys:
            continue
        missing = sorted(key for key in keys if lock.get(key) is None)
        if missing:
            for key in missing:
                failures.append(
                    f"{template.relative_to(repo)}: @URL_{key}@ is not in sources.lock"
                )
            continue

        # A recipe that downloads several sources usually has no single
        # upstream version to agree with; the kernel's bpftool is the shape
        # this would be, if it were not built from the kernel's own tree.
        # Skipped rather than failed, and the self-test pins that: a rule that
        # guessed which of several sources "the" version came from would be
        # wrong the first time someone reordered them.
        #
        # Unless they all carry the same version, which is not a guess. That
        # is compiler-rt: its sources are two tarballs cut from one LLVM
        # release, and skipping it would drop the check from the one recipe in
        # the tree that is version-locked on purpose (HOLD in tools/check-latest)
        # -- exactly where a silent drift costs most.
        versions = {str(lock[key].get("version", "")) for key in keys}
        if len(versions) > 1:
            continue

        key = sorted(keys)[0]
        entry = lock[key]

        declared = recipe_version(text)
        if declared is None:
            failures.append(f"{template.relative_to(repo)}: no `version:` list")
            continue

        upstream = str(entry.get("version", "")).split(".")
        if declared != upstream:
            named = key if len(keys) == 1 else " and ".join(sorted(keys))
            failures.append(
                f"{template.relative_to(repo)}: version {'.'.join(declared)} "
                f"but {named} is {entry.get('version')}"
            )
            continue

        checked += 1

    return failures, checked


# --- self-test ------------------------------------------------------------
#
# A gate that has never been shown to fail is a gate nobody should trust. Each
# case below builds a throwaway tree, runs the real check over it, and asserts
# the verdict -- including the two cases where the right answer is to say
# nothing, which are the ones a stricter rule would get wrong.

def _tree(root, lock, recipes):
    """Write a minimal tree: one sources.lock and some build.yaml.in files."""
    (root / "manifest").mkdir(parents=True)
    (root / "manifest" / "sources.lock").write_text(lock)
    for name, body in recipes.items():
        path = root / "recipes" / "layer" / name
        path.mkdir(parents=True)
        (path / "build.yaml.in").write_text(body)
    return root


def _recipe(version_items, urls):
    lines = ["name: pkg", "version:"]
    lines += [f"- '{item}'" for item in version_items]
    lines += ["dependencies: []", "steps:", "- stage: Prepare", "  name: fetch", "  dl_urls:"]
    lines += [f"    @URL_{key}@: '@SHA256_{key}@'" for key in urls]
    lines += ["  run:", "  - true", ""]
    return "\n".join(lines)


def self_test():
    import tempfile

    cases = []

    def case(name, lock, recipes, expect_failure, because):
        cases.append((name, lock, recipes, expect_failure, because))

    case(
        "agreeing recipe passes",
        "FOO:\n  url: https://example.invalid/foo-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n",
        {"foo": _recipe(["1", "2", "3"], ["FOO"])},
        False,
        "1.2.3 split on dots is ['1','2','3']",
    )
    case(
        "stale recipe fails",
        "FOO:\n  url: https://example.invalid/foo-2.0.0.tar.xz\n  version: '2.0.0'\n  sha256: TODO\n",
        {"foo": _recipe(["1", "2", "3"], ["FOO"])},
        True,
        "the bump moved sources.lock and left the recipe behind -- the whole point",
    )
    case(
        "missing lock entry fails",
        "BAR:\n  url: https://example.invalid/bar-1.0.tar.xz\n  version: '1.0'\n  sha256: TODO\n",
        {"foo": _recipe(["1", "2", "3"], ["FOO"])},
        True,
        "a recipe naming a key nobody pinned cannot be checked, and silence would hide it",
    )
    case(
        "multi-source recipe is skipped, not failed",
        ("FOO:\n  url: https://example.invalid/foo-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n"
         "BAZ:\n  url: https://example.invalid/baz-9.9.tar.xz\n  version: '9.9'\n  sha256: TODO\n"),
        {"foo": _recipe(["7", "7", "7"], ["FOO", "BAZ"])},
        False,
        "two sources, no single upstream version to agree with; guessing one would be wrong",
    )
    case(
        "multi-source recipes that agree ARE checked",
        ("FOO:\n  url: https://example.invalid/foo-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n"
         "BAZ:\n  url: https://example.invalid/baz-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n"),
        {"foo": _recipe(["1", "2", "3"], ["FOO", "BAZ"])},
        False,
        "one release split across two tarballs, which is compiler-rt's shape",
    )
    case(
        "and drift against them is caught",
        ("FOO:\n  url: https://example.invalid/foo-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n"
         "BAZ:\n  url: https://example.invalid/baz-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n"),
        {"foo": _recipe(["1", "2", "2"], ["FOO", "BAZ"])},
        True,
        "the skip used to hide this, which is the whole reason for the agreeing case",
    )
    case(
        "a missing lock entry beside a present one still fails",
        "FOO:\n  url: https://example.invalid/foo-1.2.3.tar.xz\n  version: '1.2.3'\n  sha256: TODO\n",
        {"foo": _recipe(["1", "2", "3"], ["FOO", "QUX"])},
        True,
        "an unpinned source must not be excused by a pinned sibling",
    )
    case(
        "trailing zero is not truncated",
        "FOO:\n  url: https://example.invalid/foo-2.70.tar.xz\n  version: '2.70'\n  sha256: TODO\n",
        {"foo": _recipe(["2", "70"], ["FOO"])},
        False,
        "unquoted, YAML would read 2.70 as the float 2.7 and this would fail",
    )
    case(
        "unquoted trailing zero IS caught",
        "FOO:\n  url: https://example.invalid/foo-2.70.tar.xz\n  version: 2.70\n  sha256: TODO\n",
        {"foo": _recipe(["2", "70"], ["FOO"])},
        True,
        "which is what makes the quoting in sources.lock load-bearing rather than style",
    )
    case(
        "a single-component version works",
        "FOO:\n  url: https://example.invalid/foo-39.tar.xz\n  version: '39'\n  sha256: TODO\n",
        {"foo": _recipe(["39"], ["FOO"])},
        False,
        "not every upstream uses dots",
    )
    case(
        "a recipe with no sources is skipped",
        "FOO:\n  url: https://example.invalid/foo-1.0.tar.xz\n  version: '1.0'\n  sha256: TODO\n",
        {"own": "name: own\nversion:\n- '0'\ndependencies: []\nsteps: []\n"},
        False,
        "this tree's own packages have no upstream version to agree with",
    )

    ok = True
    for name, lock, recipes, expect_failure, because in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = _tree(pathlib.Path(tmp), lock, recipes)
            failures, _ = check(root)
            got = bool(failures)
            if got != expect_failure:
                ok = False
                print(f"  MISSED  {name}", file=sys.stderr)
                print(f"          expected {'a failure' if expect_failure else 'no failure'}, "
                      f"got {failures or 'none'}", file=sys.stderr)
            else:
                print(f"  ok      {name:44} ({because})")

    # And the real tree, which is the case that matters in practice.
    failures, checked = check(REPO)
    if failures:
        ok = False
        print(f"  MISSED  the repository's own tree: {failures}", file=sys.stderr)
    else:
        print(f"  ok      the repository's own tree{'':21} ({checked} recipes)")

    print("versions --self-test: " + ("all cases behave" if ok else "FAILED"))
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return self_test()

    failures, checked = check(REPO)

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
