#!/bin/sh
# Make curl's generated version script survive cross-DSO CFI, and prove the
# build took the version-script branch at all.
#
# WHY THIS EXISTS AT ALL
#
# lib/Makefile.am:117-124 picks one of two ways to restrict libcurl's exports:
#
#     if CURL_LT_SHLIB_USE_VERSIONED_SYMBOLS
#     libcurl_la_LDFLAGS_EXTRA += -Wl,--version-script=libcurl.vers
#     else
#     if DOING_CURL_SYMBOL_HIDING
#     libcurl_la_LDFLAGS_EXTRA += -export-symbols-regex '^curl_.*'
#     endif
#     endif
#
# The `else` asks *libtool* to compute the export list, and libtool computes it
# by running $NM over the objects and piping the output through
# $lt_cv_sys_global_symbol_pipe. That variable is empty in every autotools
# package in this tree, because the probe that sets it dies under this
# toolchain: with -flto=thin the probe's object is LLVM bitcode, llvm-nm lists
# the __cfi_check and __cfi_check_fail that cross-DSO CFI emitted, libtool
# generates a C file taking the address of each, and compiling *that* with
# -fsanitize-cfi-cross-dso crashes clang in EmitCfiCheckStub(). libtool treats
# the crash as "the pipe does not work", sets it empty, prints `failed`, and
# builds on. The bill arrives here, as a shell error rather than a link error:
#
#     ../libtool: eval: line 1877: syntax error near unexpected token `|'
#     ../libtool: eval: line 1877: `/usr/bin/llvm-nm  .libs/libcurl_la-...
#
# because ltmain evals `$NM <objects> | $pipe` and the line ends in a bare `|`.
#
# The crash is confirmed on clang 18.1.3, 19.1.1 and 22.1.8 -- three majors
# apart, and worth stating because this whole recipe rests on it and the
# obvious hope is that a newer compiler has fixed it. It has not. At 22.1.8,
# on a container base built from Arch, configure still printed
#
#     checking command to parse llvm-nm output from clang object... failed
#
# for both openssl and curl, and curl went on to build anyway -- which is this
# script doing its job rather than the problem having gone away.
#
# That `failed` line is the cheapest way to check the state of this on any new
# toolchain: it is in the build log of every autotools package, and it needs no
# artifact, no successful link and no nm. Two warnings for whoever reproduces
# the crash itself instead. The translation unit needs a CFI-relevant function
# in it as well as the `extern char __cfi_check;` -- the declaration alone
# compiles fine -- so the smallest possible test looks like the bug is fixed.
# And none of these numbers is the container's compiler, deliberately: that has
# been three different values in a day, so a comment naming it is wrong by the
# next base image and a comment naming what was measured never is.
#
# So the recipe passes --enable-versioned-symbols and takes the `if` branch,
# where libtool hands a finished version script to the linker and never reads a
# symbol itself. That is also what every mainstream distribution builds curl
# with, so it is the well-travelled path rather than a workaround.
#
# WHAT THIS SCRIPT FIXES
#
# lib/libcurl.vers.in is, in full:
#
#     CURL_@..._PREFIX@@..._SONAME@ { global: curl_*; local: *; };
#
# `local: *;` is the problem. It hides every symbol that is not curl_*, and
# __cfi_check is not curl_*. Cross-DSO CFI needs __cfi_check exported from
# every shared object: the runtime finds a callee's DSO and dispatches through
# that DSO's __cfi_check, which it looks up by name in the dynamic symbol
# table. Strip it and, at best, CFI silently stops checking calls into libcurl
# -- which is the one library here that parses hostile network bytes and so the
# last one to want it off. That failure is invisible: the build stays green,
# the image ships, and nothing says CFI stopped covering curl.
#
# This is the same shape as the thing it replaces, which is why it is worth
# saying out loud: swapping -export-symbols-regex for a version script trades a
# loud shell error for a silent loss of hardening unless the version script is
# corrected too.
#
# Promoting it works because __cfi_check is not hidden to begin with. clang
# emits it at default visibility whether or not -fvisibility=hidden is in the
# flags (measured on clang 18.1.3 and again on 19.1.1, the second for the
# reason given above; recorded in share/check-exports.sh),
# precisely so cross-DSO dispatch can find it. A version script can promote a
# STV_DEFAULT symbol; it could not have rescued a hidden one.
#
# THE OTHER HALF: PROVING THE BRANCH WAS TAKEN
#
# configure.ac:2816-2821 does not fail when --enable-versioned-symbols cannot
# be honoured. It probes `$LD --help` for version-script support and, finding
# none, calls AC_MSG_WARN and carries on with versioned_symbols unset -- which
# drops the build straight back onto -export-symbols-regex and the shell error
# above, several hundred lines of output later, reading exactly like the
# failure we came here to fix. A warning that turns into an identical-looking
# failure much later is worse than no fix, so this asserts the generated
# lib/Makefile actually carries --version-script before touching anything.
#
# A step has no shell and no pipes (C1), which is why this is a script.
#
# Usage: sh version-script.sh <build-dir>

set -eu

B="${1:?usage: version-script.sh <build-dir>}"

makefile="$B/lib/Makefile"
vers="$B/lib/libcurl.vers"

[ -f "$makefile" ] || {
	echo "version-script: $makefile does not exist -- configure did not finish" >&2
	exit 1
}

# The assertion the whole script is here for. configure only warns when it
# cannot honour --enable-versioned-symbols, so this is the only place the
# difference is visible before the build is deep into libtool.
if ! grep -q -- '--version-script' "$makefile"; then
	echo "version-script: lib/Makefile carries no --version-script." >&2
	echo "version-script: --enable-versioned-symbols was passed but configure" >&2
	echo "version-script: did not honour it -- configure.ac:2816 probes" >&2
	echo "version-script: \`\$LD --help\` for version-script support and only" >&2
	echo "version-script: WARNS when it finds none. The build would now fall" >&2
	echo "version-script: back on -export-symbols-regex and die inside libtool" >&2
	echo "version-script: with a bare \`|\`, because this tree's libtool symbol" >&2
	echo "version-script: pipe is empty. Check which ld configure found." >&2
	exit 1
fi

[ -f "$vers" ] || {
	echo "version-script: $vers does not exist" >&2
	exit 1
}

# Promote __cfi_check alongside the public API. Matching `global:` rather than
# the whole line keeps this working if upstream ever adds a second name to the
# list.
sed -i 's/global: curl_\*;/global: curl_*; __cfi_check;/' "$vers"

# Never trust a sed. A silent no-op here would put us back to shipping a
# libcurl that cross-DSO CFI cannot dispatch into, with nothing to show for it.
if ! grep -q '__cfi_check' "$vers"; then
	echo "version-script: failed to add __cfi_check to $vers" >&2
	echo "version-script: its global: line is not the shape this expected:" >&2
	cat "$vers" >&2
	exit 1
fi

echo "version-script: libcurl.vers exports the public API and __cfi_check"
