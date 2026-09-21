# This `set shell` governs the *plain* recipes only. A recipe whose body starts
# with a shebang is written to a file and executed, so it gets bash's defaults
# and none of these flags -- which is why every shebang body below repeats
# `set -euo pipefail` on its own line. Without it a failing command is ignored
# and the recipe still exits 0, and `check` in particular printed "check: green"
# over a configure step that had already failed. A gate that cannot go red is
# worse than no gate, because CI reports it as proof.
set shell := ["bash", "-euo", "pipefail", "-c"]

# `*args` recipes are invoked as `just <recipe> -- --flag`, because without the
# separator just parses `--flag` as an option to itself. just does not consume
# the separator, though: it forwards it as the recipe's first argument, and
# Python's argparse then reports every flag after it as unrecognized. So each
# such recipe drops one leading `--` before it forwards anything.
set positional-arguments

# `canonicalize` rather than the directory just was handed, because this path
# is also a path *inside the container*. `tools/container` resolves the
# repository with `Path.resolve()` and bind-mounts it at that resolved path, so
# on any host where the checkout is reached through a symlink -- `/home` is one
# on Fedora Silverblue and every other ostree system, pointing at `/var/home` --
# the two disagree, and the recursive `just --justfile {{repo}}/Justfile` the
# container recipes run asks for a path the container does not have:
#
#   error: Failed to read justfile at `/home/<user>/.../Justfile`:
#   No such file or directory (os error 2)
#
# which reads as a missing Justfile and is a missing *mount*. `tools/configure`
# already resolves the same way when it bakes absolute paths into the generated
# recipes (C1), so canonicalising here is also what keeps those two agreeing.
# Needs just >= 1.24 for the function; `docs/host-requirements.md` says so.
repo := canonicalize(justfile_directory())
pm := env_var_or_default("PM", repo + "/../pm/target/release/pm")
pm_root := env_var_or_default("PM_ROOT", repo + "/../pm")
mirror_port := env_var_or_default("LOSOS_MIRROR_PORT", "8730")
mirror_url := "http://127.0.0.1:" + mirror_port

export TMPDIR := repo + "/out/tmp"
export XDG_CONFIG_HOME := repo + "/.pm-config"

default:
  #!/usr/bin/env bash
  set -euo pipefail
  printf '%s\n' \
    'just <command>' \
    '' \
    '  check [args...]            configure, sign, prove the download-path digest,' \
    '                             then lint. The gate. No network, no KVM, no nix.' \
    '  configure [args...]        recipes/**/*.in -> out/recipes/**' \
    '  sign                       pm sign every generated build file' \
    '  digest                     prove configure'\''s download path against pm' \
    '  lint                       schema, URL form, fingerprints, pm explain' \
    '  build [layer]              the real build (default: top of manifest/layers.yaml)' \
    '  fetch [args...]            mirror every pinned source into out/sources,' \
    '                             filling any TODO hash' \
    '  serve                      serve out/sources over loopback for pm' \
    '  plugins [crate]            build the pm plugin components into plugins/dist' \
    '                             (needs the wasm32 Rust target; not part of check)' \
    '  packages [args...]         publish/pull signed .cpkg containers with pm-oci' \
    '  clean                      remove out/recipes, out/pkgs, out/tmp' \
    '  container pull             download or refresh the published build host' \
    '  container build            explicitly rebuild the image locally' \
    '  container [command]        run an arbitrary command inside it, or a shell' \
    '                             with no command. The image is the build host:' \
    '                             it is what docs/host-requirements.md asks for,' \
    '                             pinned.' \
    '  container-check [args...]  run `just check` inside the container host' \
    '  container-plugins [args...]' \
    '                             run `just plugins` inside the container host' \
    '  container-run <command>    run an arbitrary command inside the container host' \
    '' \
    'Environment:' \
    '  LOSOS_CONTAINER_IMAGE  build-host image tag or digest (default GHCR latest)' \
    '  PM                   path to the pm binary (default ../pm/target/release/pm)' \
    '  LOSOS_MIRROR_PORT    loopback port for the source mirror (default 8730)' \
    '  PM_ROOT              path to the sibling pm checkout (default ../pm). Two' \
    '                       gates read pm'\''s source, not its binary, and both' \
    '                       downgrade to a pass when they cannot find it.'

