//! WebKit's half of Danube, on the process's main thread under GLib's main
//! loop: one WebKitWebView per tab on WPE's headless display, each frame
//! copied out for the window to draw, the window's input turned into WPE
//! events, and the content blockers losos-adblock compiles loaded into
//! every page.
//!
//! The engine lives in a thread-local, since every WebKit object belongs to
//! this thread. Signal handlers get the tab's id as their data and mostly
//! touch only [`Shared`]; the few that need the engine itself borrow it
//! with [`try_with`] and give up cleanly (deny, block the popup) when a
//! command being handled already has it, which only happens when WebKit
//! emits a signal from inside a call the engine made.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_uint};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use eframe::egui;
use tracing::{info, warn};

use crate::callback;
use crate::config::{self, Settings};
use crate::ffi::*;
use crate::input::{Input, Touch};
use crate::permissions::{self, Kind};
use crate::shared::{Command, PermissionAsk, Shared, Tab, TabId};

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
    view: *mut WebKitWebView,
    wpe: *mut WPEView,
}

struct Engine {
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    main_loop: *mut GMainLoop,
    display: *mut WPEDisplay,
    session: *mut WebKitNetworkSession,
    settings: *mut WebKitSettings,
    content: *mut WebKitUserContentManager,
    tabs: Vec<Entry>,
    active: Option<TabId>,
    next_tab: TabId,
    size: (i32, i32, f64),
    focused: bool,
    modifiers: u32,
    buttons: u32,
    pointer: (f64, f64),
    pending: HashMap<u64, (*mut WebKitPermissionRequest, String, Kind)>,
    next_permission: u64,
    permissions: permissions::Store,
    home: String,
    desktop_agent: String,
    started: Instant,
}

/// The rule sets in force, kept apart from the engine because the store's
/// callbacks run on their own schedule.
struct Filters {
    content: *mut WebKitUserContentManager,
    store: *mut WebKitUserContentFilterStore,
    dir: PathBuf,
    /// The index's modification time when it was last read.
    stamp: Option<SystemTime>,
    /// Identifiers of the sets now loaded, `<id>-<mtime>`, so a changed set
    /// compiles again under a new name and the old one can be removed.
    current: Vec<String>,
    loaded: usize,
    shared: Arc<Shared>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static FILTERS: RefCell<Option<Filters>> = const { RefCell::new(None) };
    /// What the window last pasted into WebKit's clipboard, so that putting
    /// it there is not mistaken for the page copying it.
    static PASTED: RefCell<Option<String>> = const { RefCell::new(None) };
    static DOWNLOADS: RefCell<PathBuf> = RefCell::new(PathBuf::new());
}

