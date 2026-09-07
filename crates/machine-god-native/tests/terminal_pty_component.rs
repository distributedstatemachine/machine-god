#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(
    dead_code,
    reason = "private runtime components are composed here before full host integration"
)]
use machine_god_native::{
    NativeOwnedWorkerCleanup, NativeOwnedWorkerScope, NativeOwnedWorkerSpawner,
    TERMINAL_CAPTURED_HELPER_ARGUMENT,
};
mod terminal_action_tool {
    pub(crate) use machine_god_native::{TerminalActionHostIdentity, TerminalActionInvocation};
}
mod terminal_action_parse {
    pub(crate) use machine_god_native::decode_terminal_action;
}
#[path = "../src/background_input.rs"]
mod background_input;
#[path = "../src/background_process.rs"]
mod background_process;
#[cfg(target_os = "macos")]
#[path = "../src/process_inventory_helper.rs"]
mod process_inventory_helper;
#[cfg(target_os = "macos")]
#[path = "../src/process_inventory_protocol.rs"]
mod process_inventory_protocol;
#[cfg(target_os = "macos")]
use machine_god_native::NativeOwnedWorkerScopeIdentity;
#[cfg(target_os = "macos")]
use process_inventory_protocol::PROCESS_INVENTORY_SERVICE_ARGUMENT;
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
#[path = "../src/terminal_host_authority.rs"]
mod terminal_host_authority;
#[path = "../src/terminal_host_catalog.rs"]
mod terminal_host_catalog;
#[path = "../src/terminal_host_dispatch.rs"]
mod terminal_host_dispatch;
#[path = "../src/terminal_input.rs"]
mod terminal_input;
#[path = "../src/terminal_journal.rs"]
mod terminal_journal;
#[path = "../src/terminal_monitor.rs"]
mod terminal_monitor;
#[path = "../src/terminal_native_backend.rs"]
mod terminal_native_backend;
#[path = "../src/terminal_native_launch.rs"]
mod terminal_native_launch;
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
#[path = "../src/terminal_resident_dispatch.rs"]
mod terminal_resident_dispatch;
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
#[path = "../src/terminal_staged_start.rs"]
mod terminal_staged_start;
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
