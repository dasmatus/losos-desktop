//! WebKit's half of Danube, on the process's main thread under GLib's main
//! loop: one [`WebView`] per tab on WPE's headless display, each frame
//! copied out for the window to draw, the window's input turned into WPE
//! events, and the content blockers losos-adblock compiles loaded into
//! every page.
//!
//! The engine lives in a thread-local, since every WebKit object belongs to
//! this thread. Signal handlers are closures that know their tab's id and
//! mostly only send the window an [`Event`] (through [`LINK`], another
//! thread-local); the few that need the engine itself borrow it with
//! [`try_with`] and give up cleanly (deny, block the popup) when a command
//! being handled already has it, which only happens when WebKit emits a
//! signal from inside a call the engine made.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crossbeam_channel::Receiver;
use eframe::egui;
use tracing::{debug, info, trace, warn};

use crate::config::{self, Settings};
use crate::ffi::{
    WEBKIT_LOAD_STARTED, WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION,
    WEBKIT_POLICY_DECISION_TYPE_RESPONSE, WPE_EVENT_KEYBOARD_KEY_DOWN, WPE_EVENT_KEYBOARD_KEY_UP,
    WPE_EVENT_POINTER_DOWN, WPE_EVENT_POINTER_LEAVE, WPE_EVENT_POINTER_MOVE, WPE_EVENT_POINTER_UP,
    WPE_EVENT_TOUCH_CANCEL, WPE_EVENT_TOUCH_DOWN, WPE_EVENT_TOUCH_MOVE, WPE_EVENT_TOUCH_UP,
    WPE_INPUT_SOURCE_MOUSE, WPE_INPUT_SOURCE_TOUCHPAD, WPE_MODIFIER_KEYBOARD_CONTROL,
    WPE_MODIFIER_POINTER_BUTTON1, WPE_MODIFIER_POINTER_BUTTON2, WPE_MODIFIER_POINTER_BUTTON3,
};
use crate::input::{Input, Touch};
use crate::permissions::{self, Kind};
use crate::shared::{Command, Event, Events, PermissionAsk, Tab, TabId};
use crate::webkit::{
    self, Buffer, Display, Download, FilterStore, MainLoop, NetworkSession, PermissionRequest,
    PolicyDecision, UserContentManager, View, WebView,
};

/// How the engine starts.
pub struct Options {
    /// Where the profile lives (cookies, storage), or `None` for a
    /// throwaway one that ends with the process (captive portal).
    pub profile: Option<(PathBuf, PathBuf)>,
    /// losos-adblock's compiled rule sets, or `None` to load none.
    pub adblock: Option<PathBuf>,
    /// Where WebKit keeps the rule sets it compiled from them.
    pub filter_cache: PathBuf,
    /// Where lasting permission answers are kept.
    pub permissions: Option<PathBuf>,
    /// Where downloads go.
    pub downloads: PathBuf,
    pub settings: Settings,
    /// What to open first; the home page when empty.
    pub open: Vec<String>,
}

struct Entry {
    id: TabId,
    page: WebView,
    view: View,
}

struct Engine {
    commands: Receiver<Command>,
    main_loop: MainLoop,
    display: Display,
    session: NetworkSession,
    settings: webkit::Settings,
    content: UserContentManager,
    tabs: Vec<Entry>,
    active: Option<TabId>,
    next_tab: TabId,
    size: (i32, i32, f64),
    focused: bool,
    modifiers: u32,
    buttons: u32,
    pointer: (f64, f64),
    pending: HashMap<u64, (PermissionRequest, String, Kind)>,
    next_permission: u64,
    permissions: permissions::Store,
    home: String,
    desktop_agent: String,
    started: Instant,
    /// What the window last pasted into WebKit's clipboard, so that
    /// putting it there is not mistaken for the page copying it; shared
    /// with the clipboard's handler.
    pasted: Rc<RefCell<Option<String>>>,
}

