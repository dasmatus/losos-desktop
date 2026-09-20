#!/bin/sh
# Fail if the staged unwinder is missing the symbols an unwind starts from.
#
# This is not belt and braces. libunwind's CMakeLists has no project() call,
# so configured directly it has no ASM language, and cmake responds to a
# source it cannot compile by dropping it rather than by failing. The library
# then builds and installs with its two assembly files silently absent, and
# the first thing to notice is a link four layers up. The patch in
# files/patches fixes that; this is what proves the patch is still doing its
# job after a version bump.
#
# __unw_getcontext is the one that comes from assembly. The two _Unwind_
# entry points are what -fno-sanitize-trap=cfi actually calls.
#
# A step has no shell and no pipes (C1), which is why this is a script.
#
# Usage: sh check-unwinder.sh <nm> <library>

set -eu

NM="${1:?usage: check-unwinder.sh <nm> <library>}"
LIB="${2:?usage: check-unwinder.sh <nm> <library>}"

nm_flags='--defined-only'
accept_versions=false

case "$LIB" in
  *.so|*.so.*)
    nm_flags='--dynamic --defined-only'
    accept_versions=true
    ;;
esac

for symbol in __unw_getcontext _Unwind_Backtrace _Unwind_GetIP; do
  if [ "$accept_versions" = true ]; then
    symbol_pattern="[ 	]$symbol($|@)"
    grep_flags='-Eq'
  else
    symbol_pattern="[ 	]$symbol\$"
    grep_flags='-q'
  fi
  if ! "$NM" $nm_flags "$LIB" | grep $grep_flags "$symbol_pattern"; then
    echo "check-unwinder: $LIB does not define $symbol"
    echo "check-unwinder: if it is __unw_getcontext, cmake dropped the"
    echo "check-unwinder: assembly sources -- see files/patches."
    exit 1
  fi
done

echo "check-unwinder: $LIB defines the unwind entry points"
