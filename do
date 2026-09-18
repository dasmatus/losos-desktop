#!/usr/bin/env bash
# The one entry point. `./do` with no argument lists what it can do.
#
# There is no Makefile and no flake here: pm is the build system, and wrapping
# it in a second one would only add a place for the two to disagree. This
# script's whole job is to put pm in the right directory with the right
# environment, in the right order.
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
pm=${PM:-$repo/../pm/target/release/pm}
mirror_port=${LOSOS_MIRROR_PORT:-8730}
mirror_url="http://127.0.0.1:${mirror_port}"

# Every pm build workspace lives under TMPDIR and a large package needs several
# GB of it. On a tmpfs /tmp -- the default on much of the world -- a real build
# dies partway through with "Disk quota exceeded (os error 122)", which looks
# like a bug in the recipe and is not. out/tmp is on whatever disk the
# repository is on.
export TMPDIR="$repo/out/tmp"
# The repo-local trust store. Never the developer's real ~/.config/pm.
export XDG_CONFIG_HOME="$repo/.pm-config"

mkdir -p "$repo/out/tmp" "$repo/out/pkgs"

have_pm() {
  if [ ! -x "$pm" ]; then
    echo "no pm binary at $pm" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi
}

mirror_running() { python3 "$repo/tools/serve-sources" --port "$mirror_port" --check; }

start_mirror() {
  if mirror_running; then return 0; fi
  if [ ! -d "$repo/out/sources" ]; then return 1; fi
  python3 "$repo/tools/serve-sources" --port "$mirror_port" >"$repo/out/tmp/mirror.log" 2>&1 &
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    mirror_running && return 0
    sleep 0.3
  done
  return 1
}

cmd_configure() {
  # Generate against the local mirror when one is available. pm's downloader
  # trusts only the Mozilla roots compiled into it and reads no CA setting, so
  # on a host that re-terminates TLS it cannot fetch upstream at all; the
  # mirror serves the same bytes over loopback and the SHA-256 pin is unchanged.
  # See tools/fetch-sources.
  if [ -d "$repo/out/sources" ]; then
    python3 "$repo/tools/configure" --mirror "$mirror_url" "$@"
  else
    python3 "$repo/tools/configure" "$@"
  fi
}

cmd_lint() {
  have_pm
  python3 "$repo/tools/gates/schema.py"
  python3 "$repo/tools/gates/url-canonical.py"
  python3 "$repo/tools/gates/fingerprint-lint.py" --check-table
  python3 "$repo/tools/gates/fingerprint-lint.py"
  python3 "$repo/tools/gates/test-image.py"
  "$repo/tools/gates/explain-all"
}

cmd_check() {
  have_pm
  echo "== configure"
  # The gate runs with unresolved hashes tolerated: it lints the shape of the
  # tree, and an unpinned source is a fetch problem, not a recipe problem.
  # `./do build` still refuses to start with any TODO left.
  cmd_configure --allow-unresolved "$@"
  echo "== sign"
  "$repo/tools/sign-all" >/dev/null
  echo "== digest agreement with pm"
  PM="$pm" python3 "$repo/tools/check-digest"
  echo "== lint"
  cmd_lint
  echo
  echo "check: green"
}

cmd_build() {
  have_pm
  local target="${1:-}"
  if [ -z "$target" ]; then
    # Default to the top of the chain: the last layer in manifest/layers.yaml.
    target=$(python3 -c "
import yaml, pathlib
layers = yaml.safe_load(pathlib.Path('$repo/manifest/layers.yaml').read_text()) or []
print(layers[-1]['name'] if layers else '')
")
  fi
  [ -n "$target" ] || { echo "nothing to build" >&2; exit 1; }

  start_mirror || echo "do: no local source mirror; pm will fetch upstream" >&2
  # Generate the whole tree, then insist only that the layers this build
  # actually walks are pinned. Refusing because some unrelated upper layer has
  # an unfetched source would make a partial tree unbuildable for no reason --
  # and a partial tree is the normal state while a distribution is being
  # brought up.
  cmd_configure --allow-unresolved
  python3 "$repo/tools/gates/chain-pinned.py" "$target" || exit 1
  "$repo/tools/sign-all" >/dev/null
  echo "== building $target"
  ( cd "$repo/out/pkgs" && "$pm" build "../recipes/$target/build.yaml" )
}

cmd_clean() {
  rm -rf "$repo/out/recipes" "$repo/out/pkgs" "$repo/out/tmp"
  echo "clean: removed generated recipes, packages and workspaces"
  echo "clean: kept out/sources (the fetched tarballs) and .pm-config"
}

case "${1:-}" in
  configure) shift; cmd_configure "$@" ;;
  sign)      shift; "$repo/tools/sign-all" ;;
  lint)      shift; cmd_lint ;;
  check)     shift; cmd_check "$@" ;;
  build)     shift; cmd_build "$@" ;;
  fetch)     shift; python3 "$repo/tools/fetch-sources" "$@" ;;
  serve)     shift; python3 "$repo/tools/serve-sources" --port "$mirror_port" ;;
  clean)     shift; cmd_clean ;;
  *)
    cat <<USAGE
./do <command>

  check [--allow-unresolved]  configure, sign, prove the download-path digest,
                              then lint. The gate. No network, no KVM, no nix.
  configure                   recipes/**/*.in -> out/recipes/**
  sign                        pm sign every generated build file
  lint                        schema, URL form, fingerprints, pm explain
  build [layer]               the real build (default: top of manifest/layers.yaml)
  fetch [--update]            mirror every pinned source into out/sources,
                              filling any TODO hash
  serve                       serve out/sources over loopback for pm
  clean                       remove out/recipes, out/pkgs, out/tmp

Environment:
  PM                   path to the pm binary (default ../pm/target/release/pm)
  LOSOS_MIRROR_PORT    loopback port for the source mirror (default 8730)
USAGE
    ;;
esac
