//! WebKit's and WPE's objects as Rust types that own them: each holds one
//! reference, takes another when cloned and gives it back when dropped,
//! and offers the calls Danube makes as methods on `&self`. Signals take
//! closures, which GObject frees with the connection, and hand them the
//! emitting object borrowed. The `unsafe` of ffi.rs ends here: engine.rs
//! never sees a pointer.
//!
//! Every object belongs to the thread that made it, which is the GLib main
//! loop's; none of these types is `Send`, so the compiler keeps it there.

use std::ffi::{c_char, CStr, CString};
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::ptr::{self, NonNull};

use crate::ffi::*;

/// A GObject, held by one reference.
macro_rules! object {
    ($(#[$doc:meta])* $name:ident($raw:ty)) => {
        $(#[$doc])*
        pub struct $name {
            ptr: NonNull<$raw>,
            /// Pins the object to its thread: a raw pointer is neither
            /// `Send` nor `Sync`.
            _thread: PhantomData<*mut ()>,
        }

        #[allow(dead_code)]
        impl $name {
            /// Takes over a reference WebKit handed out (transfer full).
            unsafe fn owned(ptr: *mut $raw) -> Option<Self> {
                NonNull::new(ptr).map(|ptr| Self {
                    ptr,
                    _thread: PhantomData,
                })
            }

            /// Adds a reference to an object WebKit keeps (transfer none).
            unsafe fn borrowed(ptr: *mut $raw) -> Option<Self> {
                let object = unsafe { Self::owned(ptr) }?;
                unsafe { g_object_ref(ptr.cast()) };
                Some(object)
            }

            /// An object a signal is passing, for the handler's duration:
            /// no reference taken, none given back.
            unsafe fn passing(ptr: *mut $raw) -> ManuallyDrop<Self> {
                ManuallyDrop::new(unsafe { Self::owned(ptr) }.expect("a signal never passes NULL"))
            }

            fn as_ptr(&self) -> *mut $raw {
                self.ptr.as_ptr()
            }

            /// Connects `f` to `signal` through `trampoline`, a C function
            /// of the signal's signature that calls `f` from its data.
            fn connect<F: 'static>(&self, signal: &str, trampoline: *const (), f: F) {
                unsafe { connect(self.as_ptr().cast(), signal, trampoline, f) }
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                unsafe { Self::borrowed(self.as_ptr()) }.expect("a held object is not NULL")
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                unsafe { g_object_unref(self.as_ptr().cast()) }
            }
        }
    };
}

/// A reference-counted type of WPE's or WebKit's that is not a GObject.
macro_rules! counted {
    ($(#[$doc:meta])* $name:ident($raw:ty, $ref:ident, $unref:ident)) => {
        $(#[$doc])*
        pub struct $name {
            ptr: NonNull<$raw>,
            _thread: PhantomData<*mut ()>,
        }

        impl $name {
            unsafe fn owned(ptr: *mut $raw) -> Option<Self> {
                NonNull::new(ptr).map(|ptr| Self {
                    ptr,
                    _thread: PhantomData,
                })
            }

            fn as_ptr(&self) -> *mut $raw {
                self.ptr.as_ptr()
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                unsafe { Self::owned($ref(self.as_ptr())) }.expect("ref returns its argument")
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                unsafe { $unref(self.as_ptr()) }
            }
        }
    };
}

/// Connects `f` to `signal` on `instance`. `f` is boxed as the handler's
/// data and freed by GObject when the signal is disconnected, which is
/// when the object dies.
///
/// # Safety
/// `trampoline` must be an `unsafe extern "C"` function of the signal's
/// signature that reads its data as `*const F`, and `instance` a live
/// GObject.
unsafe fn connect<F: 'static>(instance: gpointer, signal: &str, trampoline: *const (), f: F) {
    unsafe extern "C" fn free<F>(data: gpointer, _: *mut GClosure) {
        drop(unsafe { Box::from_raw(data as *mut F) });
    }
    let signal = CString::new(signal).expect("signal names have no NUL");
    let data = Box::into_raw(Box::new(f)) as gpointer;
    // SAFETY: a plain function pointer as the GCallback GObject stores;
    // GObject calls it back with the signal's own signature.
    let handler: GCallback =
        Some(unsafe { std::mem::transmute::<*const (), unsafe extern "C" fn()>(trampoline) });
    unsafe { g_signal_connect_data(instance, signal.as_ptr(), handler, data, Some(free::<F>), 0) };
}

/// A borrowed C string as an owned Rust one; `None` for NULL.
unsafe fn string(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned())
    }
}

/// A Rust string as a C one, with any NUL cut off rather than failing: a
/// URL or title never legitimately has one.
fn cstring(s: &str) -> CString {
    let s = s.split('\0').next().unwrap_or_default();
    CString::new(s).expect("NUL removed")
}

fn cpath(path: &Path) -> CString {
    cstring(&path.to_string_lossy())
}