/// The rule sets in force, kept apart from the engine because the store's
/// callbacks run on their own schedule.
struct Filters {
    content: UserContentManager,
    store: FilterStore,
    dir: PathBuf,
    /// The index's modification time when it was last read.
    stamp: Option<SystemTime>,
    /// Identifiers of the sets now loaded, `<id>-<mtime>`, so a changed set
    /// compiles again under a new name and the old one can be removed.
    current: Vec<String>,
    loaded: usize,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    /// The channel to the window, for signal handlers, which must reach
    /// it even while a command has the engine borrowed.
    static LINK: RefCell<Option<Events>> = const { RefCell::new(None) };
}

fn try_with<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINE.with(|e| e.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Tells the window `event`, and repaints it. The window being gone is
/// not an error here: WebKit is about to quit on its command.
fn emit(event: Event) {
    LINK.with(|l| {
        if let Some(link) = l.borrow().as_ref() {
            let _ = link.send(event);
        }
    });
}

/// Sends the engine a command of its own, through the window's channel,
/// for a handler that cannot borrow the engine right now.
fn later(command: Command) {
    LINK.with(|l| {
        if let Some(link) = l.borrow().as_ref() {
            link.defer(command);
        }
    });
}

/// Wakes the main loop to handle the window's commands. Safe from any
/// thread.
pub fn wake() {
    webkit::idle_add(|| {
        let busy = try_with(|e| {
            while let Ok(command) = e.commands.try_recv() {
                e.handle(command);
            }
        })
        .is_none();
        // Busy only if a command's own WebKit call ran the loop; try again.
        busy
    });
}

/// Runs WebKit until the window closes or asks it to quit.
pub fn run(commands: Receiver<Command>, events: Events, options: Options) {
    LINK.with(|l| *l.borrow_mut() = Some(events));

    let display = match Display::headless() {
        Ok(display) => display,
        Err(error) => {
            warn!(error, "cannot connect WPE's headless display");
            return;
        }
    };

    let session = match &options.profile {
        Some((data, cache)) => NetworkSession::persistent(data, cache),
        None => NetworkSession::ephemeral(),
    };
    // Tracking prevention: third-party cookies and storage are cut off
    // for sites that only ever appear as trackers.
    session.set_itp_enabled(true);
    // No keyring: the sandbox has no Secret Service to talk to, and a
    // password manager extension keeps passwords.
    session.set_persistent_credential_storage_enabled(false);
    let downloads = options.downloads.clone();
    session.on_download_started(move |_, download| watch_download(download, &downloads));

    let settings = webkit::Settings::new();
    settings.set_enable_developer_extras(false);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_allow_file_access_from_file_urls(false);
    settings.set_allow_universal_access_from_file_urls(false);
    settings.set_allow_top_navigation_to_data_urls(false);
    settings.set_application("Danube", env!("CARGO_PKG_VERSION"));
    let desktop_agent = settings.user_agent();

    let content = UserContentManager::new();

    let pasted = Rc::new(RefCell::new(None));
    let ours = pasted.clone();
    display.clipboard().on_change(move |clipboard| {
        let Some(text) = clipboard.text() else { return };
        if ours.borrow().as_deref() != Some(text.as_str()) {
            emit(Event::Copied(text));
        }
    });

    if let Some(dir) = &options.adblock {
        let _ = std::fs::create_dir_all(&options.filter_cache);
        let filters = Rc::new(RefCell::new(Filters {
            content: content.clone(),
            store: FilterStore::new(&options.filter_cache),
            dir: dir.clone(),
            stamp: None,
            current: Vec::new(),
            loaded: 0,
        }));
        reload_filters(&filters);
        // losos-adblock compiles daily; a minute is soon enough to pick
        // up a new compile, and costs one stat.
        webkit::timeout_add(60_000, move || {
            reload_filters(&filters);
            true
        });
    }

    let engine = Engine {
        commands,
        main_loop: MainLoop::new(),
        display,
        session,
        settings,
        content,
        tabs: Vec::new(),
        active: None,
        next_tab: 1,
        size: (800, 600, 1.0),
        focused: true,
        modifiers: 0,
        buttons: 0,
        pointer: (0.0, 0.0),
        pending: HashMap::new(),
        next_permission: 1,
        permissions: permissions::Store::open(options.permissions.clone()),
        home: options.settings.home.clone(),
        desktop_agent,
        started: Instant::now(),
        pasted,
    };
    let main_loop = engine.main_loop.clone();
    ENGINE.with(|e| *e.borrow_mut() = Some(engine));

    let open = if options.open.is_empty() {
        vec![options.settings.home.clone()]
    } else {
        options.open.clone()
    };
    try_with(|e| {
        for (n, uri) in open.iter().enumerate() {
            let id = e.open(Some(uri), None);
            if n == 0 {
                e.activate(id);
            }
        }
    });
    // Commands the window sent before the engine existed.
    wake();

    info!("WebKit is up");
    main_loop.run();

    emit(Event::Quit);
    // Dropping the channels: senders of commands see WebKit is gone.
    ENGINE.with(|e| e.borrow_mut().take());
    LINK.with(|l| l.borrow_mut().take());
}

impl Engine {
    fn handle(&mut self, command: Command) {
        match command {
            Command::Attach(ctx) => LINK.with(|l| {
                if let Some(link) = l.borrow_mut().as_mut() {
                    link.attach(ctx);
                }
            }),
            Command::NewTab { uri, activate } => {
                let id = self.open(uri.as_deref(), None);
                if activate {
                    self.activate(id);
                }
            }
            Command::Activate(id) => self.activate(id),
            Command::Close(id) => self.close(id),
            Command::Load(id, uri) => {
                if let Some(page) = self.page(id) {
                    page.load_uri(&uri);
                }
            }
            Command::Back(id) => self.page(id).map_or((), WebView::go_back),
            Command::Forward(id) => self.page(id).map_or((), WebView::go_forward),
            Command::Reload(id) => self.page(id).map_or((), WebView::reload),
            Command::Stop(id) => self.page(id).map_or((), WebView::stop_loading),
            Command::Resize {
                width,
                height,
                scale,
            } => {
                let width = width.max(1);
                let height = height.max(1);
                if self.size != (width, height, scale) {
                    self.size = (width, height, scale);
                    for entry in &self.tabs {
                        self.fit(&entry.view);
                    }
                }
            }
            Command::Focus(focused) => {
                self.focused = focused;
                if let Some(view) = self.active.and_then(|id| self.view(id)) {
                    self.set_focus(&view, focused);
                }
            }
            Command::Input(id, inputs) => {
                if let Some(view) = self.view(id) {
                    for input in inputs {
                        self.input(&view, input);
                    }
                }
            }
            Command::Paste(id, text) => {
                *self.pasted.borrow_mut() = Some(text.clone());
                self.display.clipboard().set_text(&text);
                if let Some(view) = self.view(id) {
                    let held = self.modifiers;
                    self.modifiers = WPE_MODIFIER_KEYBOARD_CONTROL;
                    self.input(
                        &view,
                        Input::Key {
                            keyval: 'v' as u32,
                            pressed: true,
                        },
                    );
                    self.input(
                        &view,
                        Input::Key {
                            keyval: 'v' as u32,
                            pressed: false,
                        },
                    );
                    self.modifiers = held;
                }
            }
            Command::Permission {
                id,
                allow,
                remember,
            } => {
                if let Some((request, host, kind)) = self.pending.remove(&id) {
                    if remember {
                        self.permissions.set(&host, kind, allow);
                    }
                    if allow {
                        request.allow();
                    } else {
                        request.deny();
                    }
                }
                emit(Event::PermissionDone(id));
            }
            Command::Phone(phone) => {
                let agent = if phone {
                    // What phone sites look for: "Mobile", and "Android"
                    // for the many that only check for it. The engine is
                    // still WebKit's, as the rest of the string says.
                    format!(
                        "Mozilla/5.0 (Linux; Android 14; Mobile) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile Safari/605.1.15 Danube/{}",
                        env!("CARGO_PKG_VERSION")
                    )
                } else {
                    self.desktop_agent.clone()
                };
                self.settings.set_user_agent(&agent);
            }
            Command::Quit => self.main_loop.quit(),
        }
    }

    fn entry(&self, id: TabId) -> Option<&Entry> {
        self.tabs.iter().find(|e| e.id == id)
    }

    fn page(&self, id: TabId) -> Option<&WebView> {
        self.entry(id).map(|e| &e.page)
    }

    fn view(&self, id: TabId) -> Option<View> {
        self.entry(id).map(|e| e.view.clone())
    }

    /// Opens a tab, loading `uri` (the home page when `None`), or, with
    /// `related`, the view WebKit asked for to open a popup in; that one
    /// loads by itself.
    fn open(&mut self, uri: Option<&str>, related: Option<&WebView>) -> TabId {
        let id = self.next_tab;
        self.next_tab += 1;
        let page = match related {
            // A popup shares its opener's process, display and session.
            Some(related) => WebView::related(related, &self.settings, &self.content),
            None => WebView::new(&self.display, &self.session, &self.settings, &self.content),
        };
        for property in ["title", "uri", "estimated-load-progress", "is-loading"] {
            page.on_notify(property, move |page| {
                emit(Event::Changed(refresh(page, id)))
            });
        }
        page.on_load_changed(move |page, event| {
            debug!(tab = id, event, "load changed");
            if event == WEBKIT_LOAD_STARTED {
                emit(Event::LoadStarted(id));
            }
            emit(Event::Changed(refresh(page, id)));
        });
        page.on_create(move |_| {
            try_with(|e| {
                let related = e.page(id)?.clone();
                let new = e.open(None, Some(&related));
                e.page(new).cloned()
            })
            .flatten()
        });
        page.on_ready_to_show(move |_| {
            if try_with(|e| e.activate(id)).is_none() {
                later(Command::Activate(id));
            }
        });
        page.on_close(move |_| later(Command::Close(id)));
        page.on_web_process_terminated(move |_| {
            warn!(tab = id, "a web process ended");
            emit(Event::Crashed(id));
        });
        page.on_decide_policy(|_, decision, kind| decide(decision, kind));
        page.on_permission_request(move |page, request| ask(page, request, id));
        page.on_mouse_target_changed(|_, result| emit(Event::HoveredLink(result.link_uri())));
        let view = page.view();
        view.on_buffer_rendered(move |_, buffer| frame(buffer, id));
        view.set_visible(false);
        self.fit(&view);
        self.tabs.push(Entry { id, page, view });
        emit(Event::Opened(Tab {
            id,
            ..Tab::default()
        }));
        if related.is_none() {
            let uri = uri.unwrap_or(&self.home);
            self.tabs.last().expect("just pushed").page.load_uri(uri);
        }
        id
    }

    fn activate(&mut self, id: TabId) {
        if self.entry(id).is_none() {
            return;
        }
        let previous = self.active.replace(id);
        for entry in &self.tabs {
            entry.view.set_visible(entry.id == id);
        }
        if let Some(old) = previous
            .filter(|old| *old != id)
            .and_then(|old| self.view(old))
        {
            self.set_focus(&old, false);
        }
        let view = self.view(id).expect("checked above");
        self.set_focus(&view, self.focused);
        emit(Event::Activated(id));
    }

    fn close(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|e| e.id == id) else {
            return;
        };
        // Dropping the entry drops the view's last reference.
        drop(self.tabs.remove(index));
        if self.active == Some(id) {
            self.active = None;
            // The neighbour to the right takes its place, as in most
            // browsers, or the one to the left at the end of the strip.
            let next = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|e| e.id);
            if let Some(next) = next {
                self.activate(next);
            }
        }
        emit(Event::Closed(id));
        if self.tabs.is_empty() {
            // The last tab closed: the window goes too.
            self.main_loop.quit();
        }
    }

    /// Gives a view the page area's size and scale.
    fn fit(&self, view: &View) {
        let (width, height, scale) = self.size;
        if let Some(toplevel) = view.toplevel() {
            toplevel.set_scale(scale);
            toplevel.resize(width, height);
        }
    }

    fn set_focus(&self, view: &View, focused: bool) {
        if let Some(toplevel) = view.toplevel() {
            toplevel.set_active(focused);
        }
        view.set_focused(focused);
    }

    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn input(&mut self, view: &View, input: Input) {
        let time = self.time();
        let event = match input {
            Input::Modifiers(m) => {
                self.modifiers = m.0;
                return;
            }
            Input::Move { x, y } => {
                let (dx, dy) = (x - self.pointer.0, y - self.pointer.1);
                self.pointer = (x, y);
                webkit::Event::pointer_move(
                    WPE_EVENT_POINTER_MOVE,
                    view,
                    WPE_INPUT_SOURCE_MOUSE,
                    time,
                    self.modifiers | self.buttons,
                    x,
                    y,
                    dx,
                    dy,
                )
            }
            Input::Leave => webkit::Event::pointer_move(
                WPE_EVENT_POINTER_LEAVE,
                view,
                WPE_INPUT_SOURCE_MOUSE,
                time,
                self.modifiers,
                self.pointer.0,
                self.pointer.1,
                0.0,
                0.0,
            ),
            Input::Button {
                x,
                y,
                button,
                pressed,
            } => {
                self.pointer = (x, y);
                let mask = match button {
                    1 => WPE_MODIFIER_POINTER_BUTTON1,
                    2 => WPE_MODIFIER_POINTER_BUTTON2,
                    _ => WPE_MODIFIER_POINTER_BUTTON3,
                };
                let (kind, count) = if pressed {
                    self.buttons |= mask;
                    (WPE_EVENT_POINTER_DOWN, view.press_count(x, y, button, time))
                } else {
                    self.buttons &= !mask;
                    (WPE_EVENT_POINTER_UP, 0)
                };
                webkit::Event::pointer_button(
                    kind,
                    view,
                    WPE_INPUT_SOURCE_MOUSE,
                    time,
                    self.modifiers | self.buttons,
                    button,
                    x,
                    y,
                    count,
                )
            }
            Input::Scroll {
                x,
                y,
                dx,
                dy,
                precise,
            } => {
                let (x, y) = if x.is_nan() { self.pointer } else { (x, y) };
                let source = if precise {
                    WPE_INPUT_SOURCE_TOUCHPAD
                } else {
                    WPE_INPUT_SOURCE_MOUSE
                };
                webkit::Event::scroll(view, source, time, self.modifiers, dx, dy, precise, x, y)
            }
            Input::Key { keyval, pressed } => {
                let kind = if pressed {
                    WPE_EVENT_KEYBOARD_KEY_DOWN
                } else {
                    WPE_EVENT_KEYBOARD_KEY_UP
                };
                let keycode = self.display.keycode(keyval);
                webkit::Event::keyboard(kind, view, time, self.modifiers, keycode, keyval)
            }
            Input::Touch { id, x, y, phase } => {
                let kind = match phase {
                    Touch::Down => WPE_EVENT_TOUCH_DOWN,
                    Touch::Move => WPE_EVENT_TOUCH_MOVE,
                    Touch::Up => WPE_EVENT_TOUCH_UP,
                    Touch::Cancel => WPE_EVENT_TOUCH_CANCEL,
                };
                webkit::Event::touch(kind, view, time, self.modifiers, id, x, y)
            }
        };
        view.send(&event);
    }
}

