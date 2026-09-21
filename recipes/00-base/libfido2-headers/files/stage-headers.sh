#!/bin/sh
# Stage libfido2's public headers and its pkg-config file, without compiling
# anything.
#
# This exists because systemd needs libfido2 at configure time and libfido2
# needs systemd's libudev at compile time. Only the second of those is a real
# link: systemd keeps nothing from the dependency but includes and compile
# args (meson.build:1238) and dlopens "libfido2.so.1" at runtime. So the
# headers can go in ahead of systemd and the library can follow after it.
#
# Everything installed here is a plain file out of the source tree --
# src/CMakeLists.txt:151-152 installs fido.h and the fido/ directory verbatim
# -- except the .pc, which upstream generates from libfido2.pc.in with an
# @ONLY substitution of four variables (line 154). Those four are reproduced
# below.
set -eu

src=$1
shift

# The version is read back out of the source rather than written here a second
# time. The recipe's own `version:` list is checked against manifest/sources.lock
# by tools/gates/versions.py; a copy in this script would be checked by nothing
# and would drift silently the first time the pin moved.
cml="$src/CMakeLists.txt"
major=$(sed -n 's/^set(FIDO_MAJOR  *"\([0-9][0-9]*\)").*$/\1/p' "$cml")
minor=$(sed -n 's/^set(FIDO_MINOR  *"\([0-9][0-9]*\)").*$/\1/p' "$cml")
patch=$(sed -n 's/^set(FIDO_PATCH  *"\([0-9][0-9]*\)").*$/\1/p' "$cml")

# An empty version would produce a .pc that pkg-config still accepts, so a
# rename upstream has to stop the build here rather than surface as a systemd
# built without FIDO2 for no stated reason.
if [ -z "$major" ] || [ -z "$minor" ] || [ -z "$patch" ]; then
	echo "stage-headers: could not read FIDO_MAJOR/MINOR/PATCH out of $cml" >&2
	exit 1
fi

for root in "$@"; do
	mkdir -p "$root/usr/include/fido" "$root/usr/lib/pkgconfig"
	cp -a "$src/src/fido.h" "$root/usr/include/fido.h"
	cp -a "$src/src/fido/." "$root/usr/include/fido/"

	# Written rather than sed-substituted from libfido2.pc.in, because the
	# template's four @VARS@ are cmake's and this recipe never runs cmake.
	# Requires: libcrypto is upstream's and is kept: pkg-config resolves it
	# transitively, so dropping it would make `pkg-config --exists libfido2`
	# answer yes on a sysroot with no openssl.
	cat > "$root/usr/lib/pkgconfig/libfido2.pc" <<-PC
	prefix=/usr
	exec_prefix=\${prefix}
	libdir=\${prefix}/lib
	includedir=\${prefix}/include

	Name: libfido2
	Description: A FIDO2 library
	URL: https://github.com/yubico/libfido2
	Version: $major.$minor.$patch
	Requires: libcrypto
	Libs: -L\${libdir} -lfido2
	Cflags: -I\${includedir}
	PC
done
