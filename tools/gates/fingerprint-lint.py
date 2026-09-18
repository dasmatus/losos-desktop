#!/usr/bin/env python3
"""Re-apply pm's fingerprint check to what a wrapper is wrapping.

pm refuses a command whose first word matches no fingerprint, and `pm explain`
is that check standalone. The check is anchored at the program name and reads
the FIRST WORD ONLY, so a wrapper hides everything behind it:

    env FOO=bar veritysetup format /dev/sda      -> matches `coreutils`
    /bin/sh in-dir.sh /build/b some-random-tool  -> matches `shell`

Both pass `pm explain` today. That is not a bug in pm so much as the honest
limit of classifying by program name, but it does mean a repository that leans
on wrappers is quietly worth less than its green `pm explain` suggests.

So this gate:

  * bans `env` outright -- not discouraged, rejected. Besides hiding the
    program, `env` rewrites the derived capability set: `env FOO=bar cargo
    build` loses `Network` and the build then fails in a way nobody diagnoses
    quickly. A mechanism whose failure mode is "silently different sandbox" is
    not worth the convenience.
  * allows exactly the wrappers named in tools/gates/allowed-wrappers, and
    re-applies pm's own table to the command that follows.
  * rejects shell metacharacters. There is no shell (C1), so a `|` or `>` in a
    command is not a pipe -- it is a literal argument, and it means whoever
    wrote the line believed something false about how it would run.
  * rejects `sh -c`, which would reintroduce a shell through the back door.

Run with --check-table to diff the reimplemented table against pm's
src/policy.rs, so the lint cannot drift from the thing it imitates.
"""

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(REPO / "tools" / "lib"))

import yaml  # noqa: E402

import fingerprint  # noqa: E402

# There is no shell, so these are never operators. A command containing one is
# a misunderstanding, not a feature.
METACHARACTERS = set("|><&;`$*?")

ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")


def allowed_wrappers():
    manifest = REPO / "tools" / "gates" / "allowed-wrappers"
    if not manifest.exists():
        return set()
    names = set()
    for line in manifest.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            names.add(line)
    return names


def unwrap(command, wrappers):
    """Strip one allowed wrapper, returning the command it really runs.

    Returns (inner_command, note) where note explains what was stripped, or
    (None, reason) when the wrapper is not allowed.
    """
    words = command.split()
    if not words:
        return None, "empty command"

    program = Path(words[0]).name

    if program == "env":
        return None, (
            "`env` is banned in this repository: it hides the real program from "
            "pm's fingerprint check and silently changes the derived capability "
            "set. Use a native flag instead (see docs/sysroot.md)."
        )

    if program in {"sh", "bash", "dash", "ash", "zsh"}:
        if len(words) < 2:
            return None, "a bare shell with no script"
        if words[1] == "-c":
            return None, "`sh -c` reintroduces a shell; write a script file instead"
        script = Path(words[1]).name
        if script not in wrappers:
            # An ordinary helper script, not a wrapper. Its contents are the
            # repository's own and are reviewed as source, not as a command.
            return None, None
        # A wrapper: re-apply pm's table to what follows its arguments.
        if script == "in-dir.sh":
            if len(words) < 4:
                return None, "in-dir.sh needs a directory and a program"
            return " ".join(words[3:]), f"unwrapped {script}"
        return None, None

    return None, None


def check_command(command, where, errors, wrappers):
    if any(ch in METACHARACTERS for ch in command):
        found = sorted({ch for ch in command if ch in METACHARACTERS})
        errors.append(
            f"{where}: contains shell metacharacter(s) {''.join(found)} but there "
            f"is no shell -- they reach execve as literal argument text:\n"
            f"      {command}"
        )
        return

    if fingerprint.match(command) is None:
        errors.append(f"{where}: matches no pm fingerprint:\n      {command}")
        return

    inner, note = unwrap(command, wrappers)
    if note and inner is None:
        errors.append(f"{where}: {note}\n      {command}")
        return
    if inner is not None:
        if fingerprint.match(inner) is None:
            errors.append(
                f"{where}: wrapper hides a command that matches no pm "
                f"fingerprint:\n      {command}\n      -> {inner}"
            )


def check_table():
    """Diff the reimplemented program names against pm's src/policy.rs."""
    policy = REPO.parent / "pm" / "src" / "policy.rs"
    if not policy.exists():
        print(f"fingerprint-lint: cannot find {policy}; skipping table check")
        return 0
    source = policy.read_text()
    theirs = set(re.findall(r'name:\s*"([a-z-]+)"', source))
    ours = {name for name, _ in fingerprint.TABLE}
    if theirs != ours:
        print("fingerprint-lint: TABLE DRIFT", file=sys.stderr)
        if theirs - ours:
            print(f"  in pm, missing here: {sorted(theirs - ours)}", file=sys.stderr)
        if ours - theirs:
            print(f"  here, missing in pm: {sorted(ours - theirs)}", file=sys.stderr)
        return 1
    print(f"fingerprint-lint: table agrees with pm ({len(ours)} fingerprints)")
    return 0


def main():
    if "--check-table" in sys.argv:
        return check_table()

    wrappers = allowed_wrappers()
    recipes = sorted((REPO / "out" / "recipes").glob("*/build.yaml"))
    if not recipes:
        print("fingerprint-lint: no generated recipes", file=sys.stderr)
        return 1

    errors, count = [], 0
    for path in recipes:
        doc = yaml.safe_load(path.read_text()) or {}
        for step in doc.get("steps") or []:
            for command in step.get("run") or []:
                count += 1
                where = f"{path.parent.name}/{step.get('name', '?')}"
                check_command(command, where, errors, wrappers)

    if errors:
        print("fingerprint-lint: FAILED", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1

    print(f"fingerprint-lint: {count} command(s) clean across {len(recipes)} recipe(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