fn try_with<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINE.with(|e| e.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Wakes the main loop to handle the window's commands. Safe from any
/// thread: g_idle_add is how GLib takes work from other threads.
pub fn wake() {
    unsafe extern "C" fn drain(_: gpointer) -> gboolean {
        let busy = try_with(|e| {
            while let Ok(command) = e.commands.try_recv() {
                e.handle(command);
            }
        })
        .is_none();
        // Busy only if a command's own WebKit call ran the loop; try again.
        if busy { TRUE } else { FALSE }
    }
    unsafe { g_idle_add_full(G_PRIORITY_DEFAULT, drain, ptr::null_mut(), None) };
}

/// Runs WebKit until the window closes or asks it to quit.
pub fn run(shared: Arc<Shared>, commands: Receiver<Command>, options: Options) {
    let _ = SHARED.set(shared.clone());
    unsafe {
        let display = wpe_display_headless_new();
        let mut error = ptr::null_mut();
        if wpe_display_connect(display, &mut error) == FALSE {
            warn!(error = take_error(error), "cannot connect WPE's headless display");
        }
        wpe_display_set_primary(display);

        let session = match &options.profile {
            Some((data, cache)) => {
                let data = cstring(&data.to_string_lossy());
                let cache = cstring(&cache.to_string_lossy());
                webkit_network_session_new(data.as_ptr(), cache.as_ptr())
            }
            None => webkit_network_session_new_ephemeral(),
        };
        // Tracking prevention: third-party cookies and storage are cut off
        // for sites that only ever appear as trackers.
        webkit_network_session_set_itp_enabled(session, TRUE);
        // No keyring: the sandbox has no Secret Service to talk to, and a
        // password manager extension keeps passwords.
        webkit_network_session_set_persistent_credential_storage_enabled(session, FALSE);
        connect(session.cast(), "download-started", callback!(on_download as unsafe extern "C" fn(_, _, _)), ptr::null_mut());
        DOWNLOADS.with(|d| *d.borrow_mut() = options.downloads.clone());

        let settings = webkit_settings_new();
        webkit_settings_set_enable_developer_extras(settings, FALSE);
        webkit_settings_set_javascript_can_open_windows_automatically(settings, FALSE);
        webkit_settings_set_allow_file_access_from_file_urls(settings, FALSE);
        webkit_settings_set_allow_universal_access_from_file_urls(settings, FALSE);
        webkit_settings_set_allow_top_navigation_to_data_urls(settings, FALSE);
        let name = cstring("Danube");
        let version = cstring(env!("CARGO_PKG_VERSION"));
        webkit_settings_set_user_agent_with_application_details(settings, name.as_ptr(), version.as_ptr());
        let desktop_agent = string(webkit_settings_get_user_agent(settings)).unwrap_or_default();

        let content = webkit_user_content_manager_new();

        let clipboard = wpe_display_get_clipboard(display);
        connect(
            clipboard.cast(),
            "notify::change-count",
            callback!(on_clipboard as unsafe extern "C" fn(_, _, _)),
            Arc::as_ptr(&shared) as gpointer,
        );

        let main_loop = g_main_loop_new(ptr::null_mut(), FALSE);

        if let Some(dir) = &options.adblock {
            let _ = std::fs::create_dir_all(&options.filter_cache);
            let path = cstring(&options.filter_cache.to_string_lossy());
            let store = webkit_user_content_filter_store_new(path.as_ptr());
            FILTERS.with(|f| {
                *f.borrow_mut() = Some(Filters {
                    content,
                    store,
                    dir: dir.clone(),
                    stamp: None,
                    current: Vec::new(),
                    loaded: 0,
                    shared: shared.clone(),
                })
            });
            reload_filters();
            // losos-adblock compiles daily; a minute is soon enough to
            // pick up a new compile, and costs one stat.
            g_timeout_add_full(G_PRIORITY_DEFAULT, 60_000, check_filters, ptr::null_mut(), None);
        }

        let engine = Engine {
            shared: shared.clone(),
            commands,
            main_loop,
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
        };
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
        g_main_loop_run(main_loop);
        shared.update(|s| s.quit = true);
    }
}

impl Engine {
    fn handle(&mut self, command: Command) {
        match command {
            Command::NewTab { uri, activate } => {
                let id = self.open(uri.as_deref(), None);
                if activate {
                    self.activate(id);
                }
            }
            Command::Activate(id) => self.activate(id),
            Command::Close(id) => self.close(id),
            Command::Load(id, uri) => {
                if let Some(view) = self.view(id) {
                    let uri = cstring(&uri);
                    unsafe { webkit_web_view_load_uri(view, uri.as_ptr()) };
                }
            }
            Command::Back(id) => self.view(id).map_or((), |v| unsafe { webkit_web_view_go_back(v) }),
            Command::Forward(id) => {
                self.view(id).map_or((), |v| unsafe { webkit_web_view_go_forward(v) })
            }
            Command::Reload(id) => {
                if let Some(view) = self.view(id) {
                    self.shared.update(|s| {
                        if let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) {
                            t.crashed = false;
                        }
                    });
                    unsafe { webkit_web_view_reload(view) };
                }
            }
            Command::Stop(id) => {
                self.view(id).map_or((), |v| unsafe { webkit_web_view_stop_loading(v) })
            }
            Command::Resize { width, height, scale } => {
                let width = width.max(1);
                let height = height.max(1);
                if self.size != (width, height, scale) {
                    self.size = (width, height, scale);
                    for entry in &self.tabs {
                        self.fit(entry.wpe);
                    }
                }
            }
            Command::Focus(focused) => {
                self.focused = focused;
                if let Some(entry) = self.active.and_then(|id| self.entry(id)) {
                    let wpe = entry.wpe;
                    self.set_focus(wpe, focused);
                }
            }
            Command::Input(id, inputs) => {
                if let Some(wpe) = self.entry(id).map(|e| e.wpe) {
                    for input in inputs {
                        self.input(wpe, input);
                    }
                }
            }
            Command::Paste(id, text) => {
                PASTED.with(|p| *p.borrow_mut() = Some(text.clone()));
                unsafe {
                    let clipboard = wpe_display_get_clipboard(self.display);
                    let content = wpe_clipboard_content_new();
                    let text = cstring(&text);
                    wpe_clipboard_content_set_text(content, text.as_ptr());
                    wpe_clipboard_set_content(clipboard, content);
                    wpe_clipboard_content_unref(content);
                }
                if let Some(wpe) = self.entry(id).map(|e| e.wpe) {
                    let held = self.modifiers;
                    self.modifiers = WPE_MODIFIER_KEYBOARD_CONTROL;
                    self.input(wpe, Input::Key { keyval: 'v' as u32, pressed: true });
                    self.input(wpe, Input::Key { keyval: 'v' as u32, pressed: false });
                    self.modifiers = held;
                }
            }
            Command::Permission { id, allow, remember } => {
                if let Some((request, host, kind)) = self.pending.remove(&id) {
                    if remember {
                        self.permissions.set(&host, kind, allow);
                    }
                    unsafe {
                        if allow {
                            webkit_permission_request_allow(request);
                        } else {
                            webkit_permission_request_deny(request);
                        }
                        g_object_unref(request.cast());
                    }
                }
                self.shared.update(|s| s.permissions.retain(|p| p.id != id));
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
                let agent = cstring(&agent);
                unsafe { webkit_settings_set_user_agent(self.settings, agent.as_ptr()) };
            }
            Command::Quit => unsafe { g_main_loop_quit(self.main_loop) },
        }
    }

    fn entry(&self, id: TabId) -> Option<&Entry> {
        self.tabs.iter().find(|e| e.id == id)
    }

    fn view(&self, id: TabId) -> Option<*mut WebKitWebView> {
        self.entry(id).map(|e| e.view)
    }

    /// Opens a tab, loading `uri` (the home page when `None`), or, with
    /// `related`, the view WebKit asked for to open a popup in; that one
    /// loads by itself.
    fn open(&mut self, uri: Option<&str>, related: Option<*mut WebKitWebView>) -> TabId {
        let id = self.next_tab;
        self.next_tab += 1;
        let view: *mut WebKitWebView = unsafe {
            let p = |s: &str| cstring(s);
            let (settings, content, display, session, related_view) = (
                p("settings"),
                p("user-content-manager"),
                p("display"),
                p("network-session"),
                p("related-view"),
            );
            match related {
                // A popup shares its opener's process, display and session.
                Some(related) => g_object_new(
                    webkit_web_view_get_type(),
                    related_view.as_ptr(),
                    related,
                    settings.as_ptr(),
                    self.settings,
                    content.as_ptr(),
                    self.content,
                    ptr::null::<c_char>(),
                ),
                None => g_object_new(
                    webkit_web_view_get_type(),
                    display.as_ptr(),
                    self.display,
                    session.as_ptr(),
                    self.session,
                    settings.as_ptr(),
                    self.settings,
                    content.as_ptr(),
                    self.content,
                    ptr::null::<c_char>(),
                ),
            }
            .cast()
        };
        let wpe = unsafe { webkit_web_view_get_wpe_view(view) };
        let data = id as gpointer;
        unsafe {
            let notify = callback!(on_notify as unsafe extern "C" fn(_, _, _));
            for signal in ["notify::title", "notify::uri", "notify::estimated-load-progress", "notify::is-loading"] {
                connect(view.cast(), signal, notify, data);
            }
            connect(view.cast(), "load-changed", callback!(on_load_changed as unsafe extern "C" fn(_, _, _)), data);
            connect(view.cast(), "create", callback!(on_create as unsafe extern "C" fn(_, _, _) -> _), data);
            connect(view.cast(), "ready-to-show", callback!(on_ready_to_show as unsafe extern "C" fn(_, _)), data);
            connect(view.cast(), "close", callback!(on_close as unsafe extern "C" fn(_, _)), data);
            connect(view.cast(), "decide-policy", callback!(on_decide_policy as unsafe extern "C" fn(_, _, _, _) -> _), data);
            connect(view.cast(), "permission-request", callback!(on_permission as unsafe extern "C" fn(_, _, _) -> _), data);
            connect(view.cast(), "mouse-target-changed", callback!(on_mouse_target as unsafe extern "C" fn(_, _, _, _)), data);
            connect(view.cast(), "web-process-terminated", callback!(on_crashed as unsafe extern "C" fn(_, _, _)), data);
            connect(wpe.cast(), "buffer-rendered", callback!(on_frame as unsafe extern "C" fn(_, _, _)), data);
            wpe_view_set_visible(wpe, FALSE);
        }
        self.fit(wpe);
        self.tabs.push(Entry { id, view, wpe });
        self.shared.update(|s| {
            s.tabs.push(Tab { id, ..Tab::default() });
        });
        if related.is_none() {
            let uri = cstring(uri.unwrap_or(&self.home));
            unsafe { webkit_web_view_load_uri(view, uri.as_ptr()) };
        }
        id
    }

    fn activate(&mut self, id: TabId) {
        if self.entry(id).is_none() {
            return;
        }
        let previous = self.active.replace(id);
        for entry in &self.tabs {
            let shown = entry.id == id;
            unsafe { wpe_view_set_visible(entry.wpe, shown as gboolean) };
        }
        if let Some(old) = previous.filter(|old| *old != id).and_then(|old| self.entry(old)) {
            let wpe = old.wpe;
            self.set_focus(wpe, false);
        }
        let wpe = self.entry(id).unwrap().wpe;
        self.set_focus(wpe, self.focused);
        self.shared.update(|s| s.active = Some(id));
    }

    fn close(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|e| e.id == id) else {
            return;
        };
        let entry = self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = None;
            // The neighbour to the right takes its place, as in most
            // browsers, or the one to the left at the end of the strip.
            let next = self.tabs.get(index).or_else(|| self.tabs.last()).map(|e| e.id);
            if let Some(next) = next {
                self.activate(next);
            }
        }
        self.shared.update(|s| {
            s.tabs.retain(|t| t.id != id);
            s.frames.remove(&id);
            s.permissions.retain(|p| p.tab != id);
            if s.active == Some(id) {
                s.active = None;
            }
        });
        unsafe { g_object_unref(entry.view.cast()) };
        if self.tabs.is_empty() {
            // The last tab closed: the window goes too.
            unsafe { g_main_loop_quit(self.main_loop) };
        }
    }

    /// Gives a view the page area's size and scale.
    fn fit(&self, wpe: *mut WPEView) {
        let (width, height, scale) = self.size;
        unsafe {
            let toplevel = wpe_view_get_toplevel(wpe);
            if !toplevel.is_null() {
                wpe_toplevel_scale_changed(toplevel, scale);
                wpe_toplevel_resize(toplevel, width, height);
            }
        }
    }

    fn set_focus(&self, wpe: *mut WPEView, focused: bool) {
        unsafe {
            let toplevel = wpe_view_get_toplevel(wpe);
            if !toplevel.is_null() {
                wpe_toplevel_state_changed(toplevel, if focused { WPE_TOPLEVEL_STATE_ACTIVE } else { 0 });
            }
            if focused {
                wpe_view_focus_in(wpe);
            } else {
                wpe_view_focus_out(wpe);
            }
        }
    }

    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn input(&mut self, wpe: *mut WPEView, input: Input) {
        let time = self.time();
        let event = unsafe {
            match input {
                Input::Modifiers(m) => {
                    self.modifiers = m.0;
                    return;
                }
                Input::Move { x, y } => {
                    let (dx, dy) = (x - self.pointer.0, y - self.pointer.1);
                    self.pointer = (x, y);
                    wpe_event_pointer_move_new(
                        WPE_EVENT_POINTER_MOVE,
                        wpe,
                        WPE_INPUT_SOURCE_MOUSE,
                        time,
                        self.modifiers | self.buttons,
                        x,
                        y,
                        dx,
                        dy,
                    )
                }
                Input::Leave => wpe_event_pointer_move_new(
                    WPE_EVENT_POINTER_LEAVE,
                    wpe,
                    WPE_INPUT_SOURCE_MOUSE,
                    time,
                    self.modifiers,
                    self.pointer.0,
                    self.pointer.1,
                    0.0,
                    0.0,
                ),
                Input::Button { x, y, button, pressed } => {
                    self.pointer = (x, y);
                    let mask = match button {
                        1 => WPE_MODIFIER_POINTER_BUTTON1,
                        2 => WPE_MODIFIER_POINTER_BUTTON2,
                        _ => WPE_MODIFIER_POINTER_BUTTON3,
                    };
                    let (kind, count) = if pressed {
                        self.buttons |= mask;
                        (WPE_EVENT_POINTER_DOWN, wpe_view_compute_press_count(wpe, x, y, button, time))
                    } else {
                        self.buttons &= !mask;
                        (WPE_EVENT_POINTER_UP, 0)
                    };
                    wpe_event_pointer_button_new(
                        kind,
                        wpe,
                        WPE_INPUT_SOURCE_MOUSE,
                        time,
                        self.modifiers | self.buttons,
                        button,
                        x,
                        y,
                        count,
                    )
                }
                Input::Scroll { x, y, dx, dy, precise } => {
                    let (x, y) = if x.is_nan() { self.pointer } else { (x, y) };
                    wpe_event_scroll_new(
                        wpe,
                        if precise { WPE_INPUT_SOURCE_TOUCHPAD } else { WPE_INPUT_SOURCE_MOUSE },
                        time,
                        self.modifiers,
                        dx,
                        dy,
                        precise as gboolean,
                        FALSE,
                        x,
                        y,
                    )
                }
                Input::Key { keyval, pressed } => {
                    let keycode = keycode(self.display, keyval);
                    wpe_event_keyboard_new(
                        if pressed { WPE_EVENT_KEYBOARD_KEY_DOWN } else { WPE_EVENT_KEYBOARD_KEY_UP },
                        wpe,
                        WPE_INPUT_SOURCE_KEYBOARD,
                        time,
                        self.modifiers,
                        keycode,
                        keyval,
                    )
                }
                Input::Touch { id, x, y, phase } => {
                    let kind = match phase {
                        Touch::Down => WPE_EVENT_TOUCH_DOWN,
                        Touch::Move => WPE_EVENT_TOUCH_MOVE,
                        Touch::Up => WPE_EVENT_TOUCH_UP,
                        Touch::Cancel => WPE_EVENT_TOUCH_CANCEL,
                    };
                    wpe_event_touch_new(kind, wpe, WPE_INPUT_SOURCE_TOUCHSCREEN, time, self.modifiers, id, x, y)
                }
            }
        };
        if !event.is_null() {
            unsafe {
                wpe_view_event(wpe, event);
                wpe_event_unref(event);
            }
        }
    }
}

