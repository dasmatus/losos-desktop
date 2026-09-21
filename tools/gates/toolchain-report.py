#!/usr/bin/env bash
# Compile something with the real flag set, and report what is actually on.
#
# "The build is LTO and CFI" is the kind of claim that is almost always partly
# false, because both are properties of a whole link unit rather than of a
# compiler invocation:
#
#   * an indirect call is only checked if the caller was built with CFI;
#   * a call across a shared-library boundary is only checked if both sides
#     were, and only with -fsanitize-cfi-cross-dso;
#   * CFI silently needs LTO, and LTO silently needs llvm-ar rather than GNU ar,
#     or archive members become invisible to the linker;
#   * one package that opts out is a hole nothing reports.
#
# So this does not read the manifest and pronounce. It builds a shared library
# and an executable with the exact flags manifest/toolchain.yaml declares,
# links them, runs the result, and then checks that no recipe has quietly
# introduced optimisation or sanitizer flags of its own.
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
manifest="$repo/manifest/toolchain.yaml"
verify=0
case "${1:-}" in
  --verify-cfi) verify=1 ;;
  "") ;;
  *) echo "toolchain-report: unknown argument: $1" >&2; exit 1 ;;
esac

manifest_value() {
  local section=$1 key=$2
  awk -v section="$section" -v key="$key" '
    $0 ~ "^" section ":$" { in_section=1; next }
    in_section && /^[^ ]/ { exit }
    in_section && $0 ~ "^  " key ":" {
      sub("^  " key ": *", "")
      sub(/[[:space:]]+#.*/, "")
      print
      exit
    }
  ' "$manifest"
}

manifest_list() {
  local section=$1 key=$2
  awk -v section="$section" -v key="$key" '
    $0 ~ "^" section ":$" { in_section=1; next }
    in_section && /^[^ ]/ { exit }
    in_section && $0 ~ "^  " key ":$" { in_list=1; next }
    in_list && $0 ~ /^  [^ ]/ { exit }
    in_list && $0 ~ /^    - / {
      sub(/^    - /, "")
      sub(/[[:space:]]+#.*/, "")
      print
    }
  ' "$manifest"
}

mapfile -t cfi_schemes < <(manifest_list cfi schemes)
mapfile -t hardening_cflags < <(manifest_list hardening cflags)
mapfile -t hardening_ldflags < <(manifest_list hardening ldflags)
mapfile -t exception_lines < <(
  awk '
    /^exceptions:$/ { in_exceptions=1; next }
    in_exceptions && /^[^ ]/ { exit }
    in_exceptions && /^  - package: / {
      pkg=$4
      next
    }
    in_exceptions && pkg != "" && /^    drops: \[/ {
      drops=$0
      sub(/^    drops: \[/, "", drops)
      sub(/\][[:space:]]*$/, "", drops)
      print pkg "|" drops
      pkg=""
    }
  ' "$manifest"
)

cc=$(manifest_value compiler cc)
linker=$(manifest_value compiler linker)
triple=$(manifest_value target triple)
sysroot=$(manifest_value target sysroot)
rtlib=$(manifest_value target rtlib)
unwindlib=$(manifest_value target unwindlib)
resource_dir=$(manifest_value target resource_dir)
lto_mode=$(manifest_value lto mode)
cfi_enable=$(manifest_value cfi enable)
cross_dso=$(manifest_value cfi cross_dso)
trap_mode=$(manifest_value cfi trap)
visibility=$(manifest_value cfi visibility)

cfi_flags=()
if [ "$cfi_enable" = true ] && [ "${#cfi_schemes[@]}" -gt 0 ]; then
  cfi_flags+=("-fvisibility=$visibility")
  for scheme in "${cfi_schemes[@]}"; do
    cfi_flags+=("-fsanitize=$scheme")
  done
  if [ "$cross_dso" = true ]; then
    cfi_flags+=("-fsanitize-cfi-cross-dso")
  fi
  if [ "$trap_mode" != true ]; then
    cfi_flags+=("-fno-sanitize-trap=cfi" "-fsanitize-recover=cfi")
  fi
fi

cflags=("--target=$triple" "--sysroot=$sysroot" "-resource-dir=$resource_dir")
cflags+=("${hardening_cflags[@]}")
[ -n "$lto_mode" ] && cflags+=("-flto=$lto_mode")
cflags+=("${cfi_flags[@]}")

ldflags=("--target=$triple" "--sysroot=$sysroot" "-resource-dir=$resource_dir")
[ -n "$rtlib" ] && ldflags+=("--rtlib=$rtlib")
[ -n "$unwindlib" ] && ldflags+=("--unwindlib=$unwindlib")
[ -n "$linker" ] && ldflags+=("-fuse-ld=$linker")
[ -n "$lto_mode" ] && ldflags+=("-flto=$lto_mode")
ldflags+=("${cfi_flags[@]}")
ldflags+=("${hardening_ldflags[@]}")

host_flags() {
  local flag
  for flag in "$@"; do
    case "$flag" in
      --target=*|--sysroot=*|-resource-dir=*|--unwindlib=*) ;;
      *) printf '%s\n' "$flag" ;;
    esac
  done
}

mapfile -t host_cflags < <(host_flags "${cflags[@]}")
mapfile -t host_ldflags < <(host_flags "${ldflags[@]}")

printf 'toolchain-report\n'
printf '  compiler     %s / %s\n' "$cc" "$linker"
printf '  target       %s\n' "$triple"
printf '  LTO          %s\n' "$lto_mode"
printf '  CFI          %s  cross-DSO=%s  trap=%s\n' \
  "$([ "$cfi_enable" = true ] && echo on || echo off)" "$cross_dso" "$trap_mode"
printf '  schemes      %s\n' "$(IFS=', '; echo "${cfi_schemes[*]:-none}")"

# Printed every run, because the exceptions ARE the honest part of a CFI
# claim. A scheme applied to a distribution always has them; the difference
# between a real claim and a marketing one is whether they are counted.
if [ "${#exception_lines[@]}" -gt 0 ]; then
  printf '  exceptions   %s package(s) outside the full set:\n' "${#exception_lines[@]}"
  for line in "${exception_lines[@]}"; do
    pkg=${line%%|*}
    drops=${line#*|}
    printf '    %-12s drops %s\n' "$pkg" "$drops"
  done
else
  printf '  exceptions   none\n'
fi

notes=()
if linker_output=$(
  printf 'int x;\n' | "$cc" "${host_ldflags[@]}" -nostdlib -shared -x c - -Wl,--version -o /dev/null 2>&1
); then
  banner=$(printf '%s\n' "$linker_output" | sed -n '1{s/[[:space:]]*$//;p;q;}')
  if [ -n "$banner" ]; then
    printf '  linker       %s\n' "$banner"
  else
    echo '  linker       FAILED' >&2
    echo '    the linker printed no version banner' >&2
    linker_failed=1
  fi
else
  echo '  linker       FAILED' >&2
  printf '    the driver could not start its linker:\n    %s\n' "$(printf '%s' "$linker_output" | sed 's/^/    /')" >&2
  linker_failed=1
fi

ok=1
: "${linker_failed:=0}"
[ "$linker_failed" -eq 0 ] || ok=0

work=$(mktemp -d -t losos-toolchain-XXXXXX)
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

cat > "$work/lib.c" <<'EOF'
#include <stdio.h>
__attribute__((visibility("default"))) int lib_add(int a, int b) { return a + b; }
__attribute__((visibility("default"))) void lib_say(void) { puts("lib"); }
EOF
cat > "$work/main.c" <<'EOF'
#include <stdio.h>
extern int lib_add(int, int);
extern void lib_say(void);

int main(void) {
    int (*fp)(int, int) = lib_add;
    lib_say();
    printf("%d\n", fp(2, 3));
    return 0;
}
EOF
cat > "$work/violation.c" <<'EOF'
#include <stdio.h>
extern int lib_add(int, int);

typedef long (*wrong_t)(long, long, long);

int main(void) {
    wrong_t bad = (wrong_t)(void *)lib_add;
    printf("%ld\n", bad(1, 2, 3));
    return 0;
}
EOF

if "$cc" "${host_cflags[@]}" "${host_ldflags[@]}" -shared -o "$work/libprobe.so" "$work/lib.c" >"$work/lib.err" 2>&1 \
  && "$cc" "${host_cflags[@]}" "${host_ldflags[@]}" -o "$work/probe" "$work/main.c" -L "$work" -lprobe "-Wl,-rpath,$work" >"$work/probe.err" 2>&1; then
  probe_out=$(
    cd "$work" && ./probe
  ) || {
    echo '  build        FAILED' >&2
    printf '    probe ran wrong: rc=%s out=%q\n' "$?" "$probe_out" >&2
    ok=0
  }
  if [ "$ok" -eq 1 ]; then
    if [ "$probe_out" = $'lib\n5' ]; then
      echo '  build        lib + exe compiled, linked and ran with the real flags'
    else
      echo '  build        FAILED' >&2
      printf '    probe ran wrong: out=%q\n' "$probe_out" >&2
      ok=0
    fi
  fi
else
  echo '  build        FAILED' >&2
  if [ -s "$work/lib.err" ]; then
    printf '    shared library did not build:\n' >&2
    sed 's/^/    /' "$work/lib.err" >&2
  else
    printf '    executable did not link:\n' >&2
    sed 's/^/    /' "$work/probe.err" >&2
  fi
  ok=0
fi

if [ "$ok" -eq 1 ] && [ "$verify" -eq 1 ]; then
  if "$cc" "${host_cflags[@]}" "${host_ldflags[@]}" -o "$work/violation" "$work/violation.c" -L "$work" -lprobe "-Wl,-rpath,$work" >"$work/violation-build.err" 2>&1; then
    set +e
    violation_combined=$(cd "$work" && ./violation 2>&1)
    violation_status=$?
    set -e
    lower=$(printf '%s' "$violation_combined" | tr '[:upper:]' '[:lower:]')
    if printf '%s' "$lower" | grep -Eq 'control flow integrity|cfi'; then
      echo '  cfi-live     a mistyped indirect call was caught'
    elif [ "$violation_status" -lt 0 ]; then
      printf '  cfi-live     a mistyped indirect call was caught (trapped with signal %s)\n' "$((-violation_status))"
    else
      echo '  cfi-live     FAILED' >&2
      printf "    the bad indirect call was NOT caught -- CFI compiled in but is not checking (rc=%s, output=%q)\n" "$violation_status" "$violation_combined" >&2
      ok=0
    fi
  else
    echo '  cfi-live     FAILED' >&2
    printf '    violation probe did not build:\n' >&2
    sed 's/^/    /' "$work/violation-build.err" >&2
    ok=0
  fi
elif [ "$ok" -eq 1 ]; then
  echo '  cfi-live     not checked (pass --verify-cfi)'
fi

# A recipe setting -O3 or -fsanitize= locally is how a package ends up
# outside the scheme while the manifest still claims it is inside.
# The character class must not contain a space: `-O <dir>` is an output
# directory for several tools (merge_config.sh among them) and is not an
# optimisation level. An earlier spelling included one and flagged the
# kernel's config merge.
offenders=()
while IFS= read -r -d '' template; do
  number=0
  while IFS= read -r line || [ -n "$line" ]; do
    number=$((number + 1))
    stripped=${line#"${line%%[![:space:]]*}"}
    case "$stripped" in
      \#*|'') continue ;;
    esac
    if [[ "$line" =~ (^|[[:space:]])-(O[0-9zs]|flto|fsanitize|fvisibility)($|[[:space:]=,:;]) ]]; then
      offenders+=("${template#$repo/}:$number: $stripped")
    fi
  done < "$template"
done < <(find "$repo/recipes" -mindepth 3 -maxdepth 3 -path '*/build.yaml.in' -print0 | sort -z)

if [ "${#offenders[@]}" -eq 0 ]; then
  echo '  recipes      no recipe sets its own -O/-flto/-fsanitize/-fvisibility'
else
  echo '  recipes      FAILED -- these bypass manifest/toolchain.yaml:' >&2
  printf '    %s\n' "${offenders[@]}" >&2
  ok=0
fi

echo
printf '  NOT verified here:\n'
printf '    --target=%s, --sysroot, -resource-dir\n' "$triple"
printf '    and --unwindlib:\n'
printf '    no musl sysroot exists until losos-00-toolchain is built, and the\n'
printf '    resource directory and the unwinder both live inside it. The probe\n'
printf '    above drops exactly those four flags and keeps every other one, so\n'
printf '    what it proves is the flag set against the HOST\047s runtimes and\n'
printf '    unwinder, not the staged ones.\n'
printf '    See docs/limits.md for what remains outside the scheme.\n'

exit "$((ok ? 0 : 1))"
