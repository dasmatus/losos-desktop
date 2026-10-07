//! Danube, the LosOS web browser (docs/danube.md): WPE WebKit drawn inside
//! an mcsapi window, sandboxed with Hakoniwa.
//!
//!     danube [URL...]                 open a window, or tabs in the open one
//!     danube --captive-portal [URL]   the network sign-in window
//!     danube captive-watch            open that window when a portal appears
//!
//! Started outside its sandbox, Danube builds it (danube-sandbox's
//! `browser` plan) and runs itself again inside; `--no-sandbox` skips that
//! for debugging. Inside, WebKit runs on this thread under GLib's main
//! loop (engine.rs) and the window on a second thread under winit's
//! (ui.rs).

mod captive;
mod config;
mod derisk;
mod engine;
mod ffi;
mod input;
mod permissions;
mod shared;
mod ui;

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use shared::{Command, Shared};

/// Where losos-adblock compiles its lists (nixos/modules/adblock.nix).
const ADBLOCK_STATE: &str = "/var/lib/losos-adblock";

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
enum Error {
    #[error("HOME is not set")]
    #[diagnostic(code(danube::home))]
    NoHome,
    #[error(transparent)]
    #[diagnostic(transparent)]
    Sandbox(#[from] danube_sandbox::Error),
    #[error("no captive portal check is configured")]
    #[diagnostic(code(danube::captive), help("losos.captivePortal.checkUrl writes /etc/danube/captive-portal.conf"))]
    NoCheck,
    #[error("cannot open the window")]
    #[diagnostic(code(danube::window))]
    Window(String),
}

struct Args {
    captive: bool,
    watch: bool,
    sandbox: bool,
    uris: Vec<String>,
}

fn args() -> Args {
    let mut args = Args { captive: false, watch: false, sandbox: true, uris: Vec::new() };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--captive-portal" => args.captive = true,
            "--no-sandbox" => args.sandbox = false,
            "captive-watch" if args.uris.is_empty() => args.watch = true,
            // A file given by path opens as a file:// URL.
            _ if Path::new(&arg).exists() && !arg.contains("://") => {
                let path = std::fs::canonicalize(&arg).unwrap_or_else(|_| PathBuf::from(&arg));
                args.uris.push(format!("file://{}", path.display()));
            }
            _ => args.uris.push(arg),
        }
    }
    args
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match run(args()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{:?}", miette::Report::new(error));
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<ExitCode, Error> {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("danube"));
    if args.watch {
        let check = captive::Check::load().ok_or(Error::NoCheck)?;
        captive::watch(check, &exe);
    }
    let home = PathBuf::from(std::env::var_os("HOME").ok_or(Error::NoHome)?);
    let instance = home.join(".cache/danube/instance.sock");

    // One browser per profile: WebKit's storage is not shared between
    // processes, so a second `danube URL` hands its URLs to the first.
    if !args.captive && hand_over(&instance, &args.uris) {
        return Ok(ExitCode::SUCCESS);
    }

    if args.sandbox && std::env::var_os("DANUBE_SANDBOXED").is_none() {
        return sandboxed(&exe, &home);
    }

    let settings = config::Settings::load(&home);
    // SAFETY: no other thread has started yet.
    unsafe {
        // JavaScriptCore reads its options from JSC_ variables in every
        // process, the web processes WebKit starts included.
        if !settings.jit {
            std::env::set_var("JSC_useJIT", "false");
        }
        // Web processes go into danube-sandbox's Hakoniwa containers in
        // place of bwrap's (nixos/pkgs/patches/wpewebkit).
        if std::env::var_os("WEBKIT_SANDBOX_LAUNCHER").is_none() {
            if let Some(launcher) = exe.parent().map(|d| d.join("danube-sandbox")).filter(|p| p.exists()) {
                std::env::set_var("WEBKIT_SANDBOX_LAUNCHER", launcher);
            }
        }
    }

    let (commands_tx, commands) = mpsc::channel();
    let shared = Shared::new(commands_tx, engine::wake);

    let options = if args.captive {
        let check = captive::Check::load();
        let open = if args.uris.is_empty() {
            check.as_ref().map(|c| vec![c.url.clone()]).unwrap_or_default()
        } else {
            args.uris.clone()
        };
        if let Some(check) = check {
            // The window closes by itself once the network lets traffic
            // through.
            let shared = shared.clone();
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(3));
                if captive::check(&check) == captive::Verdict::Online {
                    info!("signed in to the network");
                    shared.send(Command::Quit);
                    return;
                }
            });
        }
        engine::Options {
            profile: None,
            adblock: None,
            filter_cache: home.join(".cache/danube/content-filters"),
            permissions: None,
            downloads: home.join("Downloads"),
            settings: settings.clone(),
            open,
        }
    } else {
        listen(&instance, shared.clone());
        derisk::spawn(shared.clone());
        engine::Options {
            profile: Some((home.join(".local/share/danube"), home.join(".cache/danube"))),
            adblock: Some(PathBuf::from(ADBLOCK_STATE).join("webkit")),
            filter_cache: home.join(".cache/danube/content-filters"),
            permissions: Some(home.join(".config/danube/permissions")),
            downloads: home.join("Downloads"),
            settings: settings.clone(),
            open: args.uris.clone(),
        }
    };

    let window = {
        let shared = shared.clone();
        let captive = args.captive;
        thread::Builder::new()
            .name("window".into())
            .spawn(move || {
                let result = window(shared.clone(), settings, captive);
                // The window is gone: so is WebKit.
                shared.send(Command::Quit);
                result
            })
            .expect("a thread can start")
    };

    engine::run(shared, commands, options);
    // WebKit stopped (last tab closed, or the window closed): the window
    // sees `quit` and closes; wait for it, then leave.
    let result = window.join().unwrap_or(Ok(()));
    let _ = std::fs::remove_file(&instance);
    result.map(|()| ExitCode::SUCCESS)
}