/// The tab's state, read from its page.
fn refresh(page: &WebView, id: TabId) -> Tab {
    Tab {
        id,
        title: page.title(),
        uri: page.uri(),
        progress: page.estimated_load_progress(),
        loading: page.is_loading(),
        can_go_back: page.can_go_back(),
        can_go_forward: page.can_go_forward(),
        secure: page.is_secure(),
        crashed: false,
    }
}

/// A rendered frame, copied out for the window.
fn frame(buffer: &Buffer, id: TabId) {
    let (width, height) = (buffer.width(), buffer.height());
    trace!(tab = id, width, height, "frame");
    let pixels = match buffer.pixels() {
        Ok(pixels) => pixels,
        Err(error) => {
            warn!(error, "cannot read a frame");
            return;
        }
    };
    if width == 0 || height == 0 {
        return;
    }
    if let Some(image) = to_image(pixels, width, height) {
        emit(Event::Frame(id, Arc::new(image)));
    }
}

/// A frame in WPE's ARGB8888 (B, G, R, A bytes, premultiplied) as egui's
/// premultiplied RGBA; `None` when the buffer is too short for its size.
pub fn to_image(pixels: &[u8], width: usize, height: usize) -> Option<egui::ColorImage> {
    let stride = pixels.len() / height;
    if stride < width * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(width * height);
    for row in pixels.chunks_exact(stride).take(height) {
        out.extend(
            row[..width * 4]
                .chunks_exact(4)
                .map(|p| egui::Color32::from_rgba_premultiplied(p[2], p[1], p[0], p[3])),
        );
    }
    Some(egui::ColorImage::new([width, height], out))
}

