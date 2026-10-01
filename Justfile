# Short names for the commands CI runs. Each recipe is one nix command, except
# `plugins`. Running nix directly does the same thing.
set shell := ["bash", "-euo", "pipefail", "-c"]

# `just plugins -- losos-nix` forwards the `--` as the first argument, and the
# plugin Justfile drops it.
set positional-arguments

repo := canonicalize(justfile_directory())

default:
  @just --list

# The disk image, or any other flake output (`just build installer`).
build target="image":
  nix build -L ".#{{target}}"

# Evaluates both architectures, runs losos-security's tests, and on a builder
# with KVM boots the image in a VM.
check:
  nix flake check -L

fmt:
  nix fmt

# Boot the image in QEMU with UEFI firmware.
vm:
  nix run .#vm

# Builds pm's plugins as WebAssembly components into plugins/dist, for a pm
# installed outside the image. The image builds its own copies in
# nixos/pkgs/pm-plugins.nix.
plugins *args:
  just --justfile "{{repo}}/plugins/Justfile" --working-directory "{{repo}}" build "$@"
