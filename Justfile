set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

repo := justfile_directory()
pm := env_var_or_default("PM", repo + "/../pm/target/release/pm")
mirror_port := env_var_or_default("LOSOS_MIRROR_PORT", "8730")
mirror_url := "http://127.0.0.1:" + mirror_port

export TMPDIR := repo + "/out/tmp"
export XDG_CONFIG_HOME := repo + "/.pm-config"

default:
  #!/usr/bin/env bash
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
    '  clean                      remove out/recipes, out/pkgs, out/tmp' \
    '  container build            build the image the Containerfile describes' \
    '  container [command]        run `just <command>` inside it, or a shell with' \
    '                             no command. The image is the build host: it is' \
    '                             what docs/host-requirements.md asks for, pinned.' \
    '  container-run <command>    run an arbitrary command inside the container host' \
    '' \
    'Environment:' \
    '  PM                   path to the pm binary (default ../pm/target/release/pm)' \
    '  LOSOS_MIRROR_PORT    loopback port for the source mirror (default 8730)' \
    '  PM_ROOT              path to the sibling pm checkout (default ../pm). Two' \
    '                       gates read pm'\''s source, not its binary, and both' \
    '                       downgrade to a pass when they cannot find it.'

configure *args:
  #!/usr/bin/env bash
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ -d "{{repo}}/out/sources" ]; then
    python3 "{{repo}}/tools/configure" --mirror "{{mirror_url}}" "$@"
  else
    python3 "{{repo}}/tools/configure" "$@"
  fi

sign:
  #!/usr/bin/env bash
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  "{{repo}}/tools/sign-all"

digest:
  #!/usr/bin/env bash
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ ! -x "{{pm}}" ]; then
    echo "no pm binary at {{pm}}" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi
  PM="{{pm}}" python3 "{{repo}}/tools/check-digest"

lint:
  #!/usr/bin/env bash
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
  python3 "{{repo}}/tools/gates/test-media.py"
  python3 "{{repo}}/tools/gates/test-libvirt.py"
  python3 "{{repo}}/tools/stage-release" --arch x86_64 --version 0.0.0 --check >/dev/null
  python3 "{{repo}}/tools/gates/plugins.py"
  python3 "{{repo}}/tools/gates/workflow.py"
  python3 "{{repo}}/tools/gates/patches.py"
  python3 "{{repo}}/tools/gates/versions.py" --self-test
  python3 "{{repo}}/tools/check-latest" --self-test >/dev/null
  "{{repo}}/tools/gates/explain-all"

check *args:
  #!/usr/bin/env bash
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
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  if [ ! -x "{{pm}}" ]; then
    echo "no pm binary at {{pm}}" >&2
    echo "build it first:  cd ../pm && cargo build --release" >&2
    exit 1
  fi

  target="${1:-}"
  if [ -z "$target" ]; then
    target=$(python3 -c 'import pathlib, yaml; layers = yaml.safe_load(pathlib.Path("'"{{repo}}"'/manifest/layers.yaml").read_text()) or []; print(layers[-1]["name"] if layers else "")')
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
    echo "just: host lacks the pinned mkosi image-build prerequisites; building $1 in the container host" >&2
    just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" container-build
    # configure rewrites sources to 127.0.0.1:$LOSOS_MIRROR_PORT when the local
    # mirror exists, so the fallback container needs the host network to reach it.
    LOSOS_CONTAINER_HOST_NETWORK=1     python3 "{{repo}}/tools/container" run /bin/sh -eu -c 'LOSOS_MIRROR_PORT="$1"; LOSOS_SKIP_IMAGE_HOST_FALLBACK=1; PM="$2"; export LOSOS_MIRROR_PORT LOSOS_SKIP_IMAGE_HOST_FALLBACK PM PM_ROOT; if [ ! -f "{{repo}}/plugins/dist/losos-image.wasm" ] || [ ! -f "{{repo}}/plugins/dist/losos-mkosi.wasm" ]; then just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" plugins; fi; just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" build "$3"' _ "{{mirror_port}}" "{{pm}}" "$1"
  }

  start_mirror || echo "just: no local source mirror; pm will fetch upstream" >&2

  remembered=()
  if [ -f "{{repo}}/out/configure.args" ]; then
    while IFS= read -r line; do
      [ -n "$line" ] && remembered+=("$line")
    done < "{{repo}}/out/configure.args"
  fi

  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" configure -- --allow-unresolved "${remembered[@]+"${remembered[@]}"}"
  python3 "{{repo}}/tools/gates/chain-pinned.py" "$target" || exit 1

  if [ -z "${LOSOS_SKIP_IMAGE_HOST_FALLBACK:-}" ] && recipe_needs_image_host "$target" && ! image_build_prereqs_ready; then
    build_in_container "$target"
    exit 0
  fi

  just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" sign >/dev/null
  echo "== building $target"
  (
    cd "{{repo}}/out/pkgs"
    "{{pm}}" build "../recipes/$target/build.yaml"
  )

fetch *args:
  #!/usr/bin/env bash
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  python3 "{{repo}}/tools/fetch-sources" "$@"

serve:
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  exec python3 "{{repo}}/tools/serve-sources" --port "{{mirror_port}}"

plugins *args:
  #!/usr/bin/env bash
  mkdir -p "{{repo}}/out/tmp" "{{repo}}/out/pkgs"
  "{{repo}}/plugins/build.sh" "$@"

clean:
  rm -rf "{{repo}}/out/recipes" "{{repo}}/out/pkgs" "{{repo}}/out/tmp"
  echo "clean: removed generated recipes, packages and workspaces"
  echo "clean: kept out/sources (the fetched tarballs) and .pm-config"

container-build *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/container" build "$@"

container-run *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/container" run "$@"

container *args:
  #!/usr/bin/env bash
  if [ $# -eq 0 ]; then
    python3 "{{repo}}/tools/container" run
  elif [ "$1" = build ]; then
    shift
    python3 "{{repo}}/tools/container" build "$@"
  else
    python3 "{{repo}}/tools/container" run just --justfile "{{repo}}/Justfile" --working-directory "{{repo}}" "$@"
  fi

check-latest *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/check-latest" "$@"

release-manifest *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/release-manifest" "$@"

stage-release *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/stage-release" "$@"

toolchain-report *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/gates/toolchain-report.py" "$@"

vm-test *args:
  #!/usr/bin/env bash
  python3 "{{repo}}/tools/vm-test" "$@"
