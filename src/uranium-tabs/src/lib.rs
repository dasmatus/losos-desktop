//! Uranium's tabs in derisk's command palette.
//!
//! Chromium keeps its tabs to itself: on Wayland it exports no global menu,
//! and nothing outside it can list or switch them. So two halves meet here.
//! An extension Uranium loads into every profile (nixos/pkgs/uranium/tabs)
//! sends its tabs, through Chromium's native messaging, to this program,
//! which Chromium starts for it. This program registers them with derisk as
//! the window's global menus, over derisk's agent socket, and derisk lists
//! a window's menus in its command palette and top bar. A tab picked there
//! comes back as an event on the same connection, and goes to the extension
//! as a command, which switches to the tab, closes it or opens a new one.
//!
//! This half is the matching and the wording, kept apart from the sockets
//! so it can be tested; `main.rs` moves the messages.

use std::collections::HashSet;
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The app ids Uranium's windows carry: the launcher passes
/// `--class=uranium`, and the Flatpak its own app id.
pub const APP_IDS: [&str; 2] = ["uranium", "org.losos.Uranium"];

/// Prefix of the menu item ids this program registers, so an event for any
/// other id is not ours to act on.
const ITEM_PREFIX: &str = "uranium.";

/// What the extension sends: every window's tabs, in order.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Snapshot {
    pub windows: Vec<BrowserWindow>,
}

/// One browser window, by the extension's window id.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BrowserWindow {
    pub id: i64,
    pub tabs: Vec<Tab>,
}

/// One tab. The extension sends the address as the title of a tab that
/// has none yet.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Tab {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub active: bool,
}

impl BrowserWindow {
    fn active_title(&self) -> Option<&str> {
        self.tabs
            .iter()
            .find(|t| t.active)
            .map(|t| t.title.as_str())
    }
}

/// One of derisk's windows, from its `state` request.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ShellWindow {
    pub id: u64,
    pub app_id: String,
    pub title: String,
}

/// What the extension is asked to do, as it reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Activate { tab: i64 },
    Close { tab: i64 },
    New { window: i64 },
}

/// Pairs each browser window with derisk's window for it.
///
/// Neither side knows the other's ids, and Chromium does not tell the
/// extension its Wayland surface. What they share is the title: Chromium
/// names a window after its active tab, followed by " - " and the product
/// name. So a derisk window of Uranium's whose title starts with a browser
/// window's active tab title is that window's. The longest title is
/// matched first, so "Inbox" does not claim the window showing "Inbox (3)",
/// and each derisk window goes to one browser window at most; two windows
/// showing the same title pair up in order, which only swaps two identical
/// menus' worth of tabs if it is wrong.
pub fn pair(snapshot: &Snapshot, shell: &[ShellWindow]) -> Vec<(i64, u64)> {
    let mut windows: Vec<(&BrowserWindow, &str)> = snapshot
        .windows
        .iter()
        .filter_map(|w| w.active_title().map(|t| (w, t)))
        .collect();
    windows.sort_by_key(|(_, title)| std::cmp::Reverse(title.len()));
    let mut taken = HashSet::new();
    let mut pairs = Vec::new();
    for (window, title) in windows {
        let found = shell.iter().find(|s| {
            APP_IDS.contains(&s.app_id.as_str())
                && !taken.contains(&s.id)
                && s.title.starts_with(title)
        });
        if let Some(s) = found {
            taken.insert(s.id);
            pairs.push((window.id, s.id));
        }
    }
    pairs
}

/// The global menus for one browser window, in derisk's `register_menu`
/// shape. The palette shows each item under its last name, so a tab is
/// listed by its title to switch to it, and as "Close" and its title to
/// close it; the top bar shows the same two menus.
pub fn menus(window: &BrowserWindow) -> Value {
    let mut switch = vec![
        json!({
            "kind": "item",
            "id": format!("{ITEM_PREFIX}new.{}", window.id),
            "label": "New Tab",
            "shortcut": "Ctrl+T",
        }),
        json!({"kind": "separator"}),
    ];
    let mut close = Vec::new();
    for tab in &window.tabs {
        switch.push(json!({
            "kind": "item",
            "id": format!("{ITEM_PREFIX}tab.{}", tab.id),
            "label": tab.title,
        }));
        close.push(json!({
            "kind": "item",
            "id": format!("{ITEM_PREFIX}close.{}", tab.id),
            "label": format!("Close {}", tab.title),
        }));
    }
    json!([
        {"title": "Tabs", "entries": switch},
        {"title": "Close Tab", "entries": close},
    ])
}