/// Takes a GError's message and frees it.
unsafe fn take_error(error: *mut GError) -> String {
    if error.is_null() {
        return "unknown error".into();
    }
    let message = unsafe { string((*error).message) }.unwrap_or_default();
    unsafe { g_error_free(error) };
    message
}

// GLib's TRUE, FALSE and G_PRIORITY_DEFAULT are macros bindgen leaves
// out; their values have been these since GLib 1.
const FALSE: gboolean = 0;
const G_PRIORITY_DEFAULT: std::ffi::c_int = 0;

fn gbool(b: bool) -> gboolean {
    b as gboolean
}

// GLib's main loop and its sources.

/// The thread's main loop; WebKit runs inside it.
pub struct MainLoop {
    ptr: NonNull<GMainLoop>,
    _thread: PhantomData<*mut ()>,
}

impl MainLoop {
    pub fn new() -> Self {
        let ptr = unsafe { g_main_loop_new(ptr::null_mut(), FALSE) };
        Self {
            ptr: NonNull::new(ptr).expect("GLib allocates or aborts"),
            _thread: PhantomData,
        }
    }

    pub fn run(&self) {
        unsafe { g_main_loop_run(self.ptr.as_ptr()) }
    }

    pub fn quit(&self) {
        unsafe { g_main_loop_quit(self.ptr.as_ptr()) }
    }
}

impl Clone for MainLoop {
    fn clone(&self) -> Self {
        Self {
            ptr: NonNull::new(unsafe { g_main_loop_ref(self.ptr.as_ptr()) })
                .expect("ref returns its argument"),
            _thread: PhantomData,
        }
    }
}

impl Drop for MainLoop {
    fn drop(&mut self) {
        unsafe { g_main_loop_unref(self.ptr.as_ptr()) }
    }
}

unsafe extern "C" fn source<F: FnMut() -> bool>(data: gpointer) -> gboolean {
    gbool(unsafe { (*(data as *mut F))() })
}

unsafe extern "C" fn free_source<F>(data: gpointer) {
    drop(unsafe { Box::from_raw(data as *mut F) });
}

/// Runs `f` on the main loop's thread as soon as it is idle, and again
/// while it returns `true`. Safe from any thread: this is how GLib takes
/// work from others.
pub fn idle_add<F: FnMut() -> bool + Send + 'static>(f: F) {
    let data = Box::into_raw(Box::new(f)) as gpointer;
    unsafe {
        g_idle_add_full(
            G_PRIORITY_DEFAULT,
            Some(source::<F>),
            data,
            Some(free_source::<F>),
        )
    };
}

/// Runs `f` every `millis` while it returns `true`.
pub fn timeout_add<F: FnMut() -> bool + 'static>(millis: u32, f: F) {
    let data = Box::into_raw(Box::new(f)) as gpointer;
    unsafe {
        g_timeout_add_full(
            G_PRIORITY_DEFAULT,
            millis,
            Some(source::<F>),
            data,
            Some(free_source::<F>),
        )
    };
}

// WPEPlatform.

object!(
    /// WPE's headless display: views render to buffers, not a screen.
    Display(WPEDisplay)
);

impl Display {
    /// A connected headless display, made the primary one.
    pub fn headless() -> Result<Self, String> {
        unsafe {
            let display = Self::owned(wpe_display_headless_new()).expect("WPE allocates or aborts");
            let mut error = ptr::null_mut();
            if wpe_display_connect(display.as_ptr(), &mut error) == FALSE {
                return Err(take_error(error));
            }
            wpe_display_set_primary(display.as_ptr());
            Ok(display)
        }
    }

    pub fn clipboard(&self) -> Clipboard {
        unsafe { Clipboard::borrowed(wpe_display_get_clipboard(self.as_ptr())) }
            .expect("a display has a clipboard")
    }

    /// The hardware keycode the keymap has for `keyval`, which pages see
    /// as `KeyboardEvent.code`; 0 when it has none.
    pub fn keycode(&self, keyval: u32) -> u32 {
        unsafe {
            let keymap = wpe_display_get_keymap(self.as_ptr());
            if keymap.is_null() {
                return 0;
            }
            let mut entries = ptr::null_mut();
            let mut n = 0;
            // A TRUE return fills `entries` with `n` of them.
            if wpe_keymap_get_entries_for_keyval(keymap, keyval, &mut entries, &mut n) == FALSE
                || n == 0
            {
                return 0;
            }
            // `as_ref` is `None` for a keymap that says TRUE and hands back
            // no array, so the read never touches a null pointer.
            let code = entries.as_ref().map_or(0, |entry| entry.keycode);
            g_free(entries.cast());
            code
        }
    }
}

object!(
    /// The display's clipboard, which pages copy to and paste from.
    Clipboard(WPEClipboard)
);

impl Clipboard {
    pub fn set_text(&self, text: &str) {
        unsafe {
            let content = wpe_clipboard_content_new();
            let text = cstring(text);
            wpe_clipboard_content_set_text(content, text.as_ptr());
            wpe_clipboard_set_content(self.as_ptr(), content);
            wpe_clipboard_content_unref(content);
        }
    }

