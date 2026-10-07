//! Links WPE WebKit and its headless platform through pkg-config, and
//! writes the Rust declarations of their C API with bindgen from the
//! installed headers, so that every signature, enum value and struct
//! layout ffi.rs exposes is the one the library was built with rather
//! than one copied by hand.

use std::env;
use std::path::PathBuf;

fn main() {
    let mut include = Vec::new();
    for module in ["wpe-webkit-2.0", "wpe-platform-headless-2.0"] {
        let library = pkg_config::Config::new()
            .atleast_version("2.54")
            .probe(module)
            .unwrap_or_else(|error| panic!("{error}"));
        include.extend(library.include_paths);
    }

    // WebKit's own header pulls in GLib, GObject and GIO; the platform's
    // adds the headless display. Everything the crate declares comes
    // through these two.
    let header = "#include <wpe/webkit.h>\n#include <wpe/headless/wpe-headless.h>\n";
    let bindings = bindgen::Builder::default()
        .header_contents("danube.h", header)
        .clang_args(include.iter().map(|p| format!("-I{}", p.display())))
        // Only WebKit's and WPE's API, and the few GLib calls the engine
        // makes itself; the types they mention come along with them. Not
        // the whole of GLib, which would be most of the output.
        .allowlist_function("(webkit|wpe)_.*")
        .allowlist_function(
            "g_(main_loop_(new|run|quit|ref|unref)|idle_add_full|timeout_add_full|free|strfreev|error_free|bytes_(get_data|unref)|object_(new|ref|unref)|signal_connect_data|type_name_from_instance|file_new_for_path)",
        )
        .allowlist_type("(WebKit|WPE).*")
        .allowlist_var("(WEBKIT|WPE)_.*")
        // C enums as plain integer constants, as GLib's API takes them: a
        // Rust enum would be undefined behaviour for a value WebKit adds
        // in a later release.
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .expect("bindgen reads WPE WebKit's headers");
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("bindings.rs is written");
}
