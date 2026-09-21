# This distribution's pm plugins

Three WebAssembly components, built against pm's `wit/plugin.wit`
([pm#4](https://github.com/dichhead/pm/pull/4)). pm loads every `*.wasm` in
`$XDG_CONFIG_HOME/pm/plugins/` and asks it two questions its built-in tables
cannot always answer.

Read pm's `plugins/README.md` for the sandbox and the trust model. What follows
is why *these three* exist.

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

## `losos-mkosi` — classify-command, and symbols

`mkosi` is not in pm's fingerprint table either — `docs/pm-constraints.md`
lists it by name among the things an OS build reaches for reflexively and pm
refuses (C2). This names it, so the image layer can call it:

```
$ pm explain build.yaml
grants:     Toolchain, Coreutils, Archive
plugins:    losos-image 0.1.0, losos-mkosi 0.1.0, losos-systemd 0.1.0

COMMAND                                              FINGERPRINT
mkosi --directory … --output-dir /build/media build  losos-mkosi:mkosi
```

Again no `Network`, and the ceiling says so. mkosi normally installs a
distribution's packages, which is a network build; this one does not, because
the image layer hands it a tree pm has already built (`BaseTrees=`,
`Distribution=custom`) and there is nothing left to download. That distinction
matters more here than elsewhere: network in a pm build file is per *file*, not
per step (C8), so one careless grant would put the whole image layer's jail on
the host network.

It is also the one plugin here that publishes **symbols** — the handful of
paths mkosi and the Boot Loader Specification fix, which a build file would
otherwise spell out:

```
$ pm plugins
losos-mkosi 0.1.0 (signed)
symbols:    5
  %{losos-mkosi:config} = mkosi.conf
  %{losos-mkosi:esp} = /efi
  %{losos-mkosi:loader-dir} = /efi/EFI/systemd
  %{losos-mkosi:uki-dir} = /efi/EFI/Linux
  %{losos-mkosi:xbootldr} = /boot
```

The test for whether something belongs in that list is whether the ecosystem
fixed it or this distribution chose it. `/efi/EFI/Linux` is where a UKI goes,
by specification, on every machine; where *this* build writes its output is a
decision the build file is free to make, so it stays in the build file.

`mkosi-sandbox` is deliberately not classified. mkosi execs it for itself from
inside a build, where pm's first-word resolution is never consulted, and naming
it here would suggest a recipe could call it directly — which would mean
building an image outside mkosi's own bookkeeping.

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
just plugins                     # all three, into dist/
just plugins losos-mkosi         # one
```

The top-level `just sign` recipe — via `tools/Justfile`'s `sign-all` recipe —
installs whatever is in `dist/` into the repo-local trust store and signs it
there.
Without that pm loads no plugins at all, and a recipe calling `mkosi` is
refused with "no built-in fingerprint matches", which reads as a problem with
the recipe rather than with a component that was never installed.

`tools/gates/plugins.py` checks that `wit/plugin.wit` here still matches pm's.
That is not something pm can catch: from pm's side a plugin built against an
older contract is simply a plugin, right up to the point where a record gains
a field and every component stops loading.