/// The hardware keycode the keymap has for `keyval`, which pages see as
/// `KeyboardEvent.code`; 0 when it has none.
fn keycode(display: *mut WPEDisplay, keyval: u32) -> u32 {
    unsafe {
        let keymap = wpe_display_get_keymap(display);
        if keymap.is_null() {
            return 0;
        }
        let mut entries = ptr::null_mut();
        let mut n = 0;
        // WPE sets `entries` only on success; checking it as well keeps the
        // dereference below sound if a keymap ever reports entries without one.
        if wpe_keymap_get_entries_for_keyval(keymap, keyval, &mut entries, &mut n) == FALSE
            || n == 0
            || entries.is_null()
        {
            return 0;
        }
        let code = (*entries).keycode;
        g_free(entries.cast());
        code
    }
}

/// The tab's state, read from its view.
unsafe fn refresh(view: *mut WebKitWebView, id: TabId) -> Tab {
    unsafe {
        let uri = string(webkit_web_view_get_uri(view)).unwrap_or_default();
        let mut certificate = ptr::null_mut();
        let mut errors: c_uint = 0;
        let secure = uri.starts_with("https://")
            && webkit_web_view_get_tls_info(view, &mut certificate, &mut errors) != FALSE
            && errors == 0;
        Tab {
            id,
            title: string(webkit_web_view_get_title(view)).unwrap_or_default(),
            uri,
            progress: webkit_web_view_get_estimated_load_progress(view),
            loading: webkit_web_view_is_loading(view) != FALSE,
            can_go_back: webkit_web_view_can_go_back(view) != FALSE,
            can_go_forward: webkit_web_view_can_go_forward(view) != FALSE,
            secure,
            crashed: false,
        }
    }
}