/// `true` once the decision is made here rather than left to WebKit.
fn decide(decision: &PolicyDecision, kind: u32) -> bool {
    match kind {
        // What the engine cannot show is saved instead.
        WEBKIT_POLICY_DECISION_TYPE_RESPONSE => {
            if decision.is_mime_type_supported() {
                return false;
            }
            decision.download();
            true
        }
        // A middle click on a link opens it in a new tab behind this one.
        WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION => {
            if decision.mouse_button() != 2 {
                return false;
            }
            let Some(uri) = decision.uri() else {
                return false;
            };
            decision.ignore();
            later(Command::NewTab {
                uri: Some(uri),
                activate: false,
            });
            true
        }
        _ => false,
    }
}

/// Answers a permission request from what the person said before, or
/// asks them; always `true`, the request is this engine's to answer.
fn ask(page: &WebView, request: PermissionRequest, tab: TabId) -> bool {
    let kind = match Kind::from_type_name(&request.type_name()) {
        Some(Kind::CameraAndMicrophone) => {
            let (audio, video, display) = request.user_media();
            Some(if display {
                Kind::Screen
            } else if audio && video {
                Kind::CameraAndMicrophone
            } else if video {
                Kind::Camera
            } else {
                Kind::Microphone
            })
        }
        other => other,
    };
    let Some(kind) = kind else {
        request.deny();
        return true;
    };
    let host = config::host(&page.uri());
    let asked = try_with(|e| match e.permissions.get(&host, kind) {
        Some(true) => request.allow(),
        Some(false) => request.deny(),
        None => {
            let id = e.next_permission;
            e.next_permission += 1;
            e.pending.insert(id, (request.clone(), host.clone(), kind));
            emit(Event::Permission(PermissionAsk {
                id,
                tab,
                host: host.clone(),
                kind,
            }));
        }
    });
    if asked.is_none() {
        request.deny();
    }
    true
}

