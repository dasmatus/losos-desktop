# The security report

GNOME Settings has a shield in Privacy & Security. Clicking it opens a dialog
that says "Protected" or "Security Checks Failed" and offers to copy a
technical report. Everything it knows comes from fwupd's
`GetHostSecurityAttrs`, and fwupd's Host Security ID is an index of *firmware*
protections: Secure Boot, the SPI descriptor, Intel BootGuard, TPM
reconstruction.

That is a real question. It is not the question a desktop user is asking, which
is nearer "is this machine set up the way a careful person would set it up".
Whether the disk is encrypted, whether the kernel is locked down, whether
modules are signed, whether the IOMMU is on, whether the usual hardening
sysctls are set — none of it appears in that dialog, because fwupd reports on
the OS only where a plugin happens to exist. lynis answers those questions, for
servers, as a shell script producing a report nobody reads twice.

So this OS ships both halves, in one list.

## Two providers, one shape

`losos-security` (`recipes/10-core/losos-security/`) runs the OS-level checks
and serves them on the system bus as `io.losos.Security1`, answering
`GetHostSecurityAttrs` — deliberately the same method name, and the same
`a{sv}` dictionary keys, that fwupd uses. That choice is the whole design: the
gnome-control-center patch is "ask a second bus name and file the answers
separately", not a second parser and a second widget. The key set is pinned in
`src/attr.rs` against gnome-control-center's
`panels/privacy/firmware-security/cc-firmware-security-utils.c`, which is the
consumer.

The attributes go into their own table, not into the HSI tables. HSI levels are
defined by fwupd, for firmware; filing "disk encryption" under HSI-1 would
claim an equivalence that does not exist.

## The checks

Each reports a `FwupdSecurityAttrResult`, a level, and — this is the part that
is not fwupd's — the path it read and what it found.

| Check | Reads |
|---|---|
| UEFI Secure Boot | `/sys/firmware/efi/efivars/SecureBoot-8be4df61-…`, fifth byte |
| TPM 2.0 present | `/sys/class/tpm/tpm0/tpm_version_major` |
| Measured boot | `/sys/kernel/security/tpm0/binary_bios_measurements` |
| Disk encryption | `/sys/block/*/dm/uuid`, prefix `CRYPT-LUKS` |
| Verified system image | `/sys/block/*/dm/uuid`, prefix `CRYPT-VERITY` |
| Kernel lockdown | `/sys/kernel/security/lockdown`, the bracketed mode |
| Module signature enforcement | `/sys/module/module/parameters/sig_enforce` |
| IOMMU | `/sys/class/iommu` |
| Kernel address exposure | `kernel.kptr_restrict` ≥ 1 |
| Kernel log access | `kernel.dmesg_restrict` ≥ 1 |
| Unprivileged BPF | `kernel.unprivileged_bpf_disabled` ≥ 1 |
| BPF JIT hardening | `net.core.bpf_jit_harden` ≥ 1 |

Two properties are load-bearing and easy to lose:

**Every check names where it looked.** A verdict without its evidence cannot be
argued with, and a security report nobody can argue with is one nobody can
correct. The evidence rides in the `Description` field, so the panel shows it
when the row is expanded.

**Every check is rooted.** Nothing opens an absolute path; they all go through
`Context::path`, which prefixes a configurable root. That is what makes the
suite testable — `tests/checks.rs` builds a fixture directory of sysfs files
and asserts both directions of every check, with no TPM and no root — and it
costs one function call.

The overall level is the **weakest link**: the highest level at which every
check passed, not a score and not a percentage. A machine with nine passes and
one level-1 failure has a level-0 problem, and averaging that away is how a
report becomes decoration. A level with no checks in it does not count as
passed — `all()` over an empty iterator is true, which would otherwise report
the most flattering possible answer arrived at by testing nothing.

## The panel change

`recipes/30-gnome/gnome-control-center/files/patches/0002-firmware-security-report-os-level-checks.patch`
does three things:

- Adds a third D-Bus proxy, to `io.losos.Security1`, alongside the two fwupd
  ones, and files its reply into a separate table through the existing parser.
- Makes the page's availability a count rather than a single callback's
  decision. The two providers reply in either order, so neither can conclude on
  its own that there is nothing to show; the "unavailable" page appears only
  when both have failed. The fwupd failure path now counts itself, which
  upstream never needed to do — without it a machine with the OS provider and
  no fwupd waits forever for a verdict. A provider that answered also overrides
  the page hiding itself on a virtual machine, where a firmware index means
  nothing and the OS checks still do.
- Lists the checks in the dialog. Upstream keeps them inside the copied
  technical report only, which is defensible while every check is about
  firmware the reader cannot change from here. It stops being defensible once
  the list includes a lockdown mode and four sysctls, which can be fixed by
  someone who is told which one failed.

## Running it

```sh
losos-security                 # human-readable; exit status is the verdict
losos-security --json
losos-security --root ./fixture   # every check against a fixture tree
```

The service itself is bus-activated, not enabled: the unit has no `[Install]`
section, and `io.losos.Security1.service` in `dbus-1/system-services` names
`SystemdService=` so dbus-daemon hands the start to systemd and the sandbox in
`losos-security.service` applies. It runs as root — two of the paths above are
not world-readable — and then gives back everything a root process could
otherwise do: no capabilities, no writable filesystem, no network namespace,
`AF_UNIX` only, `@system-service` and nothing else. A security reporter that is
itself an attack surface has subtracted from the security of the machine it
reports on.