    /// The text on it, when it holds any that is not a page's own.
    pub fn text(&self) -> Option<String> {
        unsafe {
            let content = wpe_clipboard_get_content(self.as_ptr());
            if content.is_null() {
                return None;
            }
            string(wpe_clipboard_content_get_text(content))
        }
    }

    /// `notify::change-count`: something was put on the clipboard.
    pub fn on_change<F: Fn(&Clipboard) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&Clipboard)>(
            clipboard: *mut WPEClipboard,
            _: *mut GParamSpec,
            data: gpointer,
        ) {
            unsafe { (*(data as *const F))(&Clipboard::passing(clipboard)) }
        }
        self.connect("notify::change-count", trampoline::<F> as *const (), f);
    }
}

object!(
    /// A WebKitWebView's rendering surface and input target.
    View(WPEView)
);

impl View {
    pub fn set_visible(&self, visible: bool) {
        unsafe { wpe_view_set_visible(self.as_ptr(), gbool(visible)) }
    }

    pub fn set_focused(&self, focused: bool) {
        unsafe {
            if focused {
                wpe_view_focus_in(self.as_ptr())
            } else {
                wpe_view_focus_out(self.as_ptr())
            }
        }
    }

    /// The toplevel the view is in, once WebKit has given it one.
    pub fn toplevel(&self) -> Option<Toplevel> {
        unsafe { Toplevel::borrowed(wpe_view_get_toplevel(self.as_ptr())) }
    }

    pub fn send(&self, event: &Event) {
        unsafe { wpe_view_event(self.as_ptr(), event.as_ptr()) }
    }

    /// How many clicks a press at `x`, `y` with `button` at `time` makes
    /// in a row: 1, 2 for a double click, and so on.
    pub fn press_count(&self, x: f64, y: f64, button: u32, time: u32) -> u32 {
        unsafe { wpe_view_compute_press_count(self.as_ptr(), x, y, button, time) }
    }

    /// `buffer-rendered`: a new frame.
    pub fn on_buffer_rendered<F: Fn(&View, &Buffer) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&View, &Buffer)>(
            view: *mut WPEView,
            buffer: *mut WPEBuffer,
            data: gpointer,
        ) {
            unsafe { (*(data as *const F))(&View::passing(view), &Buffer::passing(buffer)) }
        }
        self.connect("buffer-rendered", trampoline::<F> as *const (), f);
    }
}

object!(
    /// The headless toplevel a view is in: its size, scale and state.
    Toplevel(WPEToplevel)
);

impl Toplevel {
    pub fn resize(&self, width: i32, height: i32) {
        unsafe { wpe_toplevel_resize(self.as_ptr(), width, height) };
    }

    pub fn set_scale(&self, scale: f64) {
        unsafe { wpe_toplevel_scale_changed(self.as_ptr(), scale) }
    }

    /// Active (the keyboard's) or not.
    pub fn set_active(&self, active: bool) {
        let state = if active { WPE_TOPLEVEL_STATE_ACTIVE } else { 0 };
        unsafe { wpe_toplevel_state_changed(self.as_ptr(), state) }
    }
}

object!(
    /// A rendered frame.
    Buffer(WPEBuffer)
);

impl Buffer {
    pub fn width(&self) -> usize {
        unsafe { wpe_buffer_get_width(self.as_ptr()) }.max(0) as usize
    }

    pub fn height(&self) -> usize {
        unsafe { wpe_buffer_get_height(self.as_ptr()) }.max(0) as usize
    }

    /// The frame's pixels, ARGB8888 rows that may be padded; they belong
    /// to the buffer.
    pub fn pixels(&self) -> Result<&[u8], String> {
        unsafe {
            let mut error = ptr::null_mut();
            let bytes = wpe_buffer_import_to_pixels(self.as_ptr(), &mut error);
            if bytes.is_null() {
                return Err(take_error(error));
            }
            let mut len = 0;
            let data = g_bytes_get_data(bytes, &mut len) as *const u8;
            if data.is_null() {
                return Ok(&[]);
            }
            Ok(std::slice::from_raw_parts(data, len as usize))
        }
    }
}

counted!(
    /// An input event for a view.
    Event(WPEEvent, wpe_event_ref, wpe_event_unref)
);

