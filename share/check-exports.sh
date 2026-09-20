#!/bin/sh
# Fail if a staged shared library does not export the symbols it is here for.
#
# This exists because the failure it catches is silent. Every layer compiles
# with -fvisibility=hidden -- there only as cross-DSO CFI's precondition -- and
# a library whose public API carries no visibility attribute comes out of that
# with an empty dynamic symbol table. It still compiles, still links, still
# installs, and still passes a `test -e`. Nothing goes wrong until some other
# package, often layers later, fails to find a symbol; by then the error names
# the consumer rather than the cause. zlib's own copy of this check was written
# after exactly that, and this is that check generalised so every library can
# carry one.
#
# A linker version script does NOT save a package here and must not be taken as
# evidence that it is fine: -fvisibility=hidden writes STV_HIDDEN into the
# object, and a `global:` list has nothing left to promote.
#
# libfido2 is the case, and it is a real one: src/export.gnu names 267 symbols
# and the library exported none of them. The script is genuinely wired up --
# CMakeLists.txt:361-368 at the 1.15.0 tag appends
# -Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/src/export.gnu to
# CMAKE_SHARED_LINKER_FLAGS, three conditionals deep in the top-level file and
# never as a target property, so grepping src/CMakeLists.txt for --version-script
# or LINK_FLAGS finds nothing and reads as proof the script is unused. It is
# used. The linker was handed 267 names that -fvisibility=hidden had already
# made STV_HIDDEN and promoted not one of them, which is the paragraph above
# rather than a build that forgot its own export list. drops: [visibility] is
# what fixed it.
#
# THE OPPOSITE DIRECTION IS ALSO TRUE AND IS EASIER TO MISS. A version script
# can DESTROY a symbol that was perfectly visible. Every one ending `local: *;`
# -- which is most of them, since it is how a library says "export my API and
# nothing else" -- localises __cfi_check along with everything else that is not
# public API. That one is STV_DEFAULT even under -fvisibility=hidden, precisely
# so cross-DSO CFI can find it, so a version script is the only thing that can
# take it away. Losing it is silent and total: the runtime looks the name up in
# DT_SYMTAB and, not finding it, marks the module's whole address range
# unchecked.
#
# The two read alike and pull opposite ways: above, a version script is not
# enough to rescue a symbol; here, it is enough to remove one. Hence the two
# different messages below, and share/cfi-export.map, which is appended to
# every link so the second case cannot happen quietly.
#
# libfido2 is in both classes at once, which is worth saying because fixing one
# leaves a library that passes the other's test. Hidden visibility came first
# and emptied the table wholesale, API and all. drops: [visibility] gives the
# API back -- and hands the version script real symbols to filter, at which
# point `local: *;` at the end of src/export.gnu takes __cfi_check away
# instead. So share/cfi-export.map is load-bearing for libfido2 itself, not
# only for the packages whose dynamic tables were never empty to begin with.
#
# Symbols are NAMED rather than counted. A count is satisfied by a library
# exporting some other handful, and the symbols that matter are the ones the
# rest of the tree reaches this library through.
#
# __cfi_check earns a word, because it is easy to assert for the wrong reason.
# It is present whenever CFI is on, at hidden and default visibility alike --
# measured on clang 18.1.3, and repeated on 19.1.1 after it turned out that
# 18.1.3 is a local sandbox's compiler and not the build's: the Containerfile
# asks for LLVM_VERSION=19 and CI reports 19.1.7. Every measurement in this
# file and in share/cfi-export.map held on both. So __cfi_check on its own
# proves nothing about visibility.
# What it does prove is that a `drops: [visibility]` exception kept the checks
# instead of quietly losing them, which is the thing `drops: [cfi]` gives up.
# So a package on the narrow exception should name it alongside its API: the
# API symbol says visibility was fixed, __cfi_check says CFI survived, and
# neither says the other.
#
# A step has no shell and no pipes (C1), which is why this is a script rather
# than `nm ... | grep`.
#
# Usage: sh check-exports.sh <nm> <library> <symbol>...

set -eu

NM="${1:?usage: check-exports.sh <nm> <library> <symbol>...}"
LIB="${2:?usage: check-exports.sh <nm> <library> <symbol>...}"
shift 2

[ "$#" -gt 0 ] || { echo "check-exports: name at least one symbol"; exit 1; }

# Read the table once and keep it, because the whole-API case deserves its own
# report. A library with nothing in its dynamic symbol table is the signature
# of this class, and saying so is a diagnosis; naming whichever symbol happened
# to be checked first reads as one missing function and sends the next person
# looking for it in the source.
table=$("$NM" --dynamic --defined-only "$LIB")

if [ -z "$table" ]; then
  echo "check-exports: $LIB has an EMPTY dynamic symbol table"
  echo "check-exports: not one symbol, not just the ones named here. Its public"
  echo "check-exports: API carries no visibility attribute and -fvisibility=hidden"
  echo "check-exports: has made every definition STV_HIDDEN. See the exceptions"
  echo "check-exports: in manifest/toolchain.yaml for what to do about it."
  exit 1
fi

for symbol in "$@"; do
  if ! printf '%s\n' "$table" | grep -q "[ 	]$symbol\$\|[ 	]$symbol@"; then
    echo "check-exports: $LIB does not export $symbol"
    if [ "$symbol" = "__cfi_check" ]; then
      echo "check-exports: CFI was dropped for this package rather than kept."
      echo "check-exports: a drops: [visibility] exception should still have it,"
      echo "check-exports: and so should a library whose own version script ends"
      echo "check-exports: local: * -- share/cfi-export.map is on every link to"
      echo "check-exports: promote this one symbol back past exactly that. If it"
      echo "check-exports: is missing anyway, the map did not reach this link."
    else
      echo "check-exports: this is what -fvisibility=hidden does to a library"
      echo "check-exports: whose public API carries no visibility attribute."
      echo "check-exports: a version script does not rescue it -- the symbol is"
      echo "check-exports: already STV_HIDDEN by the time the linker reads one."
    fi
    exit 1
  fi
done

echo "check-exports: $LIB exports its public API"
