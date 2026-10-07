//! The C API Danube links: GLib, GObject and GIO's handful of calls, WPE
//! WebKit's WebKitWebView and friends, and WPEPlatform's display, view,
//! toplevel, events and clipboard. bindgen writes every declaration here
//! from the installed headers when the crate builds (build.rs), so a
//! signature or enum value is always the library's own; nothing is copied
//! by hand. Only webkit.rs calls into this module, and wraps each object
//! in a type that owns its reference.

#![allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