impl Event {
    fn new(ptr: *mut WPEEvent) -> Self {
        unsafe { Self::owned(ptr) }.expect("WPE allocates or aborts")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer_move(
        kind: WPEEventType,
        view: &View,
        source: WPEInputSource,
        time: u32,
        modifiers: WPEModifiers,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    ) -> Self {
        Self::new(unsafe {
            wpe_event_pointer_move_new(kind, view.as_ptr(), source, time, modifiers, x, y, dx, dy)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer_button(
        kind: WPEEventType,
        view: &View,
        source: WPEInputSource,
        time: u32,
        modifiers: WPEModifiers,
        button: u32,
        x: f64,
        y: f64,
        press_count: u32,
    ) -> Self {
        Self::new(unsafe {
            wpe_event_pointer_button_new(
                kind,
                view.as_ptr(),
                source,
                time,
                modifiers,
                button,
                x,
                y,
                press_count,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scroll(
        view: &View,
        source: WPEInputSource,
        time: u32,
        modifiers: WPEModifiers,
        dx: f64,
        dy: f64,
        precise: bool,
        x: f64,
        y: f64,
    ) -> Self {
        Self::new(unsafe {
            wpe_event_scroll_new(
                view.as_ptr(),
                source,
                time,
                modifiers,
                dx,
                dy,
                gbool(precise),
                FALSE,
                x,
                y,
            )
        })
    }

    pub fn keyboard(
        kind: WPEEventType,
        view: &View,
        time: u32,
        modifiers: WPEModifiers,
        keycode: u32,
        keyval: u32,
    ) -> Self {
        Self::new(unsafe {
            wpe_event_keyboard_new(
                kind,
                view.as_ptr(),
                WPE_INPUT_SOURCE_KEYBOARD,
                time,
                modifiers,
                keycode,
                keyval,
            )
        })
    }

    pub fn touch(
        kind: WPEEventType,
        view: &View,
        time: u32,
        modifiers: WPEModifiers,
        sequence: u32,
        x: f64,
        y: f64,
    ) -> Self {
        Self::new(unsafe {
            wpe_event_touch_new(
                kind,
                view.as_ptr(),
                WPE_INPUT_SOURCE_TOUCHSCREEN,
                time,
                modifiers,
                sequence,
                x,
                y,
            )
        })
    }
}

// WPE WebKit.

object!(
    /// Cookies, cache and credentials, shared by every view opened on it.
    NetworkSession(WebKitNetworkSession)
);

impl NetworkSession {
    /// A session whose data outlives the process, in `data` and `cache`.
    pub fn persistent(data: &Path, cache: &Path) -> Self {
        let (data, cache) = (cpath(data), cpath(cache));
        unsafe { Self::owned(webkit_network_session_new(data.as_ptr(), cache.as_ptr())) }
            .expect("WebKit allocates or aborts")
    }

    /// A session that forgets everything with the process.
    pub fn ephemeral() -> Self {
        unsafe { Self::owned(webkit_network_session_new_ephemeral()) }
            .expect("WebKit allocates or aborts")
    }

    /// Intelligent Tracking Prevention: third-party cookies and storage
    /// cut off for sites that only ever appear as trackers.
    pub fn set_itp_enabled(&self, enabled: bool) {
        unsafe { webkit_network_session_set_itp_enabled(self.as_ptr(), gbool(enabled)) }
    }

    pub fn set_persistent_credential_storage_enabled(&self, enabled: bool) {
        unsafe {
            webkit_network_session_set_persistent_credential_storage_enabled(
                self.as_ptr(),
                gbool(enabled),
            )
        }
    }

    /// `download-started`.
    pub fn on_download_started<F: Fn(&NetworkSession, &Download) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&NetworkSession, &Download)>(
            session: *mut WebKitNetworkSession,
            download: *mut WebKitDownload,
            data: gpointer,
        ) {
            unsafe {
                (*(data as *const F))(
                    &NetworkSession::passing(session),
                    &Download::passing(download),
                )
            }
        }
        self.connect("download-started", trampoline::<F> as *const (), f);
    }
}

object!(
    /// What pages may do, shared by every view.
    Settings(WebKitSettings)
);

impl Settings {
    pub fn new() -> Self {
        unsafe { Self::owned(webkit_settings_new()) }.expect("WebKit allocates or aborts")
    }

    pub fn set_enable_developer_extras(&self, enabled: bool) {
        unsafe { webkit_settings_set_enable_developer_extras(self.as_ptr(), gbool(enabled)) }
    }

    pub fn set_javascript_can_open_windows_automatically(&self, enabled: bool) {
        unsafe {
            webkit_settings_set_javascript_can_open_windows_automatically(
                self.as_ptr(),
                gbool(enabled),
            )
        }
    }

    pub fn set_allow_file_access_from_file_urls(&self, allowed: bool) {
        unsafe {
            webkit_settings_set_allow_file_access_from_file_urls(self.as_ptr(), gbool(allowed))
        }
    }

    pub fn set_allow_universal_access_from_file_urls(&self, allowed: bool) {
        unsafe {
            webkit_settings_set_allow_universal_access_from_file_urls(self.as_ptr(), gbool(allowed))
        }
    }

    pub fn set_allow_top_navigation_to_data_urls(&self, allowed: bool) {
        unsafe {
            webkit_settings_set_allow_top_navigation_to_data_urls(self.as_ptr(), gbool(allowed))
        }
    }

    /// Names the application in WebKit's own User-Agent string.
    pub fn set_application(&self, name: &str, version: &str) {
        let (name, version) = (cstring(name), cstring(version));
        unsafe {
            webkit_settings_set_user_agent_with_application_details(
                self.as_ptr(),
                name.as_ptr(),
                version.as_ptr(),
            )
        }
    }

    pub fn user_agent(&self) -> String {
        unsafe { string(webkit_settings_get_user_agent(self.as_ptr())) }.unwrap_or_default()
    }

    pub fn set_user_agent(&self, agent: &str) {
        let agent = cstring(agent);
        unsafe { webkit_settings_set_user_agent(self.as_ptr(), agent.as_ptr()) }
    }
}

object!(
    /// The scripts, style sheets and content filters injected into every
    /// view that shares it.
    UserContentManager(WebKitUserContentManager)
);

impl UserContentManager {
    pub fn new() -> Self {
        unsafe { Self::owned(webkit_user_content_manager_new()) }
            .expect("WebKit allocates or aborts")
    }

    pub fn add_filter(&self, filter: &Filter) {
        unsafe { webkit_user_content_manager_add_filter(self.as_ptr(), filter.as_ptr()) }
    }

    pub fn remove_all_filters(&self) {
        unsafe { webkit_user_content_manager_remove_all_filters(self.as_ptr()) }
    }
}

counted!(
    /// A compiled content blocker rule set.
    Filter(
        WebKitUserContentFilter,
        webkit_user_content_filter_ref,
        webkit_user_content_filter_unref
    )
);

object!(
    /// Where WebKit keeps the rule sets it compiled, by identifier.
    FilterStore(WebKitUserContentFilterStore)
);

/// Runs a boxed `FnOnce` as a GIO async callback; `finish` turns the
/// result into what the closure takes.
unsafe extern "C" fn ready<T, F: FnOnce(T)>(
    source: *mut GObject,
    result: *mut GAsyncResult,
    data: gpointer,
) where
    T: Finish,
{
    let f = unsafe { Box::from_raw(data as *mut F) };
    f(unsafe { T::finish(source, result) })
}

/// `f` boxed as an async call's callback and data.
fn boxed<T: Finish, F: FnOnce(T) + 'static>(f: F) -> (GAsyncReadyCallback, gpointer) {
    (Some(ready::<T, F>), Box::into_raw(Box::new(f)) as gpointer)
}

/// What an async call hands back, read from its `_finish` function.
trait Finish {
    unsafe fn finish(source: *mut GObject, result: *mut GAsyncResult) -> Self;
}

/// A rule set loaded or compiled, or why not.
pub type Loaded = Result<Filter, String>;

struct LoadFinish(Loaded);
struct SaveFinish(Loaded);
struct IdentifiersFinish(Vec<String>);

impl Finish for LoadFinish {
    unsafe fn finish(source: *mut GObject, result: *mut GAsyncResult) -> Self {
        let mut error = ptr::null_mut();
        let filter = unsafe {
            webkit_user_content_filter_store_load_finish(source.cast(), result, &mut error)
        };
        Self(match unsafe { Filter::owned(filter) } {
            Some(filter) => Ok(filter),
            None => Err(unsafe { take_error(error) }),
        })
    }
}

impl Finish for SaveFinish {
    unsafe fn finish(source: *mut GObject, result: *mut GAsyncResult) -> Self {
        let mut error = ptr::null_mut();
        let filter = unsafe {
            webkit_user_content_filter_store_save_from_file_finish(
                source.cast(),
                result,
                &mut error,
            )
        };
        Self(match unsafe { Filter::owned(filter) } {
            Some(filter) => Ok(filter),
            None => Err(unsafe { take_error(error) }),
        })
    }
}

impl Finish for IdentifiersFinish {
    unsafe fn finish(source: *mut GObject, result: *mut GAsyncResult) -> Self {
        let list = unsafe {
            webkit_user_content_filter_store_fetch_identifiers_finish(source.cast(), result)
        };
        let mut identifiers = Vec::new();
        if !list.is_null() {
            let mut at = list;
            unsafe {
                while !(*at).is_null() {
                    identifiers.extend(string(*at));
                    at = at.add(1);
                }
                g_strfreev(list);
            }
        }
        Self(identifiers)
    }
}

impl FilterStore {
    pub fn new(path: &Path) -> Self {
        let path = cpath(path);
        unsafe { Self::owned(webkit_user_content_filter_store_new(path.as_ptr())) }
            .expect("WebKit allocates or aborts")
    }

    /// Loads the set compiled under `identifier`; `done` gets an error
    /// when there is none.
    pub fn load<F: FnOnce(Loaded) + 'static>(&self, identifier: &str, done: F) {
        let identifier = cstring(identifier);
        let (callback, data) = boxed(move |LoadFinish(loaded)| done(loaded));
        unsafe {
            webkit_user_content_filter_store_load(
                self.as_ptr(),
                identifier.as_ptr(),
                ptr::null_mut(),
                callback,
                data,
            )
        }
    }

    /// Compiles the JSON rules in `file` and keeps the result under
    /// `identifier`.
    pub fn save_from_file<F: FnOnce(Loaded) + 'static>(
        &self,
        identifier: &str,
        file: &Path,
        done: F,
    ) {
        let identifier = cstring(identifier);
        let path = cpath(file);
        let (callback, data) = boxed(move |SaveFinish(loaded)| done(loaded));
        unsafe {
            let file = g_file_new_for_path(path.as_ptr());
            webkit_user_content_filter_store_save_from_file(
                self.as_ptr(),
                identifier.as_ptr(),
                file,
                ptr::null_mut(),
                callback,
                data,
            );
            g_object_unref(file.cast());
        }
    }

    /// Lists every identifier the store holds.
    pub fn fetch_identifiers<F: FnOnce(Vec<String>) + 'static>(&self, done: F) {
        let (callback, data) = boxed(move |IdentifiersFinish(ids)| done(ids));
        unsafe {
            webkit_user_content_filter_store_fetch_identifiers(
                self.as_ptr(),
                ptr::null_mut(),
                callback,
                data,
            )
        }
    }

    /// Deletes the set under `identifier`, without waiting for it.
    pub fn remove(&self, identifier: &str) {
        let identifier = cstring(identifier);
        unsafe {
            webkit_user_content_filter_store_remove(
                self.as_ptr(),
                identifier.as_ptr(),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            )
        }
    }
}

object!(
    /// One page, with its own web process.
    WebView(WebKitWebView)
);

impl WebView {
    /// A view on `display`, with `session`'s cookies, `settings` and
    /// `content`'s filters.
    pub fn new(
        display: &Display,
        session: &NetworkSession,
        settings: &Settings,
        content: &UserContentManager,
    ) -> Self {
        let (p_display, p_session, p_settings, p_content) = (
            c"display",
            c"network-session",
            c"settings",
            c"user-content-manager",
        );
        let view = unsafe {
            g_object_new(
                webkit_web_view_get_type(),
                p_display.as_ptr(),
                display.as_ptr(),
                p_session.as_ptr(),
                session.as_ptr(),
                p_settings.as_ptr(),
                settings.as_ptr(),
                p_content.as_ptr(),
                content.as_ptr(),
                ptr::null::<c_char>(),
            )
        };
        unsafe { Self::owned(view.cast()) }.expect("WebKit allocates or aborts")
    }

    /// A popup's view, which shares `related`'s process, display and
    /// session, and loads by itself.
    pub fn related(related: &WebView, settings: &Settings, content: &UserContentManager) -> Self {
        let (p_related, p_settings, p_content) =
            (c"related-view", c"settings", c"user-content-manager");
        let view = unsafe {
            g_object_new(
                webkit_web_view_get_type(),
                p_related.as_ptr(),
                related.as_ptr(),
                p_settings.as_ptr(),
                settings.as_ptr(),
                p_content.as_ptr(),
                content.as_ptr(),
                ptr::null::<c_char>(),
            )
        };
        unsafe { Self::owned(view.cast()) }.expect("WebKit allocates or aborts")
    }

    pub fn view(&self) -> View {
        unsafe { View::borrowed(webkit_web_view_get_wpe_view(self.as_ptr())) }
            .expect("a web view has a WPE view")
    }

    pub fn load_uri(&self, uri: &str) {
        let uri = cstring(uri);
        unsafe { webkit_web_view_load_uri(self.as_ptr(), uri.as_ptr()) }
    }

    pub fn go_back(&self) {
        unsafe { webkit_web_view_go_back(self.as_ptr()) }
    }

    pub fn go_forward(&self) {
        unsafe { webkit_web_view_go_forward(self.as_ptr()) }
    }

    pub fn reload(&self) {
        unsafe { webkit_web_view_reload(self.as_ptr()) }
    }

    pub fn stop_loading(&self) {
        unsafe { webkit_web_view_stop_loading(self.as_ptr()) }
    }

    pub fn uri(&self) -> String {
        unsafe { string(webkit_web_view_get_uri(self.as_ptr())) }.unwrap_or_default()
    }

    pub fn title(&self) -> String {
        unsafe { string(webkit_web_view_get_title(self.as_ptr())) }.unwrap_or_default()
    }

    /// 0 to 1 while loading.
    pub fn estimated_load_progress(&self) -> f64 {
        unsafe { webkit_web_view_get_estimated_load_progress(self.as_ptr()) }
    }

    pub fn is_loading(&self) -> bool {
        unsafe { webkit_web_view_is_loading(self.as_ptr()) != FALSE }
    }

    pub fn can_go_back(&self) -> bool {
        unsafe { webkit_web_view_can_go_back(self.as_ptr()) != FALSE }
    }

    pub fn can_go_forward(&self) -> bool {
        unsafe { webkit_web_view_can_go_forward(self.as_ptr()) != FALSE }
    }

    /// Loaded over https with a certificate that checked out.
    pub fn is_secure(&self) -> bool {
        // WebKit reports the last connection, not the page's address: a
        // page that moved from https to http keeps its info.
        if !self.uri().starts_with("https://") {
            return false;
        }
        let mut certificate = ptr::null_mut();
        let mut errors: GTlsCertificateFlags = 0;
        let https =
            unsafe { webkit_web_view_get_tls_info(self.as_ptr(), &mut certificate, &mut errors) };
        https != FALSE && errors == 0
    }

    /// `notify::<property>`: `property` changed.
    pub fn on_notify<F: Fn(&WebView) + 'static>(&self, property: &str, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&WebView)>(
            view: *mut WebKitWebView,
            _: *mut GParamSpec,
            data: gpointer,
        ) {
            unsafe { (*(data as *const F))(&WebView::passing(view)) }
        }
        self.connect(
            &format!("notify::{property}"),
            trampoline::<F> as *const (),
            f,
        );
    }

