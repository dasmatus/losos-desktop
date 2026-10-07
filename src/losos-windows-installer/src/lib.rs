//! LosOS's installer for a machine that runs Windows: it shrinks the
//! Windows partition and installs LosOS beside it, so both boot.
//! docs/windows-installer.md is the whole story; main.rs is the program.
pub mod disk;
pub mod efi;
pub mod fat;
pub mod guid;
#[cfg(unix)]
pub mod image;
pub mod install;
pub mod layout;
pub mod release;
#[cfg(windows)]
pub mod windows;

/// What the OS's configuration decided at build time (build.rs).
pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}
