#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(
    dead_code,
    reason = "private runtime components are composed here before full host integration"
)]
use machine_god_native::NativeOwnedWorkerSpawner;
#[path = "../src/background_input.rs"]
mod background_input;
#[path = "../src/background_process.rs"]
mod background_process;
#[path = "../src/terminal_catalog.rs"]
mod terminal_catalog;
#[path = "../src/terminal_catalog_view.rs"]
mod terminal_catalog_view;
#[path = "../src/terminal_display_width.rs"]
mod terminal_display_width;
#[path = "../src/terminal_grid.rs"]
mod terminal_grid;
#[path = "../src/terminal_helper.rs"]
mod terminal_helper;
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
#[path = "../src/terminal_profile.rs"]
mod terminal_profile;
#[path = "../src/terminal_profile_store.rs"]
mod terminal_profile_store;
#[path = "../src/terminal_pty.rs"]
mod terminal_pty;
#[path = "../src/terminal_registry.rs"]
mod terminal_registry;
#[path = "../src/terminal_runtime.rs"]
mod terminal_runtime;
#[path = "../src/terminal_screen.rs"]
mod terminal_screen;
#[path = "../src/terminal_session.rs"]
mod terminal_session;
#[path = "../src/terminal_session_record.rs"]
mod terminal_session_record;
#[path = "../src/terminal_shell.rs"]
mod terminal_shell;
#[path = "../src/terminal_startup.rs"]
mod terminal_startup;
#[path = "../src/terminal_tmux.rs"]
mod terminal_tmux;
#[path = "../src/terminal_tmux_helper.rs"]
mod terminal_tmux_helper;
#[path = "../src/terminal_tmux_startup.rs"]
mod terminal_tmux_startup;
#[path = "../src/terminal_unicode_data.rs"]
mod terminal_unicode_data;
#[path = "../src/terminal_wait.rs"]
mod terminal_wait;
#[path = "../src/terminal_write_completion.rs"]
mod terminal_write_completion;
