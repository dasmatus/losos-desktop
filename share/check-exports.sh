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
# object, and a `global:` list has nothing left to promote. libfido2 ships a
# version script naming 267 symbols and exported zero.
#
# Symbols are NAMED rather than counted. A count is satisfied by a library
# exporting some other handful, and the symbols that matter are the ones the
# rest of the tree reaches this library through.
#
# __cfi_check earns a word, because it is easy to assert for the wrong reason.
# It is present whenever CFI is on, at hidden and default visibility alike --
# measured on clang 19.1.1 and on 18.1.3 -- so on its
# own it proves nothing about visibility.
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
