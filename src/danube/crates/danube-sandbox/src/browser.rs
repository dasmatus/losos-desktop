//! The sandbox the whole browser runs in.
//!
//! What Danube can see: the Nix store and the system's configuration,
//! read-only; the GPU; its own profile, cache and settings directories, and
//! Downloads, writable; the fonts, themes and icons a user installs,
//! read-only; the Wayland, PipeWire and PulseAudio sockets and derisk's;
//! the compiled filter lists. The session bus only through xdg-dbus-proxy,
//! filtered to the portals, notifications and accessibility. Nothing else
//! in the home directory exists in there, so a compromised browser cannot
//! read SSH keys or other programs' data.
//!
//! The same paths make a Landlock ruleset, so even a way out of the mount
//! namespace finds the rest of the filesystem closed, and a seccomp filter
//! refuses the system calls a browser never needs. Network access is
//! shared: it is a browser.

use std::path::{Path, PathBuf};

use crate::{Access, Op, Plan, Unshare};

/// System calls a browser has no use for, refused with EPERM. Flatpak's
/// list (flatpak-run.c's syscall_blocklist), less what WebKit's own sandbox
/// needs to build the per-page one inside this one: clone and unshare with
/// new namespaces, mount, pivot_root.
pub fn syscall_denylist() -> Vec<&'static str> {
    vec![
        "syslog",
        "uselib",
        "acct",
        "quotactl",
        "add_key",
        "keyctl",
        "request_key",
        "move_pages",
        "mbind",
        "get_mempolicy",
        "set_mempolicy",
        "migrate_pages",
        "kexec_load",
        "kexec_file_load",
        "init_module",
        "finit_module",
        "delete_module",
        "perf_event_open",
        "bpf",
        "userfaultfd",
        "open_by_handle_at",
        "name_to_handle_at",
        "lookup_dcookie",
        "fanotify_init",
        "iopl",
        "ioperm",
        "swapon",
        "swapoff",
        "reboot",
        "settimeofday",
        "clock_settime",
        "clock_adjtime",
        "adjtimex",
        "nfsservctl",
        "vhangup",
        "process_vm_writev",
        "kcmp",
    ]
}

