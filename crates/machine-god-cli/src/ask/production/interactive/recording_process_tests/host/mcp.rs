//! Explicit test selection calls the same production MCP capture helpers.
use super::*;
use machine_god_native::{
    NativeReferenceHostMcpOptions, mcp::management::NativeMcpManagementService,
};

pub(super) fn prepare(
    roots: &PreparedNativeRoots,
    terminal: &NativeReferenceHostTerminalOptions,
    environment: &NativeEnvironment,
    bridge: Arc<NativeInteractivePromptBridge>,
) -> Result<
    Option<(
        Arc<NativeMcpManagementService>,
        NativeReferenceHostMcpOptions,
    )>,
    (),
> {
    match std::env::var("RECORDING_TEST_MCP").as_deref() {
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Ok("1") => {}
        _ => return Err(()),
    }
    let status = machine_god_native::inspect_native_status(environment);
    let management = super::super::super::super::mcp_startup::prepare(
        status.config_file_path().and_then(std::path::Path::parent),
    )?
    .ok_or(())?;
    let runtime = super::super::super::super::mcp_startup::prepare_runtime(
        roots,
        terminal,
        Some(&management),
        Some(bridge),
    )?
    .ok_or(())?;
    Ok(Some((management, runtime)))
}
