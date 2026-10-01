# Short names for what CI runs. Every recipe is one nix command, or the plugin
# build; `nix` itself is the interface, and these only save typing it.
set shell := ["bash", "-euo", "pipefail", "-c"]

# `just plugins -- losos-nix` forwards the `--` as the first argument; the
# plugin Justfile drops it.
set positional-arguments

repo := canonicalize(justfile_directory())

default:
  @just --list

# The disk image, or any other flake output (`just build installer`).
build target="image":
  nix build -L ".#{{target}}"

# Both architectures evaluated, losos-security's tests, and on a builder with
# KVM the VM boot test.
check:
  nix flake check -L

fmt:
  nix fmt

# Boot the image in QEMU with UEFI firmware.
vm:
  nix run .#vm

# pm's plugins as WebAssembly components, into plugins/dist. The image builds
# its own copies (nixos/pkgs/pm-plugins.nix); this is for a pm outside it.
plugins *args:
  just --justfile "{{repo}}/plugins/Justfile" --working-directory "{{repo}}" build "$@"