/// Saves a download into `dir` and tells the window how it went.
fn watch_download(download: &Download, dir: &Path) {
    let dir = dir.to_path_buf();
    download.on_decide_destination(move |download, suggested| {
        let _ = std::fs::create_dir_all(&dir);
        download.set_destination(&destination(&dir, suggested));
        true
    });
    download.on_finished(|download| {
        let path = download.destination().unwrap_or_default();
        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(path);
        emit(Event::Notice(format!("Downloaded {name} to Downloads")));
    });
    download.on_failed(|_, message| emit(Event::Notice(format!("Download failed: {message}"))));
}

/// A file name for `suggested` in `dir` that is not taken: "name.ext",
/// then "name (1).ext", and so on, with anything that could leave the
/// directory taken out.
pub fn destination(dir: &Path, suggested: &str) -> PathBuf {
    let name: String = suggested
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name = name.trim_start_matches('.').trim();
    let name = if name.is_empty() { "download" } else { name };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    let mut path = dir.join(name);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    path
}

/// Reloads every rule set if losos-adblock compiled since the last look.
fn reload_filters(filters: &Rc<RefCell<Filters>>) {
    let jobs = {
        let mut f = filters.borrow_mut();
        let index = f.dir.join("index");
        let stamp = std::fs::metadata(&index).and_then(|m| m.modified()).ok();
        if stamp == f.stamp {
            return;
        }
        f.stamp = stamp;
        let text = std::fs::read_to_string(&index).unwrap_or_default();
        let mut jobs = Vec::new();
        for id in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if id.contains(['/', '.']) {
                continue;
            }
            let path = f.dir.join(format!("{id}.json"));
            let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                continue;
            };
            let secs = modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            jobs.push((format!("{id}-{secs}"), path));
        }
        f.content.remove_all_filters();
        f.loaded = 0;
        f.current = jobs.iter().map(|(ident, _)| ident.clone()).collect();
        emit(Event::Filters(0));
        jobs
    };
    info!(sets = jobs.len(), "loading content blockers");
    let store = filters.borrow().store.clone();
    for (ident, path) in jobs {
        // A set compiled before is loaded as it is; one that is new or
        // changed is compiled from losos-adblock's JSON, which takes WebKit
        // a few seconds for a big list.
        let (filters, store2) = (filters.clone(), store.clone());
        store.load(&ident.clone(), move |loaded| match loaded {
            Ok(filter) => add_filter(&filters, &filter),
            Err(_) => {
                let (filters, set) = (filters.clone(), path.clone());
                store2.save_from_file(&ident, &set, move |saved| match saved {
                    Ok(filter) => add_filter(&filters, &filter),
                    Err(error) => {
                        warn!(set = %path.display(), error, "WebKit refused a rule set")
                    }
                });
            }
        });
    }
    // Removes the compiled sets no longer named, so the cache does not
    // grow with every daily compile.
    let (filters, store2) = (filters.clone(), store.clone());
    store.fetch_identifiers(move |identifiers| {
        let current = filters.borrow().current.clone();
        for ident in identifiers {
            if !current.contains(&ident) {
                store2.remove(&ident);
            }
        }
    });
}