configure *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ -d "{{repo}}/out/sources" ]; then
    python3 "{{repo}}/tools/configure" --mirror "{{mirror_url}}" "$@"
  else
    python3 "{{repo}}/tools/configure" "$@"
  fi

sign:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  "{{repo}}/tools/sign-all"

digest:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ ! -x "{{pm}}" ]; then
    echo "no pm binary at {{pm}}" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi
  PM="{{pm}}" python3 "{{repo}}/tools/check-digest"

lint:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ ! -x "{{pm}}" ]; then
    echo "no pm binary at {{pm}}" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi
  python3 "{{repo}}/tools/gates/schema.py"
  python3 "{{repo}}/tools/gates/url-canonical.py"
  python3 "{{repo}}/tools/gates/fingerprint-lint.py" --check-table
  python3 "{{repo}}/tools/gates/fingerprint-lint.py"
  python3 "{{repo}}/tools/gates/test-image.py"
  python3 "{{repo}}/tools/gates/test-swap.py"
  python3 "{{repo}}/tools/gates/test-media.py"
  python3 "{{repo}}/tools/gates/test-libvirt.py"
  python3 "{{repo}}/tools/gates/test-container.py"
  python3 "{{repo}}/tools/gates/test-oci.py"
  python3 "{{repo}}/tools/stage-release" --arch x86_64 --version 0.0.0 --check >/dev/null
  python3 "{{repo}}/tools/gates/plugins.py"
  python3 "{{repo}}/tools/gates/workflow.py" --self-test
  python3 "{{repo}}/tools/gates/workflow.py"
  python3 "{{repo}}/tools/gates/test-prebuilt.py"
  python3 -m basedpyright --project "{{repo}}/basedpyrightconfig.json"
  python3 "{{repo}}/tools/gates/patches.py"
  python3 "{{repo}}/tools/gates/cross-configure.py"
  python3 "{{repo}}/tools/gates/exceptions-live.py"
  python3 "{{repo}}/tools/gates/versions.py" --self-test
  # The gate itself needs the mirrored tarballs and so runs from `just build`.
  # Its reader runs here, against recipes and archives made for the purpose,
  # because a gate ./do check never exercises is one a refactor can quietly
  # turn into a gate that agrees with everything.
  python3 "{{repo}}/tools/gates/recipe-entrypoints.py" --self-test
  python3 "{{repo}}/tools/check-latest" --self-test >/dev/null
  "{{repo}}/tools/gates/explain-all"

check *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ ! -x "{{pm}}" ]; then
    echo "no pm binary at {{pm}}" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi
  echo "== configure"
  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" configure -- --allow-unresolved "$@"
  echo "== sign"
  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" sign >/dev/null
  echo "== digest agreement with pm"
  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" digest
  echo "== lint"
  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" lint
  echo
  echo "check: green"