    /// `load-changed`, with one of `WEBKIT_LOAD_*`.
    pub fn on_load_changed<F: Fn(&WebView, WebKitLoadEvent) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&WebView, WebKitLoadEvent)>(
            view: *mut WebKitWebView,
            event: WebKitLoadEvent,
            data: gpointer,
        ) {
            unsafe { (*(data as *const F))(&WebView::passing(view), event) }
        }
        self.connect("load-changed", trampoline::<F> as *const (), f);
    }

    /// `create`: a page wants a popup. `f` returns the view to open it in,
    /// made with [`WebView::related`], or `None` to block it. WebKit takes
    /// a reference of its own to the view returned.
    pub fn on_create<F: Fn(&WebView) -> Option<WebView> + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&WebView) -> Option<WebView>>(
            view: *mut WebKitWebView,
            _: *mut WebKitNavigationAction,
            data: gpointer,
        ) -> *mut WebKitWebView {
            match unsafe { (*(data as *const F))(&WebView::passing(view)) } {
                // The signal's return is transfer full: the reference the
                // closure's WebView held goes to WebKit instead of back.
                Some(new) => ManuallyDrop::new(new).as_ptr(),
                None => ptr::null_mut(),
            }
        }
        self.connect("create", trampoline::<F> as *const (), f);
    }

    /// `ready-to-show`: a popup has its first page and may be shown.
    pub fn on_ready_to_show<F: Fn(&WebView) + 'static>(&self, f: F) {
        self.connect("ready-to-show", plain::<F> as *const (), f);
    }

    /// `close`: the page asked to be closed.
    pub fn on_close<F: Fn(&WebView) + 'static>(&self, f: F) {
        self.connect("close", plain::<F> as *const (), f);
    }

    /// `web-process-terminated`: the page's process died.
    pub fn on_web_process_terminated<F: Fn(&WebView) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&WebView)>(
            view: *mut WebKitWebView,
            _reason: WebKitWebProcessTerminationReason,
            data: gpointer,
        ) {
            unsafe { (*(data as *const F))(&WebView::passing(view)) }
        }
        self.connect("web-process-terminated", trampoline::<F> as *const (), f);
    }

    /// `decide-policy`, with one of `WEBKIT_POLICY_DECISION_TYPE_*`; `f`
    /// returns `true` once it has decided, `false` to let WebKit.
    pub fn on_decide_policy<F>(&self, f: F)
    where
        F: Fn(&WebView, &PolicyDecision, WebKitPolicyDecisionType) -> bool + 'static,
    {
        unsafe extern "C" fn trampoline<
            F: Fn(&WebView, &PolicyDecision, WebKitPolicyDecisionType) -> bool,
        >(
            view: *mut WebKitWebView,
            decision: *mut WebKitPolicyDecision,
            kind: WebKitPolicyDecisionType,
            data: gpointer,
        ) -> gboolean {
            gbool(unsafe {
                (*(data as *const F))(
                    &WebView::passing(view),
                    &PolicyDecision::passing(decision),
                    kind,
                )
            })
        }
        self.connect("decide-policy", trampoline::<F> as *const (), f);
    }

    /// `permission-request`. The request is handed over for keeping, to be
    /// answered now or once the person has; `f` returns `true` when it
    /// will be answered, `false` to let WebKit deny it.
    pub fn on_permission_request<F: Fn(&WebView, PermissionRequest) -> bool + 'static>(
        &self,
        f: F,
    ) {
        unsafe extern "C" fn trampoline<F: Fn(&WebView, PermissionRequest) -> bool>(
            view: *mut WebKitWebView,
            request: *mut WebKitPermissionRequest,
            data: gpointer,
        ) -> gboolean {
            let request = unsafe { PermissionRequest::borrowed(request) }
                .expect("a signal never passes NULL");
            gbool(unsafe { (*(data as *const F))(&WebView::passing(view), request) })
        }
        self.connect("permission-request", trampoline::<F> as *const (), f);
    }

    /// `mouse-target-changed`: the pointer is over something else.
    pub fn on_mouse_target_changed<F: Fn(&WebView, &HitTestResult) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&WebView, &HitTestResult)>(
            view: *mut WebKitWebView,
            result: *mut WebKitHitTestResult,
            _modifiers: guint,
            data: gpointer,
        ) {
            unsafe {
                (*(data as *const F))(&WebView::passing(view), &HitTestResult::passing(result))
            }
        }
        self.connect("mouse-target-changed", trampoline::<F> as *const (), f);
    }
}

