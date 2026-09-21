"""Host-side pm extension for this repository's toolchain configuration.

This is the command-line entrypoint and shared library behind two things the
repository already had separately: configuring generated recipes from
manifest/toolchain.yaml and reporting what that manifest actually compiles and
links on the host.
"""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from toolchain import Toolchain

REPO = Path(__file__).resolve().parent.parent.parent

LIB_C = """
#include <stdio.h>
__attribute__((visibility("default"))) int lib_add(int a, int b) { return a + b; }
__attribute__((visibility("default"))) void lib_say(void) { puts("lib"); }
"""

MAIN_C = """
#include <stdio.h>
extern int lib_add(int, int);
extern void lib_say(void);

int main(void) {
    int (*fp)(int, int) = lib_add;
    lib_say();
    printf("%d\\n", fp(2, 3));
    return 0;
}
"""

VIOLATION_C = """
#include <stdio.h>
extern int lib_add(int, int);

typedef long (*wrong_t)(long, long, long);

int main(void) {
    wrong_t bad = (wrong_t)(void *)lib_add;
    printf("%ld\\n", bad(1, 2, 3));
    return 0;
}
"""


def configured_toolchain(*, repo=REPO, triple=None, cfi_export_map=None):
    """Load manifest/toolchain.yaml and apply this caller's configuration."""
    tc = Toolchain.load(repo / "manifest" / "toolchain.yaml")
    if triple is not None:
        tc.target["triple"] = triple
    if cfi_export_map is not None:
        tc.cfi_export_map = Path(cfi_export_map)
    return tc


def run(cmd, **kwargs):
    return subprocess.run(cmd, capture_output=True, text=True, **kwargs)


def host_flags(flags):
    """The flag set minus the cross-target parts."""
    dropped = ("--target=", "--sysroot=", "-resource-dir=", "--unwindlib=")
    return [f for f in flags if not f.startswith(dropped)]


def linker_in_use(tc, notes):
    """Ask the driver which linker `-fuse-ld=` actually resolved to."""
    cc = tc.compiler.get("cc", "clang")
    result = run(
        [
            cc,
            *host_flags(tc.ldflags()),
            "-nostdlib",
            "-shared",
            "-x",
            "c",
            "-",
            "-Wl,--version",
            "-o",
            "/dev/null",
        ],
        input="int x;\n",
    )
    if result.returncode != 0:
        notes.append(
            "the driver could not start its linker:\n" + result.stderr.strip()
        )
        return None
    banner = (result.stdout.splitlines() or [""])[0].strip()
    if not banner:
        notes.append("the linker printed no version banner")
        return None
    return banner


def compile_probe(tc, work, notes):
    """Build lib + exe with the real flags. Returns True on success."""
    cc = tc.compiler.get("cc", "clang")
    cflags = host_flags(tc.cflags())
    ldflags = host_flags(tc.ldflags())

    (work / "lib.c").write_text(LIB_C)
    (work / "main.c").write_text(MAIN_C)

    result = run(
        [cc, *cflags, *ldflags, "-shared", "-o", str(work / "libprobe.so"),
         str(work / "lib.c")]
    )
    if result.returncode != 0:
        notes.append("shared library did not build:\n" + result.stderr.strip())
        return False

    result = run(
        [cc, *cflags, *ldflags, "-o", str(work / "probe"), str(work / "main.c"),
         "-L", str(work), "-lprobe", "-Wl,-rpath," + str(work)]
    )
    if result.returncode != 0:
        notes.append("executable did not link:\n" + result.stderr.strip())
        return False

    result = run([str(work / "probe")])
    if result.returncode != 0 or result.stdout.strip() != "lib\n5".strip():
        notes.append(f"probe ran wrong: rc={result.returncode} out={result.stdout!r}")
        return False
    return True


def verify_cfi_live(tc, work, notes):
    """Prove the CFI checks actually fire, not merely that they compiled."""
    cc = tc.compiler.get("cc", "clang")
    cflags = host_flags(tc.cflags())
    ldflags = host_flags(tc.ldflags())

    (work / "violation.c").write_text(VIOLATION_C)
    result = run(
        [cc, *cflags, *ldflags, "-o", str(work / "violation"),
         str(work / "violation.c"), "-L", str(work), "-lprobe",
         "-Wl,-rpath," + str(work)]
    )
    if result.returncode != 0:
        notes.append("violation probe did not build:\n" + result.stderr.strip())
        return False

    result = run([str(work / "violation")])
    combined = (result.stdout + result.stderr).lower()
    if "control flow integrity" in combined or "cfi" in combined:
        return True
    if result.returncode < 0:
        notes.append(f"trapped with signal {-result.returncode} (trap mode)")
        return True
    notes.append(
        "the bad indirect call was NOT caught -- CFI compiled in but is not "
        f"checking (rc={result.returncode}, output={result.stdout.strip()!r})"
    )
    return False


def audit_recipes(repo, notes):
    """No recipe may spell its own optimisation or sanitizer flags."""
    forbidden = re.compile(r"(?:^|\s)-(?:O[0-9zs]|flto|fsanitize|fvisibility)\b")
    offenders = []
    for template in sorted((repo / "recipes").glob("*/*/build.yaml.in")):
        for number, line in enumerate(template.read_text().splitlines(), 1):
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            if forbidden.search(line):
                offenders.append(f"{template.relative_to(repo)}:{number}: {stripped}")
    if offenders:
        notes.extend(offenders)
        return False
    return True