build target="":
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  pm_bin="${PM:-{{pm}}}"
  if [ ! -x "$pm_bin" ]; then
    echo "no pm binary at $pm_bin" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi

  target="{{target}}"
  top_target=$(python3 -c 'import pathlib, yaml; layers = yaml.safe_load(pathlib.Path("'"{{repo}}"'/manifest/layers.yaml").read_text()) or []; print(layers[-1]["name"] if layers else "")')
  if [ -z "$target" ]; then
    target="$top_target"
  fi
  [ -n "$target" ] || { echo "nothing to build" >&2; exit 1; }

  mirror_running() {
    python3 "{{repo}}/tools/serve-sources" --port "{{mirror_port}}" --check
  }

  start_mirror() {
    if mirror_running; then return 0; fi
    if [ ! -d "{{repo}}/out/sources" ]; then return 1; fi
    python3 "{{repo}}/tools/serve-sources" --port "{{mirror_port}}" >"{{repo}}/out/tmp/mirror.log" 2>&1 &
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      mirror_running && return 0
      sleep 0.3
    done
    return 1
  }

  recipe_needs_image_host() {
    local recipe="{{repo}}/out/recipes/$1/build.yaml"
    [ -f "$recipe" ] || return 1
    grep -Eq '(%\{losos-mkosi:|(^|[[:space:]])mkosi([[:space:]]|$)|(^|[[:space:]])qemu-img([[:space:]]|$)|(^|[[:space:]])xorriso([[:space:]]|$))' "$recipe"
  }

  image_build_prereqs_ready() {
    local mkosi_path mkosi_resolved qemu_path xorriso_path
    mkosi_path=$(command -v mkosi 2>/dev/null) || return 1
    qemu_path=$(command -v qemu-img 2>/dev/null) || return 1
    xorriso_path=$(command -v xorriso 2>/dev/null) || return 1
    case "$qemu_path" in /bin/*|/sbin/*|/usr/bin/*|/usr/sbin/*) ;; *) return 1 ;; esac
    case "$xorriso_path" in /bin/*|/sbin/*|/usr/bin/*|/usr/sbin/*) ;; *) return 1 ;; esac
    [ "$mkosi_path" = "/usr/bin/mkosi" ] || return 1
    [ -L "$mkosi_path" ] || return 1
    mkosi_resolved=$(readlink -f "$mkosi_path") || return 1
    [ "$mkosi_resolved" = "/usr/lib/mkosi/bin/mkosi" ] || return 1
    [ -f "{{repo}}/plugins/dist/losos-image.wasm" ] || return 1
    [ -f "{{repo}}/plugins/dist/losos-mkosi.wasm" ] || return 1
  }

  build_in_container() {
    local args_backup status
    echo "just: host lacks the pinned mkosi image-build prerequisites; building $1 in the container host" >&2
    # configure rewrites sources to 127.0.0.1:$LOSOS_MIRROR_PORT when the local
    # mirror exists, so the fallback container needs the host network to reach it.
    # The recursive `just build` reads out/configure.args back from disk, so keep
    # the mounted copy in sync with the effective configure arguments here, then
    # restore the host's remembered configuration once the nested build exits.
    args_backup=''
    if [ -f "{{repo}}/out/configure.args" ]; then
      args_backup="{{repo}}/out/tmp/configure.args.container.$$"
      cp "{{repo}}/out/configure.args" "$args_backup"
    fi
    if [ "${#configure_args[@]}" -gt 0 ]; then
      printf '%s\n' "${configure_args[@]}" > "{{repo}}/out/configure.args"
    else
      : > "{{repo}}/out/configure.args"
    fi
    if PM="$pm_bin" PM_ROOT="{{pm_root}}" LOSOS_CONTAINER_HOST_NETWORK=1 python3 "{{repo}}/tools/container" run /bin/sh -eu -c 'LOSOS_MIRROR_PORT="$1"; LOSOS_SKIP_IMAGE_HOST_FALLBACK=1; export LOSOS_MIRROR_PORT LOSOS_SKIP_IMAGE_HOST_FALLBACK PM PM_ROOT; if [ ! -f "{{repo}}/plugins/dist/losos-image.wasm" ] || [ ! -f "{{repo}}/plugins/dist/losos-mkosi.wasm" ]; then just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" plugins; fi; just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" build "$2"' _ "{{mirror_port}}" "$1"; then
      status=0
    else
      status=$?
    fi
    if [ -n "$args_backup" ]; then
      mv "$args_backup" "{{repo}}/out/configure.args"
    else
      rm -f "{{repo}}/out/configure.args"
    fi
    return "$status"
  }

  start_mirror || echo "just: no local source mirror; pm will fetch upstream" >&2

  remembered=()
  if [ -f "{{repo}}/out/configure.args" ]; then
    while IFS= read -r line; do
      if [ -n "$line" ]; then remembered+=("$line"); fi
    done < "{{repo}}/out/configure.args"
  fi

  configure_args=("${remembered[@]}")
  have_allow_unresolved=0
  for arg in "${configure_args[@]}"; do
    if [ "$arg" = "--allow-unresolved" ]; then have_allow_unresolved=1; fi
  done
  if [ "$target" != "$top_target" ] && [ "$have_allow_unresolved" -eq 0 ]; then
    configure_args=(--allow-unresolved "${configure_args[@]}")
  fi
  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" configure -- "${configure_args[@]+"${configure_args[@]}"}"
  python3 "{{repo}}/tools/gates/chain-pinned.py" "$target" || exit 1
  # The one gate that needs the bytes, so it cannot be in `just check`: it asks
  # whether each recipe's build system is actually in the tarball it pins. Here
  # because the mirror is up by now and pm has not started; it costs seconds and
  # it is the difference between reading every such mistake at once and finding
  # them one hour-long build at a time. A source not mirrored is skipped and
  # counted, so a partial mirror weakens the report without failing it.
  python3 "{{repo}}/tools/gates/recipe-entrypoints.py" || exit 1

  if [ -z "${LOSOS_SKIP_IMAGE_HOST_FALLBACK:-}" ] && recipe_needs_image_host "$target" && ! image_build_prereqs_ready; then
    build_in_container "$target"
    exit 0
  fi

  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" sign >/dev/null
  echo "== building $target"
  (
    cd "{{repo}}/out/pkgs"
    "$pm_bin" build "../recipes/$target/build.yaml"
  )

fetch *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  python3 "{{repo}}/tools/fetch-sources" "$@"

packages *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/pm-oci" "$@"

serve:
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  exec python3 "{{repo}}/tools/serve-sources" --port "{{mirror_port}}"

plugins *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  "{{repo}}/plugins/build.sh" "$@"

clean:
  rm -rf "{{repo}}/out/recipes" "{{repo}}/out/pkgs" "{{repo}}/out/tmp"
  echo "clean: removed generated recipes, packages and workspaces"
  echo "clean: kept out/sources (the fetched tarballs) and .pm-config"

container-build *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/container" build "$@"

container-run *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/container" run "$@"

container-check *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  container_pm="{{pm}}"
  case "$container_pm" in
    "{{pm_root}}"/*) ;;
    *) container_pm='' ;;
  esac
  PM_ROOT="{{pm_root}}" python3 "{{repo}}/tools/container" run /bin/sh -eu -c 'if [ -n "${1:-}" ]; then PM="$1"; export PM; fi; shift; if [ $# -eq 0 ]; then exec just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" check; fi; case "$1" in -*) exec just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" check -- "$@" ;; *) exec just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" check "$@" ;; esac' _ "$container_pm" "$@"

container-plugins *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  container_pm="{{pm}}"
  case "$container_pm" in
    "{{pm_root}}"/*) ;;
    *) container_pm='' ;;
  esac
  PM_ROOT="{{pm_root}}" python3 "{{repo}}/tools/container" run /bin/sh -eu -c 'if [ -n "${1:-}" ]; then PM="$1"; export PM; fi; shift; if [ $# -eq 0 ]; then exec just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" plugins; fi; case "$1" in -*) exec just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" plugins -- "$@" ;; *) exec just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" plugins "$@" ;; esac' _ "$container_pm" "$@"

container *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  if [ $# -eq 0 ]; then
    python3 "{{repo}}/tools/container" run
  elif [ "$1" = build ] || [ "$1" = pull ]; then
    python3 "{{repo}}/tools/container" "$@"
  elif [ "$1" = check ]; then
    shift
    just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" container-check "$@"
  elif [ "$1" = plugins ]; then
    shift
    just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" container-plugins "$@"
  else
    python3 "{{repo}}/tools/container" run "$@"
  fi

check-latest *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/check-latest" "$@"

release-manifest *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/release-manifest" "$@"

stage-release *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/stage-release" "$@"

toolchain-report *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/gates/toolchain-report.py" "$@"

vm-test *args:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "${1:-}" = "--" ]; then shift; fi
  python3 "{{repo}}/tools/vm-test" "$@"
