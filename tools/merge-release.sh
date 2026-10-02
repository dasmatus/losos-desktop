#!/bin/sh
# Put a fragmented release file back together.
#
# tools/split-release cuts a release file larger than 1 GB into
# NAME.part-000, NAME.part-001, ... and writes NAME.parts, a sha256sum-format
# list of every fragment and, last, the whole file. This joins them and
# refuses to leave a file behind unless every digest in that list holds.
#
#   merge-release.sh [-d DIR] [-k] [-g KEYRING] [-u BASE_URL] NAME...
#
#   NAME        the file to rebuild, as it was before the split
#   -d DIR      where the fragments are, and where NAME is written (default .)
#   -u URL      download NAME.parts, SHA256SUMS(.gpg) and the fragments from
#               this release URL first (needs curl; resumes interrupted ones)
#   -g KEYRING  check SHA256SUMS.gpg against this public keyring (needs gpgv),
#               and the .parts file against SHA256SUMS. Without it the digests
#               only catch damage, not a fragment set someone swapped whole.
#   -k          keep the fragments after a successful merge
#
# Joining needs room for the whole file beside its fragments; the fragments
# are removed once the result verifies, unless -k.
#
# POSIX sh and coreutils only, because this runs on whatever machine holds the
# download, which is not necessarily one with this repository on it.
set -eu

dir=.
keep=0
keyring=
url=

usage() {
    sed -n '2,/^$/p;' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

die() {
    echo "merge-release: $*" >&2
    exit 1
}

while getopts d:u:g:kh opt; do
    case $opt in
        d) dir=$OPTARG ;;
        u) url=$OPTARG ;;
        g) keyring=$OPTARG ;;
        k) keep=1 ;;
        *) usage ;;
    esac
done
shift $((OPTIND - 1))
[ $# -ge 1 ] || usage

if command -v sha256sum >/dev/null 2>&1; then
    sum() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sum() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    die "neither sha256sum nor shasum is installed"
fi

fetch() {
    # -C - resumes, which is the point for a download of several gigabytes.
    curl -fL --retry 3 -C - -o "$dir/$1" "${url%/}/$1"
}

if [ -n "$keyring" ]; then
    command -v gpgv >/dev/null 2>&1 || die "-g needs gpgv"
fi
mkdir -p "$dir"

if [ -n "$url" ] && [ -n "$keyring" ]; then
    fetch SHA256SUMS
    fetch SHA256SUMS.gpg
fi
if [ -n "$keyring" ]; then
    [ -f "$dir/SHA256SUMS" ] && [ -f "$dir/SHA256SUMS.gpg" ] \
        || die "-g needs SHA256SUMS and SHA256SUMS.gpg in $dir"
    gpgv --keyring "$keyring" "$dir/SHA256SUMS.gpg" "$dir/SHA256SUMS" \
        || die "SHA256SUMS does not verify against $keyring"
fi

for name in "$@"; do
    case $name in
        */* | '') die "NAME is a file name, not a path: $name" ;;
    esac
    parts=$name.parts
    [ -z "$url" ] || fetch "$parts"
    [ -f "$dir/$parts" ] || die "no $dir/$parts"

    if [ -n "$keyring" ]; then
        want=$(awk -v n="$parts" '$2 == n {print $1}' "$dir/SHA256SUMS")
        [ -n "$want" ] || die "$parts is not listed in the signed SHA256SUMS"
        [ "$want" = "$(sum "$dir/$parts")" ] \
            || die "$parts is not the one the signed SHA256SUMS lists"
    fi

    # Fragment lines, in the order the list gives them; the last line is the
    # whole file and is checked after the join.
    list=$(awk -v n="$name" 'index($2, n ".part-") == 1 {print $2}' "$dir/$parts")
    [ -n "$list" ] || die "$parts lists no fragments"
    whole=$(awk -v n="$name" '$2 == n {print $1}' "$dir/$parts")
    [ -n "$whole" ] || die "$parts has no digest for $name"

    # In order and gapless: part-000, part-001, ... A list that skips one
    # would join into a file of the right fragments and the wrong shape.
    i=0
    for part in $list; do
        [ "$part" = "$(printf '%s.part-%03d' "$name" "$i")" ] \
            || die "$parts is out of order or skips a fragment at $part"
        i=$((i + 1))
    done

    for part in $list; do
        [ -z "$url" ] || fetch "$part"
        [ -f "$dir/$part" ] || die "missing fragment $part"
        want=$(awk -v n="$part" '$2 == n {print $1}' "$dir/$parts")
        [ "$want" = "$(sum "$dir/$part")" ] || die "$part is damaged; download it again"
    done

    tmp=$dir/$name.merging
    rm -f "$tmp"
    for part in $list; do
        cat "$dir/$part" >> "$tmp"
    done
    if [ "$(sum "$tmp")" != "$whole" ]; then
        rm -f "$tmp"
        die "$name does not match the digest in $parts after joining"
    fi
    mv -f "$tmp" "$dir/$name"
    echo "merge-release: $dir/$name ($i fragments, verified)"

    if [ "$keep" -eq 0 ]; then
        for part in $list; do
            rm -f "$dir/$part"
        done
    fi
done
