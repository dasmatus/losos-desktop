"""pm's fingerprint table, reimplemented so this repo can lint past it.

pm derives a build's sandbox policy by matching each command against a built-in
table and refusing anything unmatched (C2). The pattern is anchored at the
program name and reads the FIRST WORD ONLY, which leaves a hole: `env FOO=bar
meson setup ...` matches `coreutils` on `env`, and `meson` is never examined.

This repo bans `env` and permits exactly one wrapper, `share/in-dir.sh`. The
lint in tools/gates/fingerprint-lint.py uses the table below to re-apply pm's
own check to whatever the wrapper is wrapping, so a wrapper cannot be used to
smuggle an unfingerprinted program past `pm explain`.

The patterns mirror pm's src/policy.rs. `tools/gates/fingerprint-lint.py
--check-table` diffs the program names below against that file, so the two
cannot drift apart silently.
"""

import re

# `program!` in pm: an optional leading path that must end in '/', the
# alternation, and a word terminator. The leading-path group is what lets
# `/bin/sh` and `./configure` match while `evilmake` does not match `make`.
_PROGRAM = r"^(?:[\w.+/-]*/)?(?:%s)(?:\s|$)"
# `prefixed_program!`: as above, plus up to four `word-` groups, so
# `x86_64-linux-gnu-gcc` matches wherever `gcc` does.
_PREFIXED = r"^(?:[\w.+/-]*/)?(?:[A-Za-z0-9_]+-){0,4}(?:%s)(?:\s|$)"

# Order is precedence, exactly as in pm: the first match wins, so the
# catch-all `coreutils` entry comes after the specific ones.
TABLE = [
    ("make", _PROGRAM % (r"g?make")),
    ("configure", _PROGRAM % (r"configure")),
    ("cmake", _PROGRAM % (r"cmake|ctest|cpack")),
    ("ninja", _PROGRAM % (r"ninja|samu")),
    ("meson", _PROGRAM % (r"meson")),
    ("cargo", _PROGRAM % (r"cargo|rustc")),
    ("go", _PROGRAM % (r"go")),
    ("node", _PROGRAM % (r"npm|yarn|pnpm|npx|node")),
    ("pip", _PROGRAM % (r"pip[23]?")),
    ("python", _PROGRAM % (r"python[23]?(?:\.\d+)?")),
    ("pkg-config", _PROGRAM % (r"pkg-config|pkgconf")),
    ("compiler", _PREFIXED % (r"(?:cc|c\+\+|gcc|g\+\+|clang|clang\+\+)(?:-\d+(?:\.\d+)*)?")),
    ("ld", _PREFIXED % (r"ld|ld\.bfd|ld\.gold|ld\.lld|lld|ar|ranlib|nm|strip|objcopy")),
    (
        "coreutils",
        _PROGRAM % (
            r"install|cp|mv|rm|mkdir|rmdir|chmod|chown|ln|ls|cat|echo|printf|touch|true|"
            r"false|test|pwd|env|mktemp|sed|awk|gawk|grep|find|xargs|sort|head|tail|cut|"
            r"tr|sync|patch"
        ),
    ),
    ("shell", _PROGRAM % (r"sh|bash|dash|ash|zsh")),
    (
        "archive",
        _PROGRAM % (
            r"tar|unzip|zip|xz|unxz|gzip|gunzip|bzip2|bunzip2|zstd|unzstd|7z|cpio"
        ),
    ),
    ("git", _PROGRAM % (r"git")),
]

_COMPILED = [(name, re.compile(pattern)) for name, pattern in TABLE]

# Capabilities that grant the jail a shared host network namespace (C8).
NETWORK_FINGERPRINTS = {"cargo", "go", "node", "pip", "git"}


def match(command: str) -> str | None:
    """The fingerprint pm would assign to `command`, or None."""
    command = command.strip()
    if not command:
        return None
    for name, regex in _COMPILED:
        if regex.match(command):
            return name
    return None
