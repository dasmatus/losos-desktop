#!/bin/sh
# Run a command with the working directory changed. The repository's ONLY
# wrapper, and it is named as a compromise rather than hidden as a convenience.
#
# Why it has to exist: a build step has no shell and therefore no `cd` (C1), and
# the working directory is pinned to /build. `make -C`, `ninja -C`, `cmake -S/-B`
# and `meson setup <builddir> <srcdir>` all take a directory, so they need
# nothing. `./configure` does not -- autotools has no out-of-tree flag, it infers
# the source tree from argv[0] and builds in the current directory. Without this
# wrapper, an out-of-tree autotools build is inexpressible, and an in-tree one
# would limit a layer bundle to a single autotools package.
#
# Why it is a compromise: pm's fingerprint check reads the first word only (C2),
# so `/bin/sh in-dir.sh <dir> <prog>` is classified `shell` and <prog> is never
# checked. That is the same hole `env` opens. The difference is that this script
# is enumerated in tools/gates/allowed-wrappers and the fingerprint lint
# re-applies pm's table to whatever follows the directory argument -- so the
# check `pm explain` appears to give is actually given, by us, here.
#
# Usage: sh in-dir.sh <dir> <program> [args...]

set -eu

dir="${1:?usage: in-dir.sh <dir> <program> [args...]}"
shift

cd "$dir"
exec "$@"