/// The command for a picked menu item, or nothing for an id this program
/// did not register.
pub fn command(item: &str) -> Option<Command> {
    let (kind, id) = item.strip_prefix(ITEM_PREFIX)?.split_once('.')?;
    let id = id.parse().ok()?;
    match kind {
        "tab" => Some(Command::Activate { tab: id }),
        "close" => Some(Command::Close { tab: id }),
        "new" => Some(Command::New { window: id }),
        _ => None,
    }
}

/// The most Chromium sends in one native message. Its own limit is 4 GiB;
/// a snapshot of a thousand tabs is a few hundred kilobytes, so anything
/// past this is not a snapshot.
const MAX_MESSAGE: u32 = 16 << 20;

/// Reads one native message: a length in the host's byte order, then that
/// much JSON. None when Chromium closes the pipe, which is how it says the
/// extension let go.
pub fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_ne_bytes(len);
    if len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a {len}-byte native message"),
        ));
    }
    let mut body = vec![0; len as usize];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Writes one native message.
pub fn write_message(output: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message too long"))?;
    output.write_all(&len.to_ne_bytes())?;
    output.write_all(&body)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: i64, title: &str, active: bool) -> Tab {
        Tab {
            id,
            title: title.into(),
            active,
        }
    }

    fn shell(id: u64, app_id: &str, title: &str) -> ShellWindow {
        ShellWindow {
            id,
            app_id: app_id.into(),
            title: title.into(),
        }
    }

    #[test]
    fn windows_pair_by_their_active_tab_title() {
        let snapshot = Snapshot {
            windows: vec![
                BrowserWindow {
                    id: 1,
                    tabs: vec![tab(10, "Inbox", true), tab(11, "News", false)],
                },
                BrowserWindow {
                    id: 2,
                    tabs: vec![tab(20, "Inbox (3)", true)],
                },
            ],
        };
        let shell = [
            shell(7, "uranium", "Inbox (3) - Chromium"),
            shell(8, "uranium", "Inbox - Chromium"),
            shell(9, "kitty", "Inbox - vim"),
        ];
        let mut pairs = pair(&snapshot, &shell);
        pairs.sort();
        assert_eq!(pairs, [(1, 8), (2, 7)]);
    }

    #[test]
    fn other_apps_windows_are_never_claimed() {
        let snapshot = Snapshot {
            windows: vec![BrowserWindow {
                id: 1,
                tabs: vec![tab(10, "Notes", true)],
            }],
        };
        assert!(pair(
            &snapshot,
            &[shell(3, "org.derisk.editor", "Notes - Editor")]
        )
        .is_empty());
    }

    #[test]
    fn menu_items_come_back_as_commands() {
        let window = BrowserWindow {
            id: 4,
            tabs: vec![tab(42, "Weather", true)],
        };
        let menus = menus(&window);
        let ids: Vec<&str> = menus
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["entries"].as_array().unwrap())
            .filter_map(|e| e["id"].as_str())
            .collect();
        assert_eq!(ids, ["uranium.new.4", "uranium.tab.42", "uranium.close.42"]);
        assert_eq!(command(ids[0]), Some(Command::New { window: 4 }));
        assert_eq!(command(ids[1]), Some(Command::Activate { tab: 42 }));
        assert_eq!(command(ids[2]), Some(Command::Close { tab: 42 }));
        assert_eq!(command("derisk.window.close"), None);
        assert_eq!(command("uranium.tab.x"), None);
    }

    #[test]
    fn commands_read_as_the_extension_expects() {
        assert_eq!(
            serde_json::to_value(Command::Activate { tab: 3 }).unwrap(),
            json!({"command": "activate", "tab": 3})
        );
    }

    #[test]
    fn native_messages_round_trip() {
        let mut wire = Vec::new();
        write_message(&mut wire, &json!({"a": 1})).unwrap();
        let mut input = wire.as_slice();
        let body = read_message(&mut input).unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"a": 1})
        );
        assert!(read_message(&mut input).unwrap().is_none());
    }
}
