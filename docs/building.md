# Building

Clone with `git clone --recurse-submodules`: derisk, mcsapi and pm are git
submodules under `components/` ([What this trusts from
outside](trust.md#the-components-submodules)).

Image-producing outputs require `LOSOS_PROXY_URL` to point at the deployment
that serves updates. Since the flake reads this environment variable, pass
`--impure` when building an image, installer, QCOW2, release, UKI, or VM.

```sh
nix build --impure              # the disk image: dd it to a disk and boot
nix build .#installer --impure  # the installer ISO
nix build .#release --impure    # the files CI publishes through the proxy
nix run .#vm --impure           # boot the image in QEMU with UEFI firmware
nix flake check        # both architectures, losos-security's and
                       # losos-installer's tests, and
                       # (on a builder with KVM) a VM boot test
```

The stock nixpkgs closure is substituted from cache.nixos.org when available;
the image and project-specific packages are built locally unless present in
the project's cache ([Binary cache](binary-cache.md)). The image is built by `systemd-repart` inside
the build sandbox; it needs no loop device, no root and no KVM. Only the VM
tests need KVM.

CI builds every other check first and boots the VM tests last. A VM test
that fails there is a warning and a line in the run's summary, not a red
run, so a boot that breaks under the runner's KVM does not hold back the
nightly. Run `nix flake check` locally to see one fail for real.

`just` has short names for the same commands. `just plugins` builds pm's
plugin components for a pm installed outside the image ([pm on
LosOS](pm.md)).

Before trusting a change to `nixos/`, evaluate every target; that runs every
module and assertion without building an image:

```sh
nix eval --raw .#nixosConfigurations.losos-desktop-x86_64.config.system.build.toplevel.drvPath
nix eval --raw .#nixosConfigurations.losos-desktop-aarch64.config.system.build.toplevel.drvPath
nix eval --raw .#nixosConfigurations.losos-desktop-gsi-aarch64.config.system.build.toplevel.drvPath
```

`nix fmt` formats the Nix files; CI runs `nix fmt -- --ci`.