/// The trampoline for a web view signal with no arguments of its own.
unsafe extern "C" fn plain<F: Fn(&WebView)>(view: *mut WebKitWebView, data: gpointer) {
    unsafe { (*(data as *const F))(&WebView::passing(view)) }
}

object!(
    /// A navigation or response WebKit asks about before going on.
    PolicyDecision(WebKitPolicyDecision)
);

impl PolicyDecision {
    /// For a response decision (the signal's type says which): whether
    /// WebKit can show the content type.
    pub fn is_mime_type_supported(&self) -> bool {
        unsafe {
            webkit_response_policy_decision_is_mime_type_supported(self.as_ptr().cast()) != FALSE
        }
    }

    /// For a navigation decision: what started it.
    fn action(&self) -> *mut WebKitNavigationAction {
        unsafe { webkit_navigation_policy_decision_get_navigation_action(self.as_ptr().cast()) }
    }

    /// For a navigation decision: the mouse button that started it, 0 for
    /// none.
    pub fn mouse_button(&self) -> u32 {
        let action = self.action();
        if action.is_null() {
            0
        } else {
            unsafe { webkit_navigation_action_get_mouse_button(action) }
        }
    }

    /// For a navigation decision: where it goes.
    pub fn uri(&self) -> Option<String> {
        let action = self.action();
        if action.is_null() {
            return None;
        }
        unsafe {
            string(webkit_uri_request_get_uri(
                webkit_navigation_action_get_request(action),
            ))
        }
    }

