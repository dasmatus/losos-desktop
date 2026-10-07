//! What the window (ui.rs) and WebKit (engine.rs) hand each other. They
//! run on two threads: WebKit on the process's main thread under GLib's
//! main loop, the window on its own under winit's. Two bounded channels
//! join them and nothing is locked: the window sends [`Command`]s, WebKit
//! sends [`Event`]s, and the window keeps its own picture of the tabs
//! from them.

use std::sync::Arc;

use crossbeam_channel::{Receiver, SendError, Sender, TrySendError};
use eframe::egui;

use crate::input::Input;

/// A tab, as long as it is open. Never reused.
pub type TabId = u64;

/// What the window shows for one tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub uri: String,
    /// 0 to 1 while loading.
    pub progress: f64,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// An https page whose certificate checked out.
    pub secure: bool,
    /// Its web process died; the page shows nothing until reloaded.
    pub crashed: bool,
}

/// A site asking for something only the person may grant.
#[derive(Clone, Debug)]
pub struct PermissionAsk {
    pub id: u64,
    pub tab: TabId,
    /// The host asking, as shown and as remembered.
    pub host: String,
    pub kind: crate::permissions::Kind,
}

/// What the window asks of WebKit.
#[derive(Debug)]
pub enum Command {
    /// The window's context, sent first, for WebKit to ask it to repaint
    /// with each event.
    Attach(egui::Context),
    NewTab {
        uri: Option<String>,
        activate: bool,
    },
    Activate(TabId),
    Close(TabId),
    Load(TabId, String),
    Back(TabId),
    Forward(TabId),
    Reload(TabId),
    Stop(TabId),
    /// The page area, in logical pixels, and the window's scale.
    Resize {
        width: i32,
        height: i32,
        scale: f64,
    },
    /// The window gained or lost the keyboard.
    Focus(bool),
    Input(TabId, Vec<Input>),
    /// Paste `text` into the page: put it on WebKit's clipboard, then
    /// press Ctrl+V there.
    Paste(TabId, String),
    Permission {
        id: u64,
        allow: bool,
        remember: bool,
    },
    /// The window was laid out for a phone (short side under 600 px) or
    /// not; sets the User-Agent string for the next load.
    Phone(bool),
    Quit,
}

/// What WebKit tells the window.
#[derive(Debug)]
pub enum Event {
    /// A tab joined the end of the strip.
    Opened(Tab),
    /// A tab's title, address, progress or history changed; `crashed` is
    /// not carried, the window keeps it from [`Event::Crashed`].
    Changed(Tab),
    /// A load began, so a page that had crashed is running again.
    LoadStarted(TabId),
    Crashed(TabId),
    Activated(TabId),
    Closed(TabId),
    /// A tab's latest frame, premultiplied RGBA at device pixels.
    Frame(TabId, Arc<egui::ColorImage>),
    /// Where the pointer's link goes, for the status line.
    HoveredLink(Option<String>),
    /// Text a page put on the clipboard, for the window to hand to the
    /// desktop's.
    Copied(String),
    /// A one-line message for the person: a finished download, a failure.
    Notice(String),
    Permission(PermissionAsk),
    /// The request was answered (or its tab closed); the bar goes away.
    PermissionDone(u64),
    /// How many content blocker rule sets are in force, for the shield.
    Filters(usize),
    /// WebKit is gone; the window closes.
    Quit,
}

/// How many commands may wait for WebKit, and how many events for the
/// window. Each side drains the other's channel whole every time it
/// wakes, so neither fills unless the other has hung; a sender then
/// blocks rather than piling up work, except for frames, which are
/// dropped (the next one is as good).
const COMMANDS: usize = 64;
const EVENTS: usize = 256;

/// The window's end of the command channel, cloned into every thread that
/// asks WebKit for something: the window, derisk's menus, later `danube`
/// runs and the captive portal's recheck.
#[derive(Clone)]
pub struct Commands {
    tx: Sender<Command>,
    /// Runs on the WebKit thread's main loop to drain the channel.
    wake: fn(),
}

impl Commands {
    pub fn channel(wake: fn()) -> (Self, Receiver<Command>) {
        let (tx, rx) = crossbeam_channel::bounded(COMMANDS);
        (Self { tx, wake }, rx)
    }

    /// Sends `command` to WebKit and wakes its main loop; `false` once
    /// WebKit has stopped.
    pub fn send(&self, command: Command) -> bool {
        let sent = self.tx.send(command).is_ok();
        (self.wake)();
        sent
    }
}

/// WebKit's end of the event channel, with the window's context once the
/// window has attached it, so that each event also repaints.
pub struct Events {
    tx: Sender<Event>,
    ctx: Option<egui::Context>,
    /// The command channel's sending end, for a signal handler that has
    /// to put a command of its own behind the one being handled.
    commands: Commands,
}

impl Events {
    pub fn channel(commands: Commands) -> (Self, Receiver<Event>) {
        let (tx, rx) = crossbeam_channel::bounded(EVENTS);
        (
            Self {
                tx,
                ctx: None,
                commands,
            },
            rx,
        )
    }

    pub fn attach(&mut self, ctx: egui::Context) {
        self.ctx = Some(ctx);
    }

    /// Queues `command` for WebKit's own thread to handle next.
    pub fn defer(&self, command: Command) {
        self.commands.send(command);
    }

    pub fn send(&self, event: Event) -> Result<(), SendError<Event>> {
        let sent = match event {
            // A frame the window has not drawn yet is replaced by the
            // next, never queued behind it.
            Event::Frame(..) => match self.tx.try_send(event) {
                Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
                Err(TrySendError::Disconnected(event)) => Err(SendError(event)),
            },
            event => self.tx.send(event),
        };
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        sent
    }
}