/// The shared state, for signal handlers, which must reach it even while a
/// command has the engine borrowed. Set once, before WebKit starts.
static SHARED: std::sync::OnceLock<Arc<Shared>> = std::sync::OnceLock::new();

fn the_shared() -> Arc<Shared> {
    SHARED.get().cloned().expect("set before WebKit starts")
}

unsafe extern "C" fn on_notify(view: *mut WebKitWebView, _: *mut GParamSpec, data: gpointer) {
    let id = data as TabId;
    let tab = unsafe { refresh(view, id) };
    the_shared().update(|s| {
        if let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) {
            let crashed = t.crashed;
            *t = Tab { crashed, ..tab };
        }
    });
}

unsafe extern "C" fn on_load_changed(view: *mut WebKitWebView, event: c_int, data: gpointer) {
    if event == WEBKIT_LOAD_STARTED {
        let id = data as TabId;
        the_shared().update(|s| {
            if let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) {
                t.crashed = false;
            }
        });
    }
    unsafe { on_notify(view, ptr::null_mut(), data) };
}

unsafe extern "C" fn on_frame(_: *mut WPEView, buffer: *mut WPEBuffer, data: gpointer) {
    let id = data as TabId;
    let image = unsafe {
        let width = wpe_buffer_get_width(buffer).max(0) as usize;
        let height = wpe_buffer_get_height(buffer).max(0) as usize;
        let mut error = ptr::null_mut();
        let bytes = wpe_buffer_import_to_pixels(buffer, &mut error);
        if bytes.is_null() {
            warn!(error = take_error(error), "cannot read a frame");
            return;
        }
        let mut len = 0;
        let data = g_bytes_get_data(bytes, &mut len) as *const u8;
        if data.is_null() || width == 0 || height == 0 {
            return;
        }
        let pixels = std::slice::from_raw_parts(data, len);
        to_image(pixels, width, height)
    };
    if let Some(image) = image {
        the_shared().update(|s| {
            s.frames.insert(id, Arc::new(image));
        });
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

unsafe extern "C" fn on_create(
    _: *mut WebKitWebView,
    _: *mut WebKitNavigationAction,
    data: gpointer,
) -> *mut WebKitWebView {
    let opener = data as TabId;
    try_with(|e| {
        let related = e.view(opener)?;
        let id = e.open(None, Some(related));
        e.view(id)
    })
    .flatten()
    .unwrap_or(ptr::null_mut())
}

unsafe extern "C" fn on_ready_to_show(_: *mut WebKitWebView, data: gpointer) {
    let id = data as TabId;
    if try_with(|e| e.activate(id)).is_none() {
        the_shared().send(Command::Activate(id));
    }
}

unsafe extern "C" fn on_close(_: *mut WebKitWebView, data: gpointer) {
    the_shared().send(Command::Close(data as TabId));
}

unsafe extern "C" fn on_crashed(_: *mut WebKitWebView, _reason: c_int, data: gpointer) {
    let id = data as TabId;
    warn!(tab = id, "a web process ended");
    the_shared().update(|s| {
        if let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) {
            t.crashed = true;
            t.loading = false;
        }
    });
}

unsafe extern "C" fn on_decide_policy(
    _: *mut WebKitWebView,
    decision: *mut WebKitPolicyDecision,
    kind: c_int,
    _: gpointer,
) -> gboolean {
    unsafe {
        match kind {
            // What the engine cannot show is saved instead.
            WEBKIT_POLICY_DECISION_TYPE_RESPONSE => {
                if webkit_response_policy_decision_is_mime_type_supported(decision) == FALSE {
                    webkit_policy_decision_download(decision);
                    return TRUE;
                }
                FALSE
            }
            // A middle click on a link opens it in a new tab behind this one.
            WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION => {
                let action = webkit_navigation_policy_decision_get_navigation_action(decision);
                if action.is_null() || webkit_navigation_action_get_mouse_button(action) != 2 {
                    return FALSE;
                }
                let request = webkit_navigation_action_get_request(action);
                let Some(uri) = string(webkit_uri_request_get_uri(request)) else {
                    return FALSE;
                };
                webkit_policy_decision_ignore(decision);
                the_shared().send(Command::NewTab { uri: Some(uri), activate: false });
                TRUE
            }
            _ => FALSE,
        }
    }
}

unsafe extern "C" fn on_permission(
    view: *mut WebKitWebView,
    request: *mut WebKitPermissionRequest,
    data: gpointer,
) -> gboolean {
    let tab = data as TabId;
    unsafe {
        let type_name = string(g_type_name_from_instance(request.cast())).unwrap_or_default();
        let kind = match Kind::from_type_name(&type_name) {
            Some(Kind::CameraAndMicrophone) => {
                let audio = webkit_user_media_permission_is_for_audio_device(request) != FALSE;
                let video = webkit_user_media_permission_is_for_video_device(request) != FALSE;
                if webkit_user_media_permission_is_for_display_device(request) != FALSE {
                    Some(Kind::Screen)
                } else if audio && video {
                    Some(Kind::CameraAndMicrophone)
                } else if video {
                    Some(Kind::Camera)
                } else {
                    Some(Kind::Microphone)
                }
            }
            other => other,
        };
        let Some(kind) = kind else {
            webkit_permission_request_deny(request);
            return TRUE;
        };
        let host = config::host(&string(webkit_web_view_get_uri(view)).unwrap_or_default());
        let asked = try_with(|e| match e.permissions.get(&host, kind) {
            Some(true) => webkit_permission_request_allow(request),
            Some(false) => webkit_permission_request_deny(request),
            None => {
                let id = e.next_permission;
                e.next_permission += 1;
                g_object_ref(request.cast());
                e.pending.insert(id, (request, host.clone(), kind));
                e.shared.update(|s| {
                    s.permissions.push_back(PermissionAsk { id, tab, host: host.clone(), kind })
                });
            }
        });
        if asked.is_none() {
            webkit_permission_request_deny(request);
        }
        TRUE
    }
}

unsafe extern "C" fn on_mouse_target(
    _: *mut WebKitWebView,
    result: *mut WebKitHitTestResult,
    _: c_uint,
    _: gpointer,
) {
    let link = unsafe {
        if webkit_hit_test_result_context_is_link(result) != FALSE {
            string(webkit_hit_test_result_get_link_uri(result))
        } else {
            None
        }
    };
    the_shared().update(|s| s.hovered_link = link);
}

unsafe extern "C" fn on_clipboard(clipboard: *mut WPEClipboard, _: *mut GParamSpec, data: gpointer) {
    let shared = unsafe { &*(data as *const Shared) };
    let text = unsafe {
        let content = wpe_clipboard_get_content(clipboard);
        if content.is_null() {
            return;
        }
        string(wpe_clipboard_content_get_text(content))
    };
    let Some(text) = text else { return };
    let ours = PASTED.with(|p| p.borrow().as_deref() == Some(text.as_str()));
    if !ours {
        shared.update(|s| s.copied = Some(text));
    }
}

unsafe extern "C" fn on_download(_: *mut WebKitNetworkSession, download: *mut WebKitDownload, _: gpointer) {
    unsafe {
        connect(download.cast(), "decide-destination", callback!(on_destination as unsafe extern "C" fn(_, _, _) -> _), ptr::null_mut());
        connect(download.cast(), "finished", callback!(on_downloaded as unsafe extern "C" fn(_, _)), ptr::null_mut());
        connect(download.cast(), "failed", callback!(on_download_failed as unsafe extern "C" fn(_, _, _)), ptr::null_mut());
    }
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

unsafe extern "C" fn on_destination(download: *mut WebKitDownload, suggested: *const c_char, _: gpointer) -> gboolean {
    let suggested = unsafe { string(suggested) }.unwrap_or_default();
    let dir = DOWNLOADS.with(|d| d.borrow().clone());
    let _ = std::fs::create_dir_all(&dir);
    let path = destination(&dir, &suggested);
    let path = cstring(&path.to_string_lossy());
    unsafe { webkit_download_set_destination(download, path.as_ptr()) };
    TRUE
}

unsafe extern "C" fn on_downloaded(download: *mut WebKitDownload, _: gpointer) {
    let path = unsafe { string(webkit_download_get_destination(download)) }.unwrap_or_default();
    let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path);
    the_shared().notify(format!("Downloaded {name} to Downloads"));
}

unsafe extern "C" fn on_download_failed(_: *mut WebKitDownload, error: *mut GError, _: gpointer) {
    // The error belongs to the signal; read it, do not free it.
    let message = unsafe { if error.is_null() { None } else { string((*error).message) } };
    the_shared().notify(format!("Download failed: {}", message.unwrap_or_default()));
}

/// Reloads every rule set if losos-adblock compiled since the last look.
unsafe extern "C" fn check_filters(_: gpointer) -> gboolean {
    reload_filters();
    TRUE
}

fn reload_filters() {
    let jobs = FILTERS.with(|f| {
        let mut f = f.borrow_mut();
        let f = f.as_mut()?;
        let index = f.dir.join("index");
        let stamp = std::fs::metadata(&index).and_then(|m| m.modified()).ok();
        if stamp == f.stamp {
            return None;
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
            let secs = modified.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            jobs.push((format!("{id}-{secs}"), path));
        }
        unsafe { webkit_user_content_manager_remove_all_filters(f.content) };
        f.loaded = 0;
        f.current = jobs.iter().map(|(ident, _)| ident.clone()).collect();
        f.shared.update(|s| s.filters = 0);
        Some((f.store, jobs))
    });
    let Some((store, jobs)) = jobs else { return };
    info!(sets = jobs.len(), "loading content blockers");
    for (ident, path) in jobs {
        let job = Box::into_raw(Box::new((ident.clone(), path)));
        let ident = cstring(&ident);
        unsafe { webkit_user_content_filter_store_load(store, ident.as_ptr(), ptr::null_mut(), on_filter_loaded, job.cast()) };
    }
    unsafe { webkit_user_content_filter_store_fetch_identifiers(store, ptr::null_mut(), on_identifiers, ptr::null_mut()) };
}

fn add_filter(filter: *mut WebKitUserContentFilter) {
    FILTERS.with(|f| {
        if let Some(f) = f.borrow_mut().as_mut() {
            unsafe { webkit_user_content_manager_add_filter(f.content, filter) };
            f.loaded += 1;
            let loaded = f.loaded;
            f.shared.update(|s| s.filters = loaded);
        }
    });
    unsafe { webkit_user_content_filter_unref(filter) };
}

/// A set compiled before is loaded as it is; one that is new or changed is
/// compiled from losos-adblock's JSON, which takes WebKit a few seconds
/// for a big list.
unsafe extern "C" fn on_filter_loaded(source: *mut GObject, result: *mut GAsyncResult, data: gpointer) {
    let store = source as *mut WebKitUserContentFilterStore;
    let mut error = ptr::null_mut();
    let filter = unsafe { webkit_user_content_filter_store_load_finish(store, result, &mut error) };
    if !filter.is_null() {
        drop(unsafe { Box::from_raw(data as *mut (String, PathBuf)) });
        add_filter(filter);
        return;
    }
    unsafe { g_error_free(error) };
    let (ident, path) = unsafe { &*(data as *const (String, PathBuf)) };
    let ident = cstring(ident);
    let path = cstring(&path.to_string_lossy());
    unsafe {
        let file = g_file_new_for_path(path.as_ptr());
        webkit_user_content_filter_store_save_from_file(store, ident.as_ptr(), file, ptr::null_mut(), on_filter_saved, data);
        g_object_unref(file.cast());
    }
}

unsafe extern "C" fn on_filter_saved(source: *mut GObject, result: *mut GAsyncResult, data: gpointer) {
    let store = source as *mut WebKitUserContentFilterStore;
    let job = unsafe { Box::from_raw(data as *mut (String, PathBuf)) };
    let mut error = ptr::null_mut();
    let filter = unsafe { webkit_user_content_filter_store_save_from_file_finish(store, result, &mut error) };
    if filter.is_null() {
        warn!(set = %job.1.display(), error = unsafe { take_error(error) }, "WebKit refused a rule set");
        return;
    }
    add_filter(filter);
}

/// Removes the compiled sets no longer named, so the cache does not grow
/// with every daily compile.
unsafe extern "C" fn on_identifiers(source: *mut GObject, result: *mut GAsyncResult, _: gpointer) {
    let store = source as *mut WebKitUserContentFilterStore;
    let list = unsafe { webkit_user_content_filter_store_fetch_identifiers_finish(store, result) };
    if list.is_null() {
        return;
    }
    let current = FILTERS.with(|f| f.borrow().as_ref().map(|f| f.current.clone()).unwrap_or_default());
    let mut at = list;
    unsafe {
        while !(*at).is_null() {
            if let Some(ident) = string(*at) {
                if !current.contains(&ident) {
                    webkit_user_content_filter_store_remove(store, *at, ptr::null_mut(), None, ptr::null_mut());
                }
            }
            at = at.add(1);
        }
        g_strfreev(list);
    }
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
