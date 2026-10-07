//! The C API Danube calls: GLib's main loop and objects, WPE WebKit's
//! WebKitWebView and friends, and WPEPlatform's view, toplevel, events and
//! clipboard. Written out by hand, as the few dozen functions used, rather
//! than through gtk-rs: there are no Rust bindings for WPE's 2.0 API, and
//! every object here only ever lives on the GLib thread (engine.rs).
//!
//! Every type is opaque and handled through a pointer.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_double, c_int, c_uint, c_ulong, c_void, CStr, CString};

pub type gboolean = c_int;
pub type gpointer = *mut c_void;
pub type GType = usize;
pub type GCallback = unsafe extern "C" fn();
pub type GSourceFunc = unsafe extern "C" fn(gpointer) -> gboolean;
pub type GDestroyNotify = unsafe extern "C" fn(gpointer);
pub type GAsyncReadyCallback = unsafe extern "C" fn(*mut GObject, *mut GAsyncResult, gpointer);

macro_rules! opaque {
    ($($name:ident),* $(,)?) => {
        $(#[repr(C)] pub struct $name { _private: [u8; 0] })*
    };
}

opaque!(
    GObject,
    GMainLoop,
    GBytes,
    GAsyncResult,
    GFile,
    GTlsCertificate,
    GParamSpec,
    WPEDisplay,
    WPEView,
    WPEToplevel,
    WPEBuffer,
    WPEEvent,
    WPEClipboard,
    WPEClipboardContent,
    WPEKeymap,
    WebKitWebView,
    WebKitSettings,
    WebKitNetworkSession,
    WebKitUserContentManager,
    WebKitUserContentFilterStore,
    WebKitUserContentFilter,
    WebKitPolicyDecision,
    WebKitNavigationAction,
    WebKitURIRequest,
    WebKitURIResponse,
    WebKitDownload,
    WebKitPermissionRequest,
    WebKitHitTestResult,
);

#[repr(C)]
pub struct GError {
    pub domain: u32,
    pub code: c_int,
    pub message: *mut c_char,
}

#[repr(C)]
pub struct WPEKeymapEntry {
    pub keycode: c_uint,
    pub group: c_int,
    pub level: c_int,
}

pub const FALSE: gboolean = 0;
pub const TRUE: gboolean = 1;
pub const G_PRIORITY_DEFAULT: c_int = 0;

// WPEEventType
pub const WPE_EVENT_POINTER_DOWN: c_int = 1;
pub const WPE_EVENT_POINTER_UP: c_int = 2;
pub const WPE_EVENT_POINTER_MOVE: c_int = 3;
pub const WPE_EVENT_POINTER_ENTER: c_int = 4;
pub const WPE_EVENT_POINTER_LEAVE: c_int = 5;
pub const WPE_EVENT_KEYBOARD_KEY_DOWN: c_int = 7;
pub const WPE_EVENT_KEYBOARD_KEY_UP: c_int = 8;
pub const WPE_EVENT_TOUCH_DOWN: c_int = 9;
pub const WPE_EVENT_TOUCH_UP: c_int = 10;
pub const WPE_EVENT_TOUCH_MOVE: c_int = 11;
pub const WPE_EVENT_TOUCH_CANCEL: c_int = 12;

// WPEInputSource
pub const WPE_INPUT_SOURCE_MOUSE: c_int = 0;
pub const WPE_INPUT_SOURCE_KEYBOARD: c_int = 2;
pub const WPE_INPUT_SOURCE_TOUCHSCREEN: c_int = 3;
pub const WPE_INPUT_SOURCE_TOUCHPAD: c_int = 4;

// WPEModifiers
pub const WPE_MODIFIER_KEYBOARD_CONTROL: c_uint = 1 << 0;
pub const WPE_MODIFIER_KEYBOARD_SHIFT: c_uint = 1 << 1;
pub const WPE_MODIFIER_KEYBOARD_ALT: c_uint = 1 << 2;
pub const WPE_MODIFIER_KEYBOARD_META: c_uint = 1 << 3;
pub const WPE_MODIFIER_POINTER_BUTTON1: c_uint = 1 << 8;
pub const WPE_MODIFIER_POINTER_BUTTON2: c_uint = 1 << 9;
pub const WPE_MODIFIER_POINTER_BUTTON3: c_uint = 1 << 10;

// WPEToplevelState
pub const WPE_TOPLEVEL_STATE_ACTIVE: c_uint = 1 << 2;

// WebKitPolicyDecisionType
pub const WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION: c_int = 0;
pub const WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION: c_int = 1;
pub const WEBKIT_POLICY_DECISION_TYPE_RESPONSE: c_int = 2;
// WebKitLoadEvent
pub const WEBKIT_LOAD_STARTED: c_int = 0;
pub const WEBKIT_LOAD_COMMITTED: c_int = 2;
pub const WEBKIT_LOAD_FINISHED: c_int = 3;

unsafe extern "C" {
    // GLib and GObject
    pub fn g_main_loop_new(context: gpointer, is_running: gboolean) -> *mut GMainLoop;
    pub fn g_main_loop_run(main_loop: *mut GMainLoop);
    pub fn g_main_loop_quit(main_loop: *mut GMainLoop);
    pub fn g_idle_add_full(
        priority: c_int,
        function: GSourceFunc,
        data: gpointer,
        notify: Option<GDestroyNotify>,
    ) -> c_uint;
    pub fn g_timeout_add_full(
        priority: c_int,
        interval: c_uint,
        function: GSourceFunc,
        data: gpointer,
        notify: Option<GDestroyNotify>,
    ) -> c_uint;
    pub fn g_free(mem: gpointer);
    pub fn g_strfreev(strv: *mut *mut c_char);
    pub fn g_error_free(error: *mut GError);
    pub fn g_bytes_get_data(bytes: *mut GBytes, size: *mut usize) -> *const c_void;
    pub fn g_object_new(object_type: GType, first_property_name: *const c_char, ...) -> gpointer;
    pub fn g_object_ref(object: gpointer) -> gpointer;
    pub fn g_object_unref(object: gpointer);
    pub fn g_signal_connect_data(
        instance: gpointer,
        detailed_signal: *const c_char,
        c_handler: GCallback,
        data: gpointer,
        destroy_data: Option<GDestroyNotify>,
        connect_flags: c_int,
    ) -> c_ulong;
    pub fn g_signal_handlers_disconnect_matched(
        instance: gpointer,
        mask: c_int,
        signal_id: c_uint,
        detail: u32,
        closure: gpointer,
        func: gpointer,
        data: gpointer,
    ) -> c_uint;
    pub fn g_type_name_from_instance(instance: gpointer) -> *const c_char;
    pub fn g_file_new_for_path(path: *const c_char) -> *mut GFile;

    // WPEPlatform
    pub fn wpe_display_headless_new() -> *mut WPEDisplay;
    pub fn wpe_display_connect(display: *mut WPEDisplay, error: *mut *mut GError) -> gboolean;
    pub fn wpe_display_set_primary(display: *mut WPEDisplay);
    pub fn wpe_display_get_clipboard(display: *mut WPEDisplay) -> *mut WPEClipboard;
    pub fn wpe_display_get_keymap(display: *mut WPEDisplay) -> *mut WPEKeymap;
    pub fn wpe_keymap_get_entries_for_keyval(
        keymap: *mut WPEKeymap,
        keyval: c_uint,
        entries: *mut *mut WPEKeymapEntry,
        n_entries: *mut c_uint,
    ) -> gboolean;
    pub fn wpe_view_get_toplevel(view: *mut WPEView) -> *mut WPEToplevel;
    pub fn wpe_view_set_visible(view: *mut WPEView, visible: gboolean);
    pub fn wpe_view_event(view: *mut WPEView, event: *mut WPEEvent);
    pub fn wpe_view_focus_in(view: *mut WPEView);
    pub fn wpe_view_focus_out(view: *mut WPEView);
    pub fn wpe_view_compute_press_count(
        view: *mut WPEView,
        x: c_double,
        y: c_double,
        button: c_uint,
        time: u32,
    ) -> c_uint;
    pub fn wpe_toplevel_resize(toplevel: *mut WPEToplevel, width: c_int, height: c_int)
        -> gboolean;
    pub fn wpe_toplevel_scale_changed(toplevel: *mut WPEToplevel, scale: c_double);
    pub fn wpe_toplevel_state_changed(toplevel: *mut WPEToplevel, state: c_uint);
    pub fn wpe_buffer_get_width(buffer: *mut WPEBuffer) -> c_int;
    pub fn wpe_buffer_get_height(buffer: *mut WPEBuffer) -> c_int;
    pub fn wpe_buffer_import_to_pixels(
        buffer: *mut WPEBuffer,
        error: *mut *mut GError,
    ) -> *mut GBytes;
    pub fn wpe_event_unref(event: *mut WPEEvent);
    pub fn wpe_event_pointer_button_new(
        kind: c_int,
        view: *mut WPEView,
        source: c_int,
        time: u32,
        modifiers: c_uint,
        button: c_uint,
        x: c_double,
        y: c_double,
        press_count: c_uint,
    ) -> *mut WPEEvent;
    pub fn wpe_event_pointer_move_new(
        kind: c_int,
        view: *mut WPEView,
        source: c_int,
        time: u32,
        modifiers: c_uint,
        x: c_double,
        y: c_double,
        delta_x: c_double,
        delta_y: c_double,
    ) -> *mut WPEEvent;
    pub fn wpe_event_scroll_new(
        view: *mut WPEView,
        source: c_int,
        time: u32,
        modifiers: c_uint,
        delta_x: c_double,
        delta_y: c_double,
        precise_deltas: gboolean,
        is_stop: gboolean,
        x: c_double,
        y: c_double,
    ) -> *mut WPEEvent;
    pub fn wpe_event_keyboard_new(
        kind: c_int,
        view: *mut WPEView,
        source: c_int,
        time: u32,
        modifiers: c_uint,
        keycode: c_uint,
        keyval: c_uint,
    ) -> *mut WPEEvent;
    pub fn wpe_event_touch_new(
        kind: c_int,
        view: *mut WPEView,
        source: c_int,
        time: u32,
        modifiers: c_uint,
        sequence_id: u32,
        x: c_double,
        y: c_double,
    ) -> *mut WPEEvent;
    pub fn wpe_clipboard_set_content(
        clipboard: *mut WPEClipboard,
        content: *mut WPEClipboardContent,
    );
    pub fn wpe_clipboard_get_content(clipboard: *mut WPEClipboard) -> *mut WPEClipboardContent;
    pub fn wpe_clipboard_content_new() -> *mut WPEClipboardContent;
    pub fn wpe_clipboard_content_unref(content: *mut WPEClipboardContent);
    pub fn wpe_clipboard_content_set_text(content: *mut WPEClipboardContent, text: *const c_char);
    pub fn wpe_clipboard_content_get_text(content: *mut WPEClipboardContent) -> *const c_char;

    // WPE WebKit
    pub fn webkit_web_view_get_type() -> GType;
    pub fn webkit_web_view_get_wpe_view(view: *mut WebKitWebView) -> *mut WPEView;
    pub fn webkit_web_view_load_uri(view: *mut WebKitWebView, uri: *const c_char);
    pub fn webkit_web_view_go_back(view: *mut WebKitWebView);
    pub fn webkit_web_view_go_forward(view: *mut WebKitWebView);
    pub fn webkit_web_view_reload(view: *mut WebKitWebView);
    pub fn webkit_web_view_stop_loading(view: *mut WebKitWebView);
    pub fn webkit_web_view_get_uri(view: *mut WebKitWebView) -> *const c_char;
    pub fn webkit_web_view_get_title(view: *mut WebKitWebView) -> *const c_char;
    pub fn webkit_web_view_get_estimated_load_progress(view: *mut WebKitWebView) -> c_double;
    pub fn webkit_web_view_is_loading(view: *mut WebKitWebView) -> gboolean;
    pub fn webkit_web_view_can_go_back(view: *mut WebKitWebView) -> gboolean;
    pub fn webkit_web_view_can_go_forward(view: *mut WebKitWebView) -> gboolean;
    pub fn webkit_web_view_get_tls_info(
        view: *mut WebKitWebView,
        certificate: *mut *mut GTlsCertificate,
        errors: *mut c_uint,
    ) -> gboolean;
    pub fn webkit_web_view_get_settings(view: *mut WebKitWebView) -> *mut WebKitSettings;
    pub fn webkit_settings_new() -> *mut WebKitSettings;
    pub fn webkit_settings_get_user_agent(settings: *mut WebKitSettings) -> *const c_char;
    pub fn webkit_settings_set_user_agent(settings: *mut WebKitSettings, user_agent: *const c_char);
    pub fn webkit_settings_set_user_agent_with_application_details(
        settings: *mut WebKitSettings,
        application_name: *const c_char,
        application_version: *const c_char,
    );
    pub fn webkit_settings_set_enable_developer_extras(
        settings: *mut WebKitSettings,
        enabled: gboolean,
    );
    pub fn webkit_settings_set_javascript_can_open_windows_automatically(
        settings: *mut WebKitSettings,
        enabled: gboolean,
    );
    pub fn webkit_settings_set_allow_file_access_from_file_urls(
        settings: *mut WebKitSettings,
        allowed: gboolean,
    );
    pub fn webkit_settings_set_allow_universal_access_from_file_urls(
        settings: *mut WebKitSettings,
        allowed: gboolean,
    );
    pub fn webkit_settings_set_allow_top_navigation_to_data_urls(
        settings: *mut WebKitSettings,
        allowed: gboolean,
    );
    pub fn webkit_network_session_new(
        data_directory: *const c_char,
        cache_directory: *const c_char,
    ) -> *mut WebKitNetworkSession;
    pub fn webkit_network_session_new_ephemeral() -> *mut WebKitNetworkSession;
    pub fn webkit_network_session_set_itp_enabled(
        session: *mut WebKitNetworkSession,
        enabled: gboolean,
    );
    pub fn webkit_network_session_set_persistent_credential_storage_enabled(
        session: *mut WebKitNetworkSession,
        enabled: gboolean,
    );
    pub fn webkit_user_content_manager_new() -> *mut WebKitUserContentManager;
    pub fn webkit_user_content_manager_add_filter(
        manager: *mut WebKitUserContentManager,
        filter: *mut WebKitUserContentFilter,
    );
    pub fn webkit_user_content_manager_remove_all_filters(manager: *mut WebKitUserContentManager);
    pub fn webkit_user_content_filter_unref(filter: *mut WebKitUserContentFilter);
    pub fn webkit_user_content_filter_store_new(
        path: *const c_char,
    ) -> *mut WebKitUserContentFilterStore;
    pub fn webkit_user_content_filter_store_save_from_file(
        store: *mut WebKitUserContentFilterStore,
        identifier: *const c_char,
        file: *mut GFile,
        cancellable: gpointer,
        callback: GAsyncReadyCallback,
        user_data: gpointer,
    );
    pub fn webkit_user_content_filter_store_save_from_file_finish(
        store: *mut WebKitUserContentFilterStore,
        result: *mut GAsyncResult,
        error: *mut *mut GError,
    ) -> *mut WebKitUserContentFilter;
    pub fn webkit_user_content_filter_store_load(
        store: *mut WebKitUserContentFilterStore,
        identifier: *const c_char,
        cancellable: gpointer,
        callback: GAsyncReadyCallback,
        user_data: gpointer,
    );
    pub fn webkit_user_content_filter_store_load_finish(
        store: *mut WebKitUserContentFilterStore,
        result: *mut GAsyncResult,
        error: *mut *mut GError,
    ) -> *mut WebKitUserContentFilter;
    pub fn webkit_user_content_filter_store_remove(
        store: *mut WebKitUserContentFilterStore,
        identifier: *const c_char,
        cancellable: gpointer,
        callback: Option<GAsyncReadyCallback>,
        user_data: gpointer,
    );
    pub fn webkit_user_content_filter_store_fetch_identifiers(
        store: *mut WebKitUserContentFilterStore,
        cancellable: gpointer,
        callback: GAsyncReadyCallback,
        user_data: gpointer,
    );
    pub fn webkit_user_content_filter_store_fetch_identifiers_finish(
        store: *mut WebKitUserContentFilterStore,
        result: *mut GAsyncResult,
    ) -> *mut *mut c_char;
    pub fn webkit_policy_decision_use(decision: *mut WebKitPolicyDecision);
    pub fn webkit_policy_decision_download(decision: *mut WebKitPolicyDecision);
    pub fn webkit_response_policy_decision_is_mime_type_supported(
        decision: *mut WebKitPolicyDecision,
    ) -> gboolean;
    pub fn webkit_navigation_policy_decision_get_navigation_action(
        decision: *mut WebKitPolicyDecision,
    ) -> *mut WebKitNavigationAction;
    pub fn webkit_navigation_action_get_mouse_button(action: *mut WebKitNavigationAction)
        -> c_uint;
    pub fn webkit_policy_decision_ignore(decision: *mut WebKitPolicyDecision);
    pub fn webkit_navigation_action_get_request(
        action: *mut WebKitNavigationAction,
    ) -> *mut WebKitURIRequest;
    pub fn webkit_uri_request_get_uri(request: *mut WebKitURIRequest) -> *const c_char;
    pub fn webkit_download_set_destination(
        download: *mut WebKitDownload,
        destination: *const c_char,
    );
    pub fn webkit_download_get_destination(download: *mut WebKitDownload) -> *const c_char;
    pub fn webkit_download_set_allow_overwrite(download: *mut WebKitDownload, allowed: gboolean);
    pub fn webkit_permission_request_allow(request: *mut WebKitPermissionRequest);
    pub fn webkit_permission_request_deny(request: *mut WebKitPermissionRequest);
    pub fn webkit_user_media_permission_is_for_audio_device(
        request: *mut WebKitPermissionRequest,
    ) -> gboolean;
    pub fn webkit_user_media_permission_is_for_video_device(
        request: *mut WebKitPermissionRequest,
    ) -> gboolean;
    pub fn webkit_user_media_permission_is_for_display_device(
        request: *mut WebKitPermissionRequest,
    ) -> gboolean;
    pub fn webkit_hit_test_result_context_is_link(result: *mut WebKitHitTestResult) -> gboolean;
    pub fn webkit_hit_test_result_context_is_editable(result: *mut WebKitHitTestResult)
        -> gboolean;
    pub fn webkit_hit_test_result_get_link_uri(result: *mut WebKitHitTestResult) -> *const c_char;
}

/// Connects `handler`, a C function of the signal's own signature, to
/// `signal` on `instance`, with `data` as its last argument.
///
/// # Safety
/// `handler` must have the signal's signature and `instance` must be a
/// live GObject.
pub unsafe fn connect(
    instance: gpointer,
    signal: &str,
    handler: GCallback,
    data: gpointer,
) -> c_ulong {
    let signal = CString::new(signal).expect("signal names have no NUL");
    unsafe { g_signal_connect_data(instance, signal.as_ptr(), handler, data, None, 0) }
}

/// Casts a handler of any signature to the `GCallback` GObject stores.
#[macro_export]
macro_rules! callback {
    ($f:expr) => {
        $crate::ffi::as_callback($f as *const ())
    };
}

/// A function pointer as the `GCallback` GObject stores. Calling it as a
/// GCallback would be wrong; GObject never does, it calls it back with the
/// signature the signal has, which [`callback!`]'s caller wrote it for.
pub fn as_callback(f: *const ()) -> GCallback {
    // SAFETY: both are plain function pointers of the same size; the
    // result is only stored and passed to GObject.
    unsafe { std::mem::transmute::<*const (), GCallback>(f) }
}

/// A borrowed C string as an owned Rust one; `None` for NULL.
///
/// # Safety
/// `s` must be NULL or point to a NUL-terminated string.
pub unsafe fn string(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned())
    }
}

/// Takes a GError's message and frees it.
///
/// # Safety
/// `error` must be NULL or a GError the caller owns.
pub unsafe fn take_error(error: *mut GError) -> String {
    if error.is_null() {
        return "unknown error".into();
    }
    let message = unsafe { string((*error).message) }.unwrap_or_default();
    unsafe { g_error_free(error) };
    message
}

/// A Rust string as a C one, with any NUL cut off rather than failing: a
/// URL or title never legitimately has one.
pub fn cstring(s: &str) -> CString {
    let s = s.split('\0').next().unwrap_or_default();
    CString::new(s).expect("NUL removed")
}
