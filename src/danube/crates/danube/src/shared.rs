//! What the window (ui.rs) and WebKit (engine.rs) hand each other. They
//! run on two threads: WebKit on the process's main thread under GLib's
//! main loop, the window on its own under winit's. The window sends
//! [`Command`]s; WebKit keeps [`State`] current and asks for a repaint.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui;

use crate::input::Input;

/// A tab, as long as it is open. Never reused.
pub type TabId = u64;

/// What the window shows for one tab.
#[derive(Clone, Debug, Default)]
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

/// Everything WebKit tells the window.
#[derive(Default)]
pub struct State {
    /// In tab-strip order.
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    /// The last frame each tab rendered, premultiplied RGBA at device
    /// pixels.
    pub frames: HashMap<TabId, Arc<egui::ColorImage>>,
    /// Where the pointer's link goes, for the status line.
    pub hovered_link: Option<String>,
    /// Text a page put on the clipboard, for the window to hand to the
    /// desktop's.
    pub copied: Option<String>,
    /// One-line messages for the person: a finished download, a failure.
    pub notices: VecDeque<String>,
    pub permissions: VecDeque<PermissionAsk>,
    /// How many content blocker rule sets are in force, for the shield.
    pub filters: usize,
    /// The window's title, which pairs it with derisk's window for the
    /// global menus (derisk.rs).
    pub window_title: String,
    /// WebKit is gone; the window closes.
    pub quit: bool,
}

/// What the window asks of WebKit.
#[derive(Debug)]
pub enum Command {
    NewTab { uri: Option<String>, activate: bool },
    Activate(TabId),
    Close(TabId),
    Load(TabId, String),
    Back(TabId),
    Forward(TabId),
    Reload(TabId),
    Stop(TabId),
    /// The page area, in logical pixels, and the window's scale.
    Resize { width: i32, height: i32, scale: f64 },
    /// The window gained or lost the keyboard.
    Focus(bool),
    Input(TabId, Vec<Input>),
    /// Paste `text` into the page: put it on WebKit's clipboard, then
    /// press Ctrl+V there.
    Paste(TabId, String),
    Permission { id: u64, allow: bool, remember: bool },
    /// The window was laid out for a phone (short side under 600 px) or
    /// not; sets the User-Agent string for the next load.
    Phone(bool),
    Quit,
}

/// The two threads' shared half.
pub struct Shared {
    pub state: Mutex<State>,
    commands: Mutex<Option<Sender<Command>>>,
    /// The window's context, once the window is up, to ask it to repaint.
    pub ctx: OnceLock<egui::Context>,
    /// Runs on the WebKit thread's main loop to drain the commands.
    wake: fn(),
}

impl Shared {
    pub fn new(commands: Sender<Command>, wake: fn()) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            commands: Mutex::new(Some(commands)),
            ctx: OnceLock::new(),
            wake,
        })
    }

    /// Sends `command` to WebKit and wakes its main loop.
    pub fn send(&self, command: Command) {
        if let Some(tx) = self.commands.lock().unwrap().as_ref() {
            let _ = tx.send(command);
        }
        (self.wake)();
    }

    /// Changes the state and repaints the window.
    pub fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().unwrap());
        self.repaint();
    }

    pub fn repaint(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }

    /// Adds a one-line message for the person.
    pub fn notify(&self, message: impl Into<String>) {
        self.update(|s| s.notices.push_back(message.into()));
    }
}