/// Runs the window until it closes.
fn window(shared: std::sync::Arc<Shared>, settings: config::Settings, captive: bool) -> Result<(), Error> {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_app_id(derisk::APP_ID)
        .with_title(if captive { "Sign in to network" } else { "Danube" })
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([320.0, 400.0]);
    let options = eframe::NativeOptions {
        viewport,
        // winit runs on the process's main thread unless told otherwise;
        // that thread is WebKit's.
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            use winit::platform::x11::EventLoopBuilderExtX11;
            EventLoopBuilderExtWayland::with_any_thread(builder, true);
            EventLoopBuilderExtX11::with_any_thread(builder, true);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "Danube",
        options,
        Box::new(move |cc| {
            let _ = shared.ctx.set(cc.egui_ctx.clone());
            Ok(Box::new(ui::Window::new(shared, settings, captive)))
        }),
    )
    .map_err(|e| Error::Window(e.to_string()))
}

/// Hands `uris` to a Danube already running on this profile; `false` when
/// there is none.
fn hand_over(socket: &Path, uris: &[String]) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return false;
    };
    let line = serde_json::json!(uris).to_string() + "\n";
    stream.write_all(line.as_bytes()).is_ok()
}

/// Takes URLs from later `danube` runs and opens each in a new tab.
fn listen(socket: &Path, shared: std::sync::Arc<Shared>) {
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A socket left by a browser that crashed; hand_over found nobody on it.
    let _ = std::fs::remove_file(socket);
    let listener = match UnixListener::bind(socket) {
        Ok(listener) => listener,
        Err(error) => {
            warn!(%error, "cannot take URLs from other danube runs");
            return;
        }
    };
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            let uris: Vec<String> = serde_json::from_str(&line).unwrap_or_default();
            if uris.is_empty() {
                shared.send(Command::NewTab { uri: None, activate: true });
            }
            for (n, uri) in uris.into_iter().enumerate() {
                shared.send(Command::NewTab { uri: Some(uri), activate: n == 0 });
            }
        }
    });
}

/// Runs Danube again inside its sandbox, with a filtered session bus.
fn sandboxed(exe: &Path, home: &Path) -> Result<ExitCode, Error> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid cannot fail.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    });
    let (proxy, bus) = bus_proxy(&runtime_dir);
    let mut program = vec![exe.to_string_lossy().into_owned()];
    program.extend(std::env::args().skip(1));
    let session = danube_sandbox::browser::Session {
        home: home.to_path_buf(),
        runtime_dir,
        wayland_display: std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into()),
        bus_proxy: bus.clone(),
        adblock_state: ADBLOCK_STATE.into(),
        program,
    };
    let result = danube_sandbox::run(danube_sandbox::browser::plan(&session));
    if let Some(mut proxy) = proxy {
        let _ = proxy.kill();
        let _ = proxy.wait();
    }
    if let Some(bus) = bus {
        let _ = std::fs::remove_file(bus);
    }
    Ok(result?)
}

/// Starts xdg-dbus-proxy, so the browser sees only the parts of the
/// session bus it uses: the desktop portals (file chooser, opening links
/// in other apps), notifications and the accessibility bus. `None` when
/// there is no session bus or no proxy; the browser then gets no bus.
fn bus_proxy(runtime_dir: &Path) -> (Option<Child>, Option<PathBuf>) {
    let Some(address) = std::env::var_os("DBUS_SESSION_BUS_ADDRESS") else {
        return (None, None);
    };
    let socket = runtime_dir.join(format!("danube-bus-{}", std::process::id()));
    let child = std::process::Command::new("xdg-dbus-proxy")
        .arg(address)
        .arg(&socket)
        .args([
            "--filter",
            "--talk=org.freedesktop.portal.*",
            "--talk=org.freedesktop.Notifications",
            "--talk=org.a11y.Bus",
        ])
        .spawn();
    let Ok(child) = child else {
        warn!("xdg-dbus-proxy is not installed; the browser gets no session bus");
        return (None, None);
    };
    for _ in 0..50 {
        if socket.exists() {
            return (Some(child), Some(socket));
        }
        thread::sleep(Duration::from_millis(50));
    }
    warn!("xdg-dbus-proxy did not start; the browser gets no session bus");
    (Some(child), None)
}
