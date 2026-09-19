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

recipe_needs_image_host() {
  local recipe="$repo/out/recipes/$1/build.yaml"
  [ -f "$recipe" ] || return 1
  grep -Eq '(%\{losos-mkosi:|(^|[[:space:]])mkosi([[:space:]]|$)|(^|[[:space:]])qemu-img([[:space:]]|$)|(^|[[:space:]])xorriso([[:space:]]|$))' "$recipe"
}

image_build_prereqs_ready() {
  command -v mkosi >/dev/null 2>&1 || return 1
  command -v qemu-img >/dev/null 2>&1 || return 1
  command -v xorriso >/dev/null 2>&1 || return 1
  [ -f "$repo/plugins/dist/losos-image.wasm" ] || return 1
  [ -f "$repo/plugins/dist/losos-mkosi.wasm" ] || return 1
}

build_in_container() {
  local target="$1"
  echo "do: host lacks the pinned mkosi image-build prerequisites; building $target in the container host" >&2
  python3 "$repo/tools/container" build
  python3 "$repo/tools/container" run /bin/sh -eu -c './do plugins && ./do build "$1"' _ "$target"
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
  # assert-media.py is the build's last word on whether the disk and the ISO
  # came out right, and it runs once, four hours in, on the real artefacts.
  # Wrong in the accepting direction it never says so: the build goes green
  # and the failure surfaces as a machine that powers on to an empty boot
  # menu. So it is fed images written from the format here instead.
  python3 "$repo/tools/gates/test-media.py"
  # The libvirt domain, checked for what it must NOT hand the guest. A domain
  # that boots a kernel the host supplied shows a desktop and says nothing
  # about the image's own partition table.
  python3 "$repo/tools/gates/test-libvirt.py"
  # The release names and the shipped sysupdate MatchPatterns are one contract
  # written in two files. A mismatch does not fail an update -- sysupdate
  # reports "no update available", which is indistinguishable from being up to
  # date -- so it has to be caught here.
  python3 "$repo/tools/stage-release" --arch x86_64 --version 0.0.0 --check >/dev/null
  python3 "$repo/tools/gates/plugins.py"
  # An artifact name mismatch between two CI jobs fails only on the release
  # path, which is the one nobody exercises until it matters.
  python3 "$repo/tools/gates/workflow.py"
  # A patch series nobody applies is tracked, reviewed and inert: the build
  # goes green and the feature is simply absent.
  python3 "$repo/tools/gates/patches.py"
  # A recipe whose `version:` no longer matches the source it downloads builds
  # the new tarball under the old name, and nothing else notices.
  # --self-test rather than a bare run: it covers the real tree too, and
  # adds the cases that prove the gate can fail. A gate never shown to
  # fail is a gate nobody should trust.
  python3 "$repo/tools/gates/versions.py" --self-test
  # tools/check-latest writes sources.lock and recipe versions from what
  # upstream answers, and the part that can be wrong without saying so is the
  # matching: a pattern that stops matching reports a source as `current`
  # forever. Its self-test needs no network, so it runs here rather than only
  # in the workflow that uses it.
  python3 "$repo/tools/check-latest" --self-test >/dev/null
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
  # Regenerate with the settings the last configure was given, not with the
  # defaults. The generated tree carries an architecture, a channel and a
  # version substituted into it and says so nowhere, so re-generating with the
  # defaults silently turns an aarch64 tree into an x86_64 one and builds it.
  local remembered=()
  if [ -f "$repo/out/configure.args" ]; then
    while IFS= read -r line; do
      [ -n "$line" ] && remembered+=("$line")
    done < "$repo/out/configure.args"
  fi
  # Generate the whole tree, then insist only that the layers this build
  # actually walks are pinned. Refusing because some unrelated upper layer has
  # an unfetched source would make a partial tree unbuildable for no reason --
  # and a partial tree is the normal state while a distribution is being
  # brought up.
  cmd_configure --allow-unresolved "${remembered[@]+"${remembered[@]}"}"
  python3 "$repo/tools/gates/chain-pinned.py" "$target" || exit 1
  if recipe_needs_image_host "$target" && ! image_build_prereqs_ready; then
    build_in_container "$target"
    return
  fi
  "$repo/tools/sign-all" >/dev/null
  echo "== building $target"
  ( cd "$repo/out/pkgs" && "$pm" build "../recipes/$target/build.yaml" )
}

# Build the plugin components from source.
#
# Kept out of `check` on purpose. The components need the wasm32 Rust target and
# pm's encoder, and building the encoder needs crates.io -- while `./do check`
# is meant to run on any machine with no network and no wasm toolchain. So this
# is its own verb: CI runs it before check, a developer runs it after touching
# plugins/, and everyone else never needs it. Nothing reads a component out of
# the tree, because none is committed.
cmd_plugins() {
  "$repo/plugins/build.sh" "$@"
}

cmd_clean() {
  rm -rf "$repo/out/recipes" "$repo/out/pkgs" "$repo/out/tmp"
  echo "clean: removed generated recipes, packages and workspaces"
  echo "clean: kept out/sources (the fetched tarballs) and .pm-config"
}

# Every command above assumes the host provides docs/host-requirements.md.
# This one provides it instead: `./do container check` runs the gate inside the
# image the Containerfile describes, which is the same image CI uses.
cmd_container() {
  local verb="${1:-}"
  case "$verb" in
    build) shift; python3 "$repo/tools/container" build "$@" ;;
    "")    python3 "$repo/tools/container" run ;;
    *)     python3 "$repo/tools/container" run ./do "$@" ;;
  esac
}

case "${1:-}" in
  configure) shift; cmd_configure "$@" ;;
  sign)      shift; "$repo/tools/sign-all" ;;
  lint)      shift; cmd_lint ;;
  check)     shift; cmd_check "$@" ;;
  build)     shift; cmd_build "$@" ;;
  fetch)     shift; python3 "$repo/tools/fetch-sources" "$@" ;;
  serve)     shift; python3 "$repo/tools/serve-sources" --port "$mirror_port" ;;
  plugins)   shift; cmd_plugins "$@" ;;
  clean)     shift; cmd_clean ;;
  container) shift; cmd_container "$@" ;;
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
  plugins [crate]             build the pm plugin components into plugins/dist
                              (needs the wasm32 Rust target; not part of check)
  clean                       remove out/recipes, out/pkgs, out/tmp
  container build             build the image the Containerfile describes
  container [command]         run ./do <command> inside it, or a shell with
                              no command. The image is the build host: it is
                              what docs/host-requirements.md asks for, pinned.

Environment:
  PM                   path to the pm binary (default ../pm/target/release/pm)
  LOSOS_MIRROR_PORT    loopback port for the source mirror (default 8730)
  PM_ROOT              path to the sibling pm checkout (default ../pm). Two
                       gates read pm's source, not its binary, and both
                       downgrade to a pass when they cannot find it.
USAGE
    ;;
esac
