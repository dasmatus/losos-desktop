//! Links WPE WebKit and its headless platform through pkg-config, which
//! also brings GLib, GObject and GIO, the libraries ffi.rs declares.

fn main() {
    for module in ["wpe-webkit-2.0", "wpe-platform-headless-2.0"] {
        if let Err(error) = pkg_config::Config::new()
            .atleast_version("2.54")
            .probe(module)
        {
            // `cargo check` and the pure tests do not link; let them run
            // where WPE WebKit is not installed.
            if std::env::var_os("DANUBE_NO_WPE").is_some() {
                println!("cargo:warning=not linking {module}: {error}");
            } else {
                panic!("{error}");
            }
        }
    }
    println!("cargo:rerun-if-env-changed=DANUBE_NO_WPE");
}
