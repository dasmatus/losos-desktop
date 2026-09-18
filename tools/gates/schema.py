#!/usr/bin/env python3
"""Validate generated build files against pm's schema, without running pm.

pm's own errors for a malformed build file are serde's, and serde's errors
describe types rather than intent: an unquoted version component reports an
invalid type at a line number and never mentions quoting, which is the actual
problem in every case it happens. This runs first and says what is wrong.

It is also the only gate that needs neither pm nor a signature, so it is the
one that still works when the tree is mid-edit.
"""

import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent
STAGES = {"Prepare", "Build", "Install", "Test"}
REQUIRED = {"name", "version", "dependencies", "steps"}


def check(path, errors):
    try:
        doc = yaml.safe_load(path.read_text())
    except yaml.YAMLError as exc:
        errors.append(f"{path}: not valid YAML: {exc}")
        return

    if not isinstance(doc, dict):
        errors.append(f"{path}: top level is not a mapping")
        return

    missing = REQUIRED - doc.keys()
    if missing:
        errors.append(f"{path}: missing required field(s): {', '.join(sorted(missing))}")
        return

    extra = doc.keys() - REQUIRED
    if extra:
        errors.append(f"{path}: unknown field(s): {', '.join(sorted(extra))}")

    # The single most common way to break a recipe. serde takes `version` as a
    # Vec<String>; an unquoted 1.23 arrives as a float and parsing dies.
    version = doc.get("version")
    if not isinstance(version, list) or not version:
        errors.append(f"{path}: version must be a non-empty list")
    else:
        for part in version:
            if not isinstance(part, str):
                errors.append(
                    f"{path}: version component {part!r} is "
                    f"{type(part).__name__}, not a string -- quote it"
                )

    deps = doc.get("dependencies")
    if not isinstance(deps, list):
        errors.append(f"{path}: dependencies must be a list")
    else:
        for dep in deps:
            # Dependency paths resolve against the PROCESS working directory,
            # which for this repo is out/pkgs (C11), so that is where they are
            # checked from.
            target = (REPO / "out" / "pkgs" / dep).resolve()
            if not target.exists():
                errors.append(f"{path}: dependency does not exist: {dep}")

    steps = doc.get("steps")
    if not isinstance(steps, list):
        errors.append(f"{path}: steps must be a list")
        return

    for index, step in enumerate(steps):
        where = f"{path}: step {index}"
        if not isinstance(step, dict):
            errors.append(f"{where}: not a mapping")
            continue
        # serde does not apply the Rust default for `stage`, so an omitted one
        # is an error rather than a Prepare.
        stage = step.get("stage")
        if stage not in STAGES:
            errors.append(
                f"{where} ({step.get('name', '?')}): stage is {stage!r}, "
                f"must be one of {', '.join(sorted(STAGES))}"
            )
        if not isinstance(step.get("name"), str) or not step.get("name"):
            errors.append(f"{where}: name must be a non-empty string")
        run = step.get("run")
        if not isinstance(run, list) or not all(
            isinstance(c, str) and c.strip() for c in run
        ):
            errors.append(f"{where}: run must be a list of non-empty strings")
        urls = step.get("dl_urls", None)
        if urls is not None and not isinstance(urls, dict):
            errors.append(f"{where}: dl_urls must be a mapping or null")


def main():
    recipes = sorted((REPO / "out" / "recipes").glob("*/build.yaml"))
    if not recipes:
        print("schema: no generated recipes; run tools/configure first",
              file=sys.stderr)
        return 1

    errors = []
    for path in recipes:
        check(path, errors)

    if errors:
        print("schema: FAILED", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1

    print(f"schema: {len(recipes)} build file(s) valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())
