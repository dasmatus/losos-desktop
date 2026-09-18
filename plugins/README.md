# This distribution's pm plugins

Two WebAssembly components, built against pm's `wit/plugin.wit`
([pm#4](https://github.com/dichhead/pm/pull/4)). pm loads every `*.wasm` in
`$XDG_CONFIG_HOME/pm/plugins/` and asks it two questions its built-in tables
cannot always answer.

Read pm's `plugins/README.md` for the sandbox and the trust model. What follows
is why *these two* exist.

## `losos-image` — classify-command

pm's fingerprint table knows compilers, build systems, archivers and coreutils.
It does not know the tools that turn a staged tree into something bootable,
because those are not build systems. So before this plugin, a recipe calling
`qemu-img` or `xorriso` was refused before a single step ran:

```
$ pm --no-plugins explain build.yaml
Error: no built-in fingerprint matches these commands: `mkfs.vfat …`,
       `mcopy …`, `xorriso …`, `qemu-img …`, `patchelf …`
```

With it, the same file is accepted and every command is attributed:

```
$ pm explain build.yaml
grants:     Toolchain, Coreutils, Archive
plugins:    losos-image 0.1.0

COMMAND                                              FINGERPRINT
mkfs.vfat -F32 -n LOSOS-ESP /build/esp.img           losos-image:mkfs-vfat
mcopy -i /build/esp.img … ::/EFI/BOOT/BOOTX64.EFI    losos-image:mcopy
xorriso -as mkisofs -e esp.img … -o /dest/losos.iso  losos-image:xorriso
qemu-img convert -f raw -O qcow2 … /dest/losos.qcow2 losos-image:qemu-img
patchelf --set-rpath $ORIGIN/../lib /dest/usr/bin/app  losos-image:patchelf
```

Note what is **not** in `grants`: `Network`. The manifest's ceiling is
`Toolchain, Coreutils, Archive`, so pm would drop a network capability even if a
future edit here asked for one by mistake. An image build that reaches the
network is an image build whose contents are not the ones that were reviewed.

`patchelf` is in the list for a different reason than the rest. Setting
`RUNPATH` to `$ORIGIN/../lib` is what lets a package carry its own libraries and
resolve them from inside itself — the AppImage technique, and the missing half
of pm's self-containment. `$ORIGIN` is passed literally: there is no shell in a
build step, so nothing expands it, which is exactly what patchelf wants.

Filesystem builders are named individually (`mkfs.ext4`, not `mkfs*`) so a typo
is an unrecognised command, which pm reports, rather than a wildcard quietly
matching something else.

## `losos-systemd` — scan-source

This one is the interesting half, because it is the only signal in pm that can
be **exact**.

Every other signal is an approximation, and pm's own documentation says so: ELF
analysis reads `DT_NEEDED` and guesses a directory; source scanning finds a
string literal shaped like a path and cannot tell a real one from an error
message. pm's own profile is 56 grants of which most come from path literals in
its dependencies' *test files*.

A systemd unit is not an approximation. `ReadWritePaths=` does not suggest that
a service might write somewhere — it is the list systemd will make writable and
nothing else will be. `StateDirectory=` names the one directory systemd creates.
For a package that ships units, the most accurate description of what it needs
at run time was already inside the package, written by whoever wrote the
service, and nothing was reading it.

A unit of thirteen directives yields twenty-one grants, each with the file and
line it came from:

```
read
  /etc/losos        plugin  covers /etc/losos/probe.env
                            losos-probe.service: losos-systemd: 14: ConfigurationDirectory= /etc/losos
                            losos-probe.service: losos-systemd: 9: EnvironmentFile= /etc/losos/probe.env
write
  /var/log/losos-probe  plugin  losos-probe.service: losos-systemd: 12: LogsDirectory= /var/log/losos-probe
exec
  /usr/bin/losos-probe  plugin  losos-probe.service: losos-systemd: 7: ExecStart= /usr/bin/losos-probe
network
  (granted)  plugin  losos-probe.service: losos-systemd: 17: IPAddressAllow= localhost
```

It is deliberately conservative. The absence of `PrivateNetwork=yes` is **not**
read as evidence that a service needs the network — almost no unit sets it, so
treating absence as evidence would grant the network to everything and mean
nothing. `ExecStart=` prefixes (`-`, `@`, `+`, `!`, `!!`) are stripped, because a
grant naming `-/usr/lib/losos/prepare` names a file that does not exist. `%`
specifiers are left alone rather than guessed at. Continued lines (trailing `\`)
are skipped rather than half-read.

`.target` and `.slice` are not scanned: they carry ordering and resource limits,
never a path.

## Building

```sh
rustup target add wasm32-unknown-unknown
./build.sh                       # both, into dist/
./build.sh losos-image           # one

pm sign "$XDG_CONFIG_HOME/pm/plugins/losos-image.wasm"
pm plugins                       # confirm both loaded and signed
```

`tools/gates/plugins.py` checks that `wit/plugin.wit` here still matches pm's
and that no component is older than its source. Neither is something pm can
catch: from pm's side, a stale plugin is simply a plugin.

That check compares bytes, so it is only as stable as the pm it compares
against. CI therefore checks pm out at a pinned commit —
`145bae94bf170a9779029abdb41777ea380dc9f0`, the one `wit/plugin.wit` here is a
copy of — rather than at `master`. While it followed `master`, the contract
changing upstream turned every open pull request red inside half an hour, on
trees nobody had touched. Bumping that pin in `.github/workflows/images.yml`,
re-copying `wit/plugin.wit` from the same commit and re-running `./do plugins`
is one change rather than three; `tools/gates/workflow.py` holds both checkouts
in that file to the same pin.
