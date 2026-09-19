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

pm's table is not the whole answer any more. A plugin can name a command pm
would otherwise refuse -- that is how this repository reaches `mkosi`,
`xorriso` and `qemu-img` -- so the lint has to know what the plugins recognise
as well, or it rejects a build file pm accepts. Those names live in exactly one
place, the `const` arrays in `plugins/*/src/lib.rs` marked `pm-fingerprints`,
and are read from there rather than restated here: a second list would be a
list that goes stale, and the failure would be a lint that refuses a build pm
runs happily.
"""

import re
from pathlib import Path

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


_PLUGINS = Path(__file__).resolve().parent.parent.parent / "plugins"

# A `const NAME: &[&str] = &[ ... ];` whose doc comment carries the marker.
_MARKED_LIST = re.compile(
    r"///\s*pm-fingerprints\b.*?const\s+\w+\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];",
    re.S,
)
_LITERAL = re.compile(r'"([^"]+)"')


def plugin_programs() -> dict[str, str]:
    """Program name -> plugin name, for every marked list in plugins/.

    Read rather than restated. If a plugin stops recognising a program, the
    lint stops accepting it in the same commit, which is the only way the two
    can be made to agree without a human remembering.
    """
    found = {}
    for source in sorted(_PLUGINS.glob("*/src/lib.rs")):
        plugin = source.parent.parent.name
        for body in _MARKED_LIST.findall(source.read_text()):
            for program in _LITERAL.findall(body):
                found.setdefault(program, plugin)
    return found


def match(command: str) -> str | None:
    """The fingerprint pm would assign to `command`, or None.

    A plugin's answer is returned as `<plugin>:<program>`, which is how pm
    records one, so a caller can always tell a built-in from an extension.
    """
    command = command.strip()
    if not command:
        return None
    for name, regex in _COMPILED:
        if regex.match(command):
            return name
    # Only after the built-in table, exactly as pm does it: pm consults a
    # plugin about a command that matched NOTHING, so a plugin can never
    # reclassify a command pm already understands.
    program = Path(command.split()[0]).name
    plugin = plugin_programs().get(program)
    return f"{plugin}:{program}" if plugin else None
