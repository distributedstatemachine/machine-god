#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(
    dead_code,
    reason = "private runtime components are composed here before full host integration"
)]
#[path = "../src/background_input.rs"]
mod background_input;
#[path = "../src/background_process.rs"]
mod background_process;
#[path = "../src/terminal_catalog.rs"]
mod terminal_catalog;
#[path = "../src/terminal_display_width.rs"]
mod terminal_display_width;
#[path = "../src/terminal_grid.rs"]
mod terminal_grid;
#[path = "../src/terminal_history.rs"]
mod terminal_history;
#[path = "../src/terminal_input.rs"]
mod terminal_input;
#[path = "../src/terminal_journal.rs"]
mod terminal_journal;
#[path = "../src/terminal_monitor.rs"]
mod terminal_monitor;
#[path = "../src/terminal_owner.rs"]
mod terminal_owner;
#[path = "../src/terminal_pty.rs"]
mod terminal_pty;
#[path = "../src/terminal_registry.rs"]
mod terminal_registry;
#[path = "../src/terminal_screen.rs"]
mod terminal_screen;
#[path = "../src/terminal_session.rs"]
mod terminal_session;
#[path = "../src/terminal_session_record.rs"]
mod terminal_session_record;
#[path = "../src/terminal_unicode_data.rs"]
mod terminal_unicode_data;
