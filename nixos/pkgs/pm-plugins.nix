# pm's plugins as WebAssembly components: the same seven `just plugins` builds
# for the pm tree -- this repository's four and the three from pm's own tree
# that this OS needs (plugins/Justfile says which and why).
#
# Installed under share/pm/plugins and deliberately nowhere pm loads from. pm
# reads only <config>/pm/plugins and runs a plugin only when it is signed by a
# key the user trusts, so shipping them in the image offers them without
# trusting them on anyone's behalf: a user who wants them copies them into
# their own config and signs them there (docs/nixos.md, "pm's plugins").
{
  lib,
  stdenv,
  rustPlatform,
  rustc,
  cargo,
  lld,
  pm,
}:

let
  # This repository's plugin workspace. Its lock pins crates.io only, so
  # importCargoLock needs no hash.
  ours = lib.fileset.toSource {
    root = ../../plugins;
    fileset = lib.fileset.unions [
      ../../plugins/Cargo.toml
      ../../plugins/Cargo.lock
      ../../plugins/wit
      ../../plugins/losos-image
      ../../plugins/losos-mkosi
      ../../plugins/losos-nix
      ../../plugins/losos-systemd
    ];
  };
  oursDeps = rustPlatform.importCargoLock { lockFile = ../../plugins/Cargo.lock; };

  # pm's plugin workspace, from the same commit as pm itself. Its lock lives
  # in the fetched source, so it is vendored by hash rather than read at
  # evaluation time.
  pmDeps = rustPlatform.fetchCargoVendor {
    name = "pm-plugins";
    src = pm.src;
    sourceRoot = "${pm.src.name}/plugins";
    hash = "sha256-OK+QHCUSv6hCWRtUs7p8aVi08mmft3b5oQ1MmqrVyeQ=";
  };

  local = [
    "losos-image"
    "losos-mkosi"
    "losos-nix"
    "losos-systemd"
  ];
  fromPm = [
    "sysext"
    "sysupdate"
    "systemd"
  ];
  packages = crates: lib.concatMapStringsSep " " (c: "-p ${c}") crates;
in
stdenv.mkDerivation {
  pname = "pm-plugins";
  inherit (pm) version;

  srcs = [
    ours
    pm.src
  ];
  sourceRoot = ".";
  unpackPhase = ''
    runHook preUnpack
    cp -r ${ours} ours
    cp -r ${pm.src} pm
    chmod -R u+w ours pm
    runHook postUnpack
  '';

  # nixpkgs' rustc links wasm32 with `lld` from PATH rather than a bundled
  # rust-lld.
  nativeBuildInputs = [
    rustc
    cargo
    lld
  ];

  # Two workspaces, each pointed at its own vendored crates; no network.
  configurePhase = ''
    runHook preConfigure
    export CARGO_HOME=$PWD/cargo-home
    vendor() {
      mkdir -p "$1/.cargo"
      cat > "$1/.cargo/config.toml" <<EOF
    [source.crates-io]
    replace-with = "vendored"
    [source.vendored]
    directory = "$2"
    EOF
    }
    vendor ours ${oursDeps}
    # fetchCargoVendor keeps crates.io crates one level down.
    vendor pm/plugins ${pmDeps}/source-registry-0
    runHook postConfigure
  '';

  buildPhase = ''
    runHook preBuild
    (cd ours && cargo build --offline --release --target wasm32-unknown-unknown ${packages local})
    (cd pm/plugins && cargo build --offline --release --target wasm32-unknown-unknown ${packages fromPm})
    # The encoder wraps a core module into a component; it runs here, on the
    # build machine, so it is the one native build.
    (cd pm/plugins && cargo build --offline --release -p encoder)
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/share/pm/plugins
    encoder=pm/plugins/target/release/encoder
    for crate in ${lib.concatStringsSep " " local}; do
      $encoder ours/target/wasm32-unknown-unknown/release/''${crate//-/_}.wasm \
        $out/share/pm/plugins/$crate.wasm
    done
    for crate in ${lib.concatStringsSep " " fromPm}; do
      $encoder pm/plugins/target/wasm32-unknown-unknown/release/''${crate//-/_}.wasm \
        $out/share/pm/plugins/$crate.wasm
    done
    runHook postInstall
  '';

  meta = {
    description = "pm's plugin components for LosOS Desktop, unsigned";
    license = [
      lib.licenses.agpl3Plus
      lib.licenses.gpl3Only
    ];
    platforms = lib.platforms.linux;
  };
}