    pub fn download(&self) {
        unsafe { webkit_policy_decision_download(self.as_ptr()) }
    }

    pub fn ignore(&self) {
        unsafe { webkit_policy_decision_ignore(self.as_ptr()) }
    }
}

object!(
    /// A page asking for something only the person may grant.
    PermissionRequest(WebKitPermissionRequest)
);

impl PermissionRequest {
    /// The request's GObject class, which names what it asks for
    /// (`permissions::Kind::from_type_name`).
    pub fn type_name(&self) -> String {
        unsafe { string(g_type_name_from_instance(self.as_ptr().cast())) }.unwrap_or_default()
    }

    /// For a user media request (`type_name` says so): whether it is for
    /// audio, video, and the screen.
    pub fn user_media(&self) -> (bool, bool, bool) {
        unsafe {
            let p = self.as_ptr().cast();
            (
                webkit_user_media_permission_is_for_audio_device(p) != FALSE,
                webkit_user_media_permission_is_for_video_device(p) != FALSE,
                webkit_user_media_permission_is_for_display_device(p) != FALSE,
            )
        }
    }

    pub fn allow(&self) {
        unsafe { webkit_permission_request_allow(self.as_ptr()) }
    }

    pub fn deny(&self) {
        unsafe { webkit_permission_request_deny(self.as_ptr()) }
    }
}