/// What the browser's sandbox needs from the session it starts in.
#[derive(Clone, Debug)]
pub struct Session {
    pub home: PathBuf,
    pub runtime_dir: PathBuf,
    /// The Wayland socket's name in `runtime_dir`.
    pub wayland_display: String,
    /// The filtered session bus socket xdg-dbus-proxy listens on, bound
    /// into the sandbox as the session bus.
    pub bus_proxy: Option<PathBuf>,
    /// Where losos-adblock compiles its lists.
    pub adblock_state: PathBuf,
    /// The program (and its arguments) to run inside.
    pub program: Vec<String>,
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The browser's sandbox for `session`. Danube's own directories are
/// created first, so they can be bound.
pub fn plan(session: &Session) -> Plan {
    let home = &session.home;
    let run = &session.runtime_dir;
    let mut plan = Plan {
        // No network namespace: the network process needs the host's.
        unshare: vec![Unshare::Pid, Unshare::Ipc, Unshare::Uts],
        deny_syscalls: syscall_denylist(),
        program: session.program.clone(),
        chdir: Some(s(home)),
        ..Plan::default()
    };
    fn bind(plan: &mut Plan, src: &Path, access: Access, optional: bool) {
        plan.ops.push(Op::Bind {
            src: s(src),
            dest: s(src),
            ro: access == Access::Read,
            dev: access == Access::Device,
            optional,
        });
        plan.landlock.push((s(src), access));
    }

    // The system, read-only: every program and library, the configuration
    // they read (fonts, certificates, resolv.conf, time zone, passwd), the
    // GPU driver NixOS links at /run/opengl-driver, and sysfs, which Mesa
    // and libdrm read to find the GPU.
    for path in [
        "/nix/store",
        "/run/current-system",
        "/run/opengl-driver",
        "/run/opengl-driver-32",
        "/etc",
        "/sys",
        // glibc's NSS asks resolved over its Varlink socket; every name
        // still goes through the adblock forwarder.
        "/run/systemd/resolve",
        // Danube reads the compiled filter lists.
        &s(&session.adblock_state),
    ] {
        bind(&mut plan, Path::new(path), Access::Read, true);
    }
    bind(&mut plan, Path::new("/dev/dri"), Access::Device, true);

    plan.ops.push(Op::Dev("/dev".into()));
    plan.ops.push(Op::Proc("/proc".into()));
    plan.ops.push(Op::Tmpfs("/tmp".into()));
    plan.landlock.push(("/tmp".into(), Access::Write));
    plan.landlock.push(("/proc".into(), Access::Read));
    plan.landlock.push(("/dev".into(), Access::Device));

    // An empty runtime directory with only the sockets it needs.
    plan.ops.push(Op::Tmpfs(s(run)));
    plan.landlock.push((s(run), Access::Write));
    for name in [
        session.wayland_display.as_str(),
        "pipewire-0",
        "pulse/native",
        // derisk's agent socket and theme: menus in the palette, the theme.
        "derisk",
        // WebKit puts its own sockets for the web processes' D-Bus proxy
        // here; the runtime tmpfs already allows that.
    ] {
        let path = run.join(name);
        plan.ops.push(Op::Bind {
            src: s(&path),
            dest: s(&path),
            ro: false,
            dev: false,
            optional: true,
        });
    }
    if let Some(proxy) = &session.bus_proxy {
        plan.ops.push(Op::Bind {
            src: s(proxy),
            dest: s(&run.join("bus")),
            ro: false,
            dev: false,
            optional: false,
        });
        plan.env.push((
            "DBUS_SESSION_BUS_ADDRESS".into(),
            Some(format!("unix:path={}", s(&run.join("bus")))),
        ));
    } else {
        plan.env.push(("DBUS_SESSION_BUS_ADDRESS".into(), None));
    }

    // An empty home with Danube's directories and the user's own fonts,
    // themes and icons in it.
    plan.ops.push(Op::Tmpfs(s(home)));
    for dir in [
        ".local/share/danube",
        ".cache/danube",
        ".config/danube",
        "Downloads",
    ] {
        let path = home.join(dir);
        let _ = std::fs::create_dir_all(&path);
        bind(&mut plan, &path, Access::Write, false);
    }
    for dir in [
        ".local/share/fonts",
        ".fonts",
        ".config/fontconfig",
        ".local/share/derisk",
        ".local/share/icons",
        ".icons",
        // Theme and settings files derisk writes for other toolkits.
        ".config/derisk",
    ] {
        bind(&mut plan, &home.join(dir), Access::Read, true);
    }
    plan.landlock.push((s(home), Access::Read));

    // Marks the inside, so the browser does not try to sandbox itself again.
    plan.env.push(("DANUBE_SANDBOXED".into(), Some("1".into())));
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_holds_only_danubes_directories() {
        let root = std::env::temp_dir().join(format!("danube-plan-{}", std::process::id()));
        let session = Session {
            home: root.join("home"),
            runtime_dir: root.join("run"),
            wayland_display: "wayland-0".into(),
            bus_proxy: None,
            adblock_state: "/var/lib/losos-adblock".into(),
            program: vec!["/bin/danube".into()],
        };
        let plan = plan(&session);
        let writable: Vec<&str> = plan
            .landlock
            .iter()
            .filter(|(_, a)| *a == Access::Write)
            .map(|(p, _)| p.as_str())
            .collect();
        for path in &writable {
            let path = Path::new(path);
            assert!(
                !path.starts_with(&session.home)
                    || [
                        ".local/share/danube",
                        ".cache/danube",
                        ".config/danube",
                        "Downloads"
                    ]
                    .iter()
                    .any(|d| path == session.home.join(d)),
                "{path:?} is writable"
            );
        }
        assert!(plan.ops.contains(&Op::Tmpfs(s(&session.home))));
        let _ = std::fs::remove_dir_all(&root);
    }
}
