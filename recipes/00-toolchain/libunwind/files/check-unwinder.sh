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
# The two libraries are asked different questions, which is the whole of the
# logic below.
#
#   * The archive is asked for __unw_getcontext, because that is the symbol
#     the assembly defines and therefore the one that goes missing when cmake
#     drops the assembly. It is read out of the ordinary symbol table.
#   * The shared library is asked only for the public unwind ABI, because
#     __unw_getcontext is NOT in its dynamic symbol table and cannot be:
#     src/assembly.h:258 defines DEFINE_LIBUNWIND_FUNCTION to emit
#     HIDDEN_SYMBOL alongside .globl, and HIDDEN_SYMBOL is `.hidden name`
#     (assembly.h:141). Upstream hides every __unw_* entry point on purpose;
#     they are libunwind's internal interface, reached from inside the
#     library, and only the _Unwind_ names are meant to be linked against.
#
# Tombstone: this script asked both libraries for all three symbols, and the
# .so passed anyway, because `nm --defined-only` reads .symtab where a hidden
# symbol is still present. Switching the .so to `nm -D` -- correct, since a
# consumer links against .dynsym and nothing else -- is what turned the wrong
# question into a failing one. Asking the .so for __unw_getcontext again will
# not find a regression; it will only break the build.
#
# _Unwind_Backtrace and _Unwind_GetIP are what -fno-sanitize-trap=cfi actually
# calls, so a shared library that exports them is one cfi_diag can use.
#
# A step has no shell and no pipes (C1), which is why this is a script.
#
# Usage: sh check-unwinder.sh <nm> <library>

set -eu

NM="${1:?usage: check-unwinder.sh <nm> <library>}"
LIB="${2:?usage: check-unwinder.sh <nm> <library>}"

case "$LIB" in
  *.so|*.so.*)
    nm_flags='-D --defined-only'
    symbols='_Unwind_Backtrace _Unwind_GetIP'
    ;;
  *)
    nm_flags='--defined-only'
    symbols='__unw_getcontext _Unwind_Backtrace _Unwind_GetIP'
    ;;
esac

for symbol in $symbols; do
  if ! "$NM" $nm_flags "$LIB" | grep -q "[ 	]$symbol\$"; then
    echo "check-unwinder: $LIB does not define $symbol"
    echo "check-unwinder: if it is __unw_getcontext, cmake dropped the"
    echo "check-unwinder: assembly sources -- see files/patches."
    exit 1
  fi
done

echo "check-unwinder: $LIB defines the unwind entry points"
