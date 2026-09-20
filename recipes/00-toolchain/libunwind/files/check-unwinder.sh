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
#   * The shared library is asked for unw_getcontext, the public alias of
#     the same assembly routine, plus the public unwind ABI. __unw_getcontext
#     is NOT in its dynamic symbol table and cannot be: src/assembly.h:258
#     defines DEFINE_LIBUNWIND_FUNCTION to emit HIDDEN_SYMBOL alongside
#     .globl, and HIDDEN_SYMBOL is `.hidden name` (assembly.h:141). Upstream
#     hides every __unw_* entry point on purpose; they are libunwind's
#     internal interface. UnwindRegistersSave.S declares the unw_* weak
#     aliases beside them, so in the shared object, whose dynamic table is
#     what `nm -D` reads, the hidden name is a local symbol and cannot show
#     up at all. Measured on the staged library: `nm` lists
#     `t __unw_getcontext` and `W unw_getcontext` at the same address,
#     `nm -D` lists only the alias. The alias is defined in the same
#     assembly file, so its presence proves the same thing.
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

nm_flags='--defined-only'
accept_versions=false
getcontext='__unw_getcontext'

case "$LIB" in
  *.so|*.so.*)
    nm_flags='-D --defined-only'
    accept_versions=true
    getcontext='unw_getcontext'
    ;;
esac

for symbol in "$getcontext" _Unwind_Backtrace _Unwind_GetIP; do
  if [ "$accept_versions" = true ]; then
    symbol_pattern="[ 	]$symbol($|@)"
    grep_flags='-Eq'
  else
    symbol_pattern="[ 	]$symbol\$"
    grep_flags='-q'
  fi
  if ! "$NM" $nm_flags "$LIB" | grep $grep_flags "$symbol_pattern"; then
    # stderr, not stdout. pm reports a failed step's stderr and discards its
    # stdout, so these three lines on stdout reached nobody: the CI summary
    # for this exact failure read `stderr: <no output>` and named only the
    # command. A diagnosis nobody reads is not a diagnosis.
    echo "check-unwinder: $LIB does not define $symbol" >&2
    echo "check-unwinder: if it is $getcontext, cmake dropped the" >&2
    echo "check-unwinder: assembly sources -- see files/patches." >&2
    exit 1
  fi
done

echo "check-unwinder: $LIB defines the unwind entry points"
