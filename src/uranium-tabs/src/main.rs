//! uranium-tabs: Chromium starts it as a native messaging host for
//! Uranium's tabs extension, and it keeps derisk's menus for Uranium's
//! windows in step with their tabs (lib.rs says how the halves meet).
//!
//! Three threads feed one loop: the extension's messages on stdin, derisk's
//! lines on its agent socket, and a one-second tick. The tick is there
//! because a window's title, which pairs it with derisk's, changes after
//! the extension reports the tab that changed it.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use tracing::warn;
use tracing_subscriber::EnvFilter;
use uranium_tabs::{command, menus, pair, read_message, write_message, ShellWindow, Snapshot};

enum Event {
    Snapshot(Snapshot),
    /// stdin closed: the extension, or the browser, is gone.
    Quit,
    Line(u64, String),
    /// The connection with this number closed.
    Lost(u64),
    Tick,
}

/// What a response line answers. derisk answers requests in order, so a
/// queue of what was asked tells which is which.
enum Asked {
    State,
    /// Menus for this window.
    Register(u64),
}

struct Derisk {
    stream: UnixStream,
    number: u64,
    asked: VecDeque<Asked>,
    /// What each window was last given, so a tick that changes nothing
    /// sends nothing.
    registered: HashMap<u64, Value>,
}

fn socket() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(dir).join("derisk").join("agent.sock"))
}

fn connect(number: u64, events: &Sender<Event>) -> Option<Derisk> {
    let stream = UnixStream::connect(socket()?).ok()?;
    let reader = BufReader::new(stream.try_clone().ok()?);
    let events = events.clone();
    thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if events.send(Event::Line(number, line)).is_err() {
                return;
            }
        }
        let _ = events.send(Event::Lost(number));
    });
    Some(Derisk {
        stream,
        number,
        asked: VecDeque::new(),
        registered: HashMap::new(),
    })
}

impl Derisk {
    fn send(&mut self, request: Value, asked: Asked) -> io::Result<()> {
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;
        self.asked.push_back(asked);
        Ok(())
    }

    /// Pairs the windows and registers the menus that changed.
    fn update(&mut self, snapshot: &Snapshot, state: &Value) -> io::Result<()> {
        let shell: Vec<ShellWindow> =
            serde_json::from_value(state["windows"].clone()).unwrap_or_default();
        for (browser, window) in pair(snapshot, &shell) {
            let Some(tabs) = snapshot.windows.iter().find(|w| w.id == browser) else {
                continue;
            };
            let menus = menus(tabs);
            if self.registered.get(&window) == Some(&menus) {
                continue;
            }
            self.send(
                json!({"method": "register_menu", "window": window, "menus": menus}),
                Asked::Register(window),
            )?;
            self.registered.insert(window, menus);
        }
        Ok(())
    }
}

fn main() {
    // stderr, never stdout: stdout is Chromium's native messaging protocol,
    // and Chromium keeps a host's stderr in its own log.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let (events, inbox) = mpsc::channel();

    let from_browser = events.clone();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        // Ends when Chromium closes the pipe, or on a broken message.
        while let Ok(Some(body)) = read_message(&mut stdin) {
            match serde_json::from_slice(&body) {
                Ok(snapshot) => {
                    if from_browser.send(Event::Snapshot(snapshot)).is_err() {
                        return;
                    }
                }
                Err(error) => warn!(%error, "not a snapshot"),
            }
        }
        let _ = from_browser.send(Event::Quit);
    });

    let ticks = events.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        if ticks.send(Event::Tick).is_err() {
            return;
        }
    });

    let mut snapshot = Snapshot::default();
    let mut derisk: Option<Derisk> = None;
    let mut connections = 0;
    let mut stdout = io::stdout().lock();

    for event in inbox {
        match event {
            Event::Quit => break,
            Event::Snapshot(s) => {
                snapshot = s;
                if let Some(d) = &mut derisk {
                    if !d.asked.iter().any(|a| matches!(a, Asked::State))
                        && d.send(json!({"method": "state"}), Asked::State).is_err()
                    {
                        derisk = None;
                    }
                }
            }
            Event::Tick => {
                // Outside a derisk session, or before it is up, there is no
                // socket; keep trying, once a second.
                if derisk.is_none() {
                    connections += 1;
                    derisk = connect(connections, &events);
                }
                if let Some(d) = &mut derisk {
                    if !d.asked.iter().any(|a| matches!(a, Asked::State))
                        && d.send(json!({"method": "state"}), Asked::State).is_err()
                    {
                        derisk = None;
                    }
                }
            }
            Event::Lost(number) => {
                if derisk.as_ref().is_some_and(|d| d.number == number) {
                    derisk = None;
                }
            }
            Event::Line(number, line) => {
                let Some(d) = derisk.as_mut().filter(|d| d.number == number) else {
                    continue;
                };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message["event"] == "menu" {
                    let Some(command) = message["item"].as_str().and_then(command) else {
                        continue;
                    };
                    if write_message(&mut stdout, &command).is_err() {
                        break;
                    }
                    continue;
                }
                match d.asked.pop_front() {
                    Some(Asked::State) if message["ok"] == true => {
                        if d.update(&snapshot, &message["result"]).is_err() {
                            derisk = None;
                        }
                    }
                    Some(Asked::Register(window)) if message["ok"] != true => {
                        // Forgotten, so the next tick tries again: the window
                        // may not have been mapped yet.
                        d.registered.remove(&window);
                        warn!(error = %message["error"], "derisk refused the menus");
                    }
                    _ => {}
                }
            }
        }
    }
}