def report(argv, *, repo=REPO):
    parser = argparse.ArgumentParser(prog="pm-toolchain report")
    parser.add_argument("--verify-cfi", action="store_true")
    args = parser.parse_args(argv)

    tc = configured_toolchain(repo=repo)

    print("toolchain-report")
    print(f"  compiler     {tc.compiler.get('cc')} / {tc.compiler.get('linker')}")
    print(f"  target       {tc.target.get('triple')}")
    print(f"  LTO          {tc.lto.get('mode')}")
    print(
        f"  CFI          {'on' if tc.cfi.get('enable') else 'off'}"
        f"  cross-DSO={tc.cfi.get('cross_dso')}"
        f"  trap={tc.cfi.get('trap')}"
    )
    print(f"  schemes      {', '.join(tc.cfi.get('schemes') or []) or 'none'}")

    if tc.exceptions:
        print(f"  exceptions   {len(tc.exceptions)} package(s) outside the full set:")
        for name, entry in tc.exceptions.items():
            print(f"    {name:12} drops {', '.join(entry.get('drops') or [])}")
    else:
        print("  exceptions   none")

    ok = True

    notes = []
    banner = linker_in_use(tc, notes)
    if banner:
        print(f"  linker       {banner}")
    else:
        print("  linker       FAILED", file=sys.stderr)
        for note in notes:
            print("    " + note.replace("\n", "\n    "), file=sys.stderr)
        ok = False

    with tempfile.TemporaryDirectory(prefix="losos-toolchain-") as tmp:
        work = Path(tmp)

        notes = []
        if compile_probe(tc, work, notes):
            print("  build        lib + exe compiled, linked and ran with the real flags")
        else:
            print("  build        FAILED", file=sys.stderr)
            for note in notes:
                print("    " + note.replace("\n", "\n    "), file=sys.stderr)
            ok = False

        if ok and args.verify_cfi:
            notes = []
            if verify_cfi_live(tc, work, notes):
                detail = f" ({notes[0]})" if notes else ""
                print(f"  cfi-live     a mistyped indirect call was caught{detail}")
            else:
                print("  cfi-live     FAILED", file=sys.stderr)
                for note in notes:
                    print("    " + note, file=sys.stderr)
                ok = False
        elif ok:
            print("  cfi-live     not checked (pass --verify-cfi)")

    notes = []
    if audit_recipes(repo, notes):
        print("  recipes      no recipe sets its own -O/-flto/-fsanitize/-fvisibility")
    else:
        print("  recipes      FAILED -- these bypass manifest/toolchain.yaml:",
              file=sys.stderr)
        for note in notes:
            print("    " + note, file=sys.stderr)
        ok = False

    print()
    print("  NOT verified here:")
    print(f"    --target={tc.target.get('triple')}, --sysroot, -resource-dir")
    print("    and --unwindlib:")
    print("    no musl sysroot exists until losos-00-toolchain is built, and the")
    print("    resource directory and the unwinder both live inside it. The probe")
    print("    above drops exactly those four flags and keeps every other one, so")
    print("    what it proves is the flag set against the HOST's runtimes and")
    print("    unwinder, not the staged ones.")
    print("    See docs/limits.md for what remains outside the scheme.")

    return 0 if ok else 1


def show(argv, *, repo=REPO):
    parser = argparse.ArgumentParser(prog="pm-toolchain show")
    parser.add_argument("--triple", help="override the target triple")
    parser.add_argument("--package", help="show the flags for one exempt package")
    parser.add_argument("--cfi-export-map", help="path to the staged cfi-export.map")
    args = parser.parse_args(argv)

    tc = configured_toolchain(
        repo=repo,
        triple=args.triple,
        cfi_export_map=args.cfi_export_map,
    )
    package = args.package
    payload = {
        "compiler": tc.compiler,
        "target": tc.target,
        "lto": tc.lto,
        "cfi": tc.cfi,
        "hardening": tc.hardening,
        "rust": {
            "stable": tc.rustflags(),
            "nightly": tc.rustflags(nightly=True),
        },
        "package": package,
        "drops": sorted(tc.exceptions.get(package, {}).get("drops") or []),
        "cflags": tc.cflags(package),
        "ldflags": tc.ldflags(package),
        "meson": tc.meson_subs(package),
        "resource_dir_prefix": tc.resource_dir_prefix(),
    }
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


def cli_main(argv=None, *, repo=REPO):
    parser = argparse.ArgumentParser(prog="pm-toolchain")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("report", help="compile and audit the declared toolchain")
    sub.add_parser("show", help="print the resolved toolchain configuration as JSON")
    args, rest = parser.parse_known_args(argv)
    if args.command == "report":
        return report(rest, repo=repo)
    if args.command == "show":
        return show(rest, repo=repo)
    parser.error(f"unknown command: {args.command}")


def main(argv=None, *, repo=REPO):
    return cli_main(argv, repo=repo)