object!(
    /// What is under the pointer.
    HitTestResult(WebKitHitTestResult)
);

impl HitTestResult {
    /// The link's address when the pointer is over one.
    pub fn link_uri(&self) -> Option<String> {
        unsafe {
            if webkit_hit_test_result_context_is_link(self.as_ptr()) == FALSE {
                return None;
            }
            string(webkit_hit_test_result_get_link_uri(self.as_ptr()))
        }
    }
}

object!(
    /// A file being saved.
    Download(WebKitDownload)
);

impl Download {
    pub fn set_destination(&self, path: &Path) {
        let path = cpath(path);
        unsafe { webkit_download_set_destination(self.as_ptr(), path.as_ptr()) }
    }

    pub fn destination(&self) -> Option<String> {
        unsafe { string(webkit_download_get_destination(self.as_ptr())) }
    }

    /// `decide-destination`, with the name the server suggested; `f`
    /// returns `true` once it has set one.
    pub fn on_decide_destination<F: Fn(&Download, &str) -> bool + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&Download, &str) -> bool>(
            download: *mut WebKitDownload,
            suggested: *const c_char,
            data: gpointer,
        ) -> gboolean {
            let suggested = unsafe { string(suggested) }.unwrap_or_default();
            gbool(unsafe { (*(data as *const F))(&Download::passing(download), &suggested) })
        }
        self.connect("decide-destination", trampoline::<F> as *const (), f);
    }

    /// `finished`.
    pub fn on_finished<F: Fn(&Download) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&Download)>(
            download: *mut WebKitDownload,
            data: gpointer,
        ) {
            unsafe { (*(data as *const F))(&Download::passing(download)) }
        }
        self.connect("finished", trampoline::<F> as *const (), f);
    }

    /// `failed`, with the error's message.
    pub fn on_failed<F: Fn(&Download, String) + 'static>(&self, f: F) {
        unsafe extern "C" fn trampoline<F: Fn(&Download, String)>(
            download: *mut WebKitDownload,
            error: *mut GError,
            data: gpointer,
        ) {
            // The error belongs to the signal; read it, do not free it.
            let message = unsafe {
                if error.is_null() {
                    None
                } else {
                    string((*error).message)
                }
            };
            unsafe {
                (*(data as *const F))(&Download::passing(download), message.unwrap_or_default())
            }
        }
        self.connect("failed", trampoline::<F> as *const (), f);
    }
}
