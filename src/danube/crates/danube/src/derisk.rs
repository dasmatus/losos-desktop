//! Danube's tabs and navigation as derisk global menus, the way every
//! derisk app puts its menus in the top bar and the command palette: one
//! line of JSON per request on derisk's agent socket
//! (`$XDG_RUNTIME_DIR/derisk/agent.sock`), `register_menu` for the window,
//! and `{"event":"menu","item":...}` lines back when one is picked.
//!
//! derisk names windows by its own ids, so Danube finds its window in
//! derisk's `state` by app id and title. The window sends an [`Update`]
//! whenever its title or tabs change, and this thread runs until the
//! window drops its end of that channel. Outside a derisk session there is
//! no socket, and the thread just keeps trying once a second.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError};
use serde_json::{json, Value};
use tracing::warn;

use crate::shared::{Command, Commands, Tab, TabId};

pub const APP_ID: &str = "org.losos.Danube";
const PREFIX: &str = "danube.";

/// What the menus are built from: the window's title, which pairs it with
/// derisk's window, and its tabs.
#[derive(Default)]
pub struct Update {
    pub title: String,
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
}

/// The menus for a window showing `tabs`, `active` the one in front.
pub fn menus(tabs: &[Tab], active: Option<TabId>) -> Value {
    let current = active.and_then(|id| tabs.iter().find(|t| t.id == id));
    let mut switch = vec![
        json!({"kind": "item", "id": format!("{PREFIX}new"), "label": "New Tab", "shortcut": "Ctrl+T"}),
        json!({"kind": "separator"}),
    ];
    let mut close = Vec::new();
    for tab in tabs {
        let title = if tab.title.is_empty() {
            tab.uri.clone()
        } else {
            tab.title.clone()
        };
        switch
            .push(json!({"kind": "item", "id": format!("{PREFIX}tab.{}", tab.id), "label": title}));
        close.push(json!({"kind": "item", "id": format!("{PREFIX}close.{}", tab.id), "label": format!("Close {title}")}));
    }
    let can = |f: fn(&Tab) -> bool| current.is_some_and(f);
    let navigate = vec![
        json!({"kind": "item", "id": format!("{PREFIX}back"), "label": "Back", "shortcut": "Alt+Left", "enabled": can(|t| t.can_go_back)}),
        json!({"kind": "item", "id": format!("{PREFIX}forward"), "label": "Forward", "shortcut": "Alt+Right", "enabled": can(|t| t.can_go_forward)}),
        json!({"kind": "item", "id": format!("{PREFIX}reload"), "label": "Reload", "shortcut": "Ctrl+R"}),
    ];
    json!([
        {"title": "Tabs", "entries": switch},
        {"title": "Navigate", "entries": navigate},
        {"title": "Close Tab", "entries": close},
    ])
}

/// The command for a picked item; `None` for one Danube did not register.
pub fn command(item: &str, active: Option<TabId>) -> Option<Command> {
    let rest = item.strip_prefix(PREFIX)?;
    let id = |s: &str| s.parse::<TabId>().ok();
    Some(match rest.split_once('.') {
        Some(("tab", n)) => Command::Activate(id(n)?),
        Some(("close", n)) => Command::Close(id(n)?),
        None => match rest {
            "new" => Command::NewTab {
                uri: None,
                activate: true,
            },
            "back" => Command::Back(active?),
            "forward" => Command::Forward(active?),
            "reload" => Command::Reload(active?),
            _ => return None,
        },
        _ => return None,
    })
}

fn socket() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("derisk/agent.sock"))
}

enum Asked {
    State,
    Register,
}

