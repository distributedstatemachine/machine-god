//! Compose the actual private host resolver against this harness's host types.

#[path = "../../src/permission_targets/terminal.rs"]
mod terminal;
pub(crate) use terminal::{NativePermissionTerminalResolution, NativePermissionTerminalResolver};
#[path = "../../src/permission_targets/host.rs"]
mod host;
pub(crate) use host::HostPermissionResolver;

fn invalid() -> machine_god_core::PermissionError {
    machine_god_core::PermissionError::new(
        "permission_target_invalid",
        "permission target preparation failed",
    )
}
