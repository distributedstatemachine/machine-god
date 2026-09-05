//! Component regression harness before persistent-runtime composition.
#![allow(
    dead_code,
    reason = "private component APIs are exercised independently before runtime composition"
)]

#[path = "../src/terminal_display_width.rs"]
mod terminal_display_width;
#[path = "../src/terminal_grid.rs"]
mod terminal_grid;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "../src/terminal_journal.rs"]
mod terminal_journal;
#[path = "../src/terminal_monitor.rs"]
mod terminal_monitor;
#[path = "../src/terminal_unicode_data.rs"]
mod terminal_unicode_data;