fn add_filter(filters: &Rc<RefCell<Filters>>, filter: &webkit::Filter) {
    let mut f = filters.borrow_mut();
    f.content.add_filter(filter);
    f.loaded += 1;
    emit(Event::Filters(f.loaded));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_drop_their_row_padding() {
        // 2x2 pixels, rows padded to 12 bytes: BGRA blue, then red.
        let row = [255, 0, 0, 255, 0, 0, 255, 255, 9, 9, 9, 9];
        let pixels: Vec<u8> = row.iter().chain(row.iter()).copied().collect();
        let image = to_image(&pixels, 2, 2).unwrap();
        assert_eq!(image.pixels[0], egui::Color32::from_rgb(0, 0, 255));
        assert_eq!(image.pixels[1], egui::Color32::from_rgb(255, 0, 0));
        assert_eq!(image.pixels.len(), 4);
    }

    #[test]
    fn downloads_never_overwrite_or_escape() {
        let dir = std::env::temp_dir().join(format!("danube-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(destination(&dir, "../../etc/passwd"), dir.join("passwd"));
        assert_eq!(destination(&dir, ".bashrc"), dir.join("bashrc"));
        std::fs::write(dir.join("a.pdf"), "").unwrap();
        assert_eq!(destination(&dir, "a.pdf"), dir.join("a (1).pdf"));
        assert_eq!(destination(&dir, ""), dir.join("download"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
