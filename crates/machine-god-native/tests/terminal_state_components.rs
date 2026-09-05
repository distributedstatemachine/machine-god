//! Component regression harness before persistent-runtime composition.
#![allow(
    dead_code,
    reason = "private component APIs are exercised independently before runtime composition"
)]

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "../src/terminal_catalog.rs"]
mod terminal_catalog;
#[path = "../src/terminal_display_width.rs"]
mod terminal_display_width;
#[path = "../src/terminal_grid.rs"]
mod terminal_grid;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "../src/terminal_journal.rs"]
mod terminal_journal;
#[path = "../src/terminal_monitor.rs"]
mod terminal_monitor;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "../src/terminal_profile_store.rs"]
mod terminal_profile_store;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "../src/terminal_session_record.rs"]
mod terminal_session_record;
#[path = "../src/terminal_unicode_data.rs"]
mod terminal_unicode_data;
