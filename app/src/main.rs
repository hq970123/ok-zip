//! OK Zip — a Bandizip-style archiver with a native Win32 user interface.
//!
//! The UI is Windows-only; all archive work lives in `okzip-core` so a future
//! macOS (AppKit) front end can share it.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The OK Zip GUI is Windows-only. Use `okzip-cli` on this platform.");
    std::process::exit(1);
}