/// Keeps derisk's menus in step with the tabs until the window is gone.
pub fn run(commands: &Commands, updates: Receiver<Update>) {
    let mut current = Update::default();
    loop {
        if let Some(stream) = socket().and_then(|path| UnixStream::connect(path).ok()) {
            if session(commands, &updates, &mut current, stream).is_err() {
                return;
            }
        }
        // A second's wait for derisk, spent taking the window's updates.
        match updates.recv_timeout(Duration::from_secs(1)) {
            Ok(update) => current = update,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The window is gone.
struct Stopped;

/// Takes what the window sent since the last look; `Err` once it is gone.
fn take(updates: &Receiver<Update>, current: &mut Update) -> Result<(), Stopped> {
    loop {
        match updates.try_recv() {
            Ok(update) => *current = update,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => return Err(Stopped),
        }
    }
}

/// One connection, until derisk closes it (`Ok`) or the window is gone.
fn session(
    commands: &Commands,
    updates: &Receiver<Update>,
    current: &mut Update,
    mut stream: UnixStream,
) -> Result<(), Stopped> {
    let Ok(reader) = stream.try_clone() else {
        return Ok(());
    };
    let (lines_tx, lines) = crossbeam_channel::bounded(16);
    // The reader blocks on the socket; the loop below closes the socket
    // to end it, so the scope always finishes.
    thread::scope(|s| {
        s.spawn(move || {
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                if lines_tx.send(line).is_err() {
                    break;
                }
            }
        });
        let result = talk(commands, updates, current, &mut stream, &lines);
        let _ = stream.shutdown(Shutdown::Both);
        result
    })
}

fn talk(
    commands: &Commands,
    updates: &Receiver<Update>,
    current: &mut Update,
    stream: &mut UnixStream,
    lines: &Receiver<String>,
) -> Result<(), Stopped> {
    let mut asked: VecDeque<Asked> = VecDeque::new();
    let mut registered: Option<(u64, Value)> = None;
    let send =
        |stream: &mut UnixStream, request: Value, what: Asked, asked: &mut VecDeque<Asked>| {
            let mut line = request.to_string().into_bytes();
            line.push(b'\n');
            asked.push_back(what);
            stream.write_all(&line).is_ok()
        };
    loop {
        take(updates, current)?;
        if asked.is_empty() && !send(stream, json!({"method": "state"}), Asked::State, &mut asked) {
            return Ok(());
        }
        let line = match lines.recv_timeout(Duration::from_secs(1)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message["event"] == "menu" {
            if let Some(command) = message["item"]
                .as_str()
                .and_then(|item| command(item, current.active))
            {
                commands.send(command);
            }
            continue;
        }
        match asked.pop_front() {
            Some(Asked::State) if message["ok"] == true => {
                let menus = menus(&current.tabs, current.active);
                let window = message["result"]["windows"].as_array().and_then(|windows| {
                    windows
                        .iter()
                        .find(|w| w["app_id"] == APP_ID && w["title"] == current.title.as_str())
                });
                let Some(window) = window.and_then(|w| w["id"].as_u64()) else {
                    continue;
                };
                if registered.as_ref() == Some(&(window, menus.clone())) {
                    // Nothing changed; ask again in a second, or sooner
                    // when the window has news.
                    match updates.recv_timeout(Duration::from_secs(1)) {
                        Ok(update) => *current = update,
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return Err(Stopped),
                    }
                    continue;
                }
                let request = json!({"method": "register_menu", "window": window, "menus": menus});
                if !send(stream, request, Asked::Register, &mut asked) {
                    return Ok(());
                }
                registered = Some((window, menus));
            }
            Some(Asked::Register) if message["ok"] != true => {
                warn!(error = %message["error"], "derisk refused the menus");
                registered = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip() {
        let tabs = vec![Tab {
            id: 4,
            title: "Example".into(),
            can_go_back: true,
            ..Tab::default()
        }];
        let menus = menus(&tabs, Some(4));
        assert_eq!(menus[0]["entries"][2]["id"], "danube.tab.4");
        assert_eq!(menus[1]["entries"][0]["enabled"], true);
        assert_eq!(menus[2]["entries"][0]["label"], "Close Example");
        assert!(matches!(
            command("danube.close.4", None),
            Some(Command::Close(4))
        ));
        assert!(matches!(
            command("danube.back", Some(4)),
            Some(Command::Back(4))
        ));
        assert!(command("danube.back", None).is_none());
        assert!(command("other.tab.4", None).is_none());
    }
}
