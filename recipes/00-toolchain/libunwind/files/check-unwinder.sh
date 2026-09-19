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

# The symbol list follows the table, because the two tables cannot answer the
# same question. DEFINE_LIBUNWIND_FUNCTION emits `.hidden` for every function
# it defines (src/assembly.h, and __unw_getcontext is defined through it in
# src/UnwindRegistersSave.S), so on ELF that symbol is STV_HIDDEN and never
# reaches a shared object's .dynsym no matter how correctly the assembly was
# built. Asking `nm -D` for it on the .so therefore fails on a perfectly good
# build, which is exactly what it did: the archive check one line above it in
# the recipe passed while this one took main red.
#
# So the dropped-assembly question is asked of the archive, which keeps hidden
# symbols in its symbol table, and the shared object is asked only whether the
# public entry points are exported -- which is the question a consumer of the
# shared library actually has, and the one .dynsym exists to answer.
nm_flags='--defined-only'
symbols='__unw_getcontext _Unwind_Backtrace _Unwind_GetIP'
case "$LIB" in
  *.so|*.so.*)
    nm_flags='-D --defined-only'
    symbols='_Unwind_Backtrace _Unwind_GetIP'
    ;;
esac

for symbol in $symbols; do
  if ! "$NM" $nm_flags "$LIB" | grep -q "[ 	]$symbol\$"; then
    # stderr, not stdout: pm reports a failed step's stderr and discards its
    # stdout, so a diagnosis written to stdout is a diagnosis nobody reads.
    echo "check-unwinder: $LIB does not define $symbol" >&2
    echo "check-unwinder: if it is __unw_getcontext, cmake dropped the" >&2
    echo "check-unwinder: assembly sources -- see files/patches." >&2
    exit 1
  fi
done

echo "check-unwinder: $LIB defines the unwind entry points"
