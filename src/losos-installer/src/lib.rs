//! The installer's pieces, as a library, so `tests/` can drive each one
//! against a fixture tree instead of a machine.
//!
//! The binary is the terminal interface over these. What it installs is not
//! decided here: systemd-repart lays out the disk from definitions the OS
//! wrote, and systemd-sysupdate fills slot A from transfers the OS wrote
//! (nixos/installer/default.nix). This crate picks the network and the disk,
//! and runs the two in order.
pub mod disks;
pub mod install;
pub mod network;
pub mod wpa;
