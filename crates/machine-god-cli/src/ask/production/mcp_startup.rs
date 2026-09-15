//! Thin MCP profile selection, caller-polled activation and host settlement.

use machine_god_native::mcp::{
    management::NativeMcpManagementService, store::NativeMcpConfigStore,
};
use std::{path::Path, sync::Arc};

pub(super) const INTERACTIVE_FAILURE_NOTICE: &[u8] = b"MCP startup failed; management remains available via /mcp. Required servers must be ready before a new model prompt can run.\n";

/// Applies captured selections without loading a profile or activating peers.
pub(super) fn configure_host(
    mut options: machine_god_native::NativeReferenceHostConversationOptions,
    management: Option<Arc<NativeMcpManagementService>>,
    runtime: Option<machine_god_native::NativeReferenceHostMcpOptions>,
) -> machine_god_native::NativeReferenceHostConversationOptions {
    if let Some(management) = management {
        options = options.with_mcp_management(management);
    }
    if let Some(runtime) = runtime {
        options = options.with_mcp_runtime(runtime);
    }
    options
}

/// No profile selection means no MCP capture. All effectful acquisition remains
/// in the explicit native startup boundary on this existing constructor worker.
pub(super) fn prepare_runtime(
    roots: &machine_god_native::PreparedNativeRoots,
    terminal: &machine_god_native::NativeReferenceHostTerminalOptions,
    management: Option<&NativeMcpManagementService>,
    presenter: Option<Arc<dyn machine_god_native::mcp::interaction::McpElicitationPresenter>>,
) -> Result<Option<machine_god_native::NativeReferenceHostMcpOptions>, ()> {
    management
        .map(|_| {
            let options = machine_god_native::NativeReferenceHostMcpOptions::capture_startup(
                roots,
                terminal,
                Arc::new(machine_god_native::mcp::context::NativeMcpContexts::new()),
            )
            .map_err(|_| ())?;
            Ok(match presenter {
                Some(presenter) => options.with_form_responder(presenter),
                None => options,
            })
        })
        .transpose()
}

/// First signals cancel the actual startup owner. Continue polling the native
/// operation so its publication/cleanup receipt is not abandoned by a select.
pub(super) async fn activate(
    host: &machine_god_native::NativeReferenceHost,
    phase: machine_god_native::mcp::startup::NativeMcpStartupPhase,
    signals: &mut super::AskSignals,
) -> Result<(), ()> {
    match activate_observed(host, phase, signals).await? {
        None => Ok(()),
        Some(_) => Err(()),
    }
}

/// Interactive management survives discovery failure. Native conversation
/// admission checks required readiness independently for each new model prompt.
pub(super) async fn activate_interactive(
    host: &machine_god_native::NativeReferenceHost,
    signals: &mut super::AskSignals,
) -> Result<Option<&'static [u8]>, ()> {
    Ok(activate_observed(
        host,
        machine_god_native::mcp::startup::NativeMcpStartupPhase::All,
        signals,
    )
    .await?
    .map(|_| INTERACTIVE_FAILURE_NOTICE))
}

async fn activate_observed(
    host: &machine_god_native::NativeReferenceHost,
    phase: machine_god_native::mcp::startup::NativeMcpStartupPhase,
    signals: &mut super::AskSignals,
) -> Result<Option<machine_god_native::mcp::controller::NativeMcpControllerFailure>, ()> {
    let Some(controller) = host.mcp_controller() else {
        return Ok(None);
    };
    let cancellation = machine_god_core::CancellationToken::new();
    let mut operation = controller.start_configured(phase, cancellation.clone());
    let result = std::future::poll_fn(|cx| {
        if signals.first_observed.is_some() || signals.poll_signal(cx).is_ready() {
            cancellation.cancel();
        }
        operation.as_mut().poll(cx)
    })
    .await;
    if cancellation.is_cancelled() {
        return Err(());
    }
    match result {
        Ok(receipt) if !receipt.closed_after_publication() => Ok(None),
        Ok(_) => Err(()),
        Err(failure) => {
            use machine_god_native::mcp::controller::NativeMcpControllerError;
            if matches!(
                failure.kind(),
                NativeMcpControllerError::Closed | NativeMcpControllerError::Cancelled
            ) {
                Err(())
            } else {
                Ok(Some(failure))
            }
        }
    }
}

/// Runs on the existing blocking CLI worker, before dropping its host lease.
/// A fresh cleanup token is independent of the cancelled model/startup token.
pub(super) fn settle(
    host: &machine_god_native::NativeReferenceHost,
    runtime: &machine_god_native::TokioWebSearchRuntime,
) -> Result<(), ()> {
    host.close_mcp();
    let Some(controller) = host.mcp_controller() else {
        return Ok(());
    };
    let deadline = controller
        .deadline_after(std::time::Duration::from_secs(30))
        .map_err(|_| ())?;
    let receipt = runtime
        .block_on(controller.settle(deadline, machine_god_core::CancellationToken::new()))
        .map_err(|_| ())?;
    if !receipt.complete {
        return Err(());
    }
    Ok(())
}

/// Does not load configuration, resolve environment values or activate servers.
/// Missing profile selection means unavailable authority, not an ambient fallback.
pub(super) fn prepare(
    profile_directory: Option<&Path>,
) -> Result<Option<Arc<NativeMcpManagementService>>, ()> {
    profile_directory
        .map(|directory| {
            let store = NativeMcpConfigStore::new(directory.to_owned()).map_err(|_| ())?;
            Ok(Arc::new(NativeMcpManagementService::new(Arc::new(store))))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_native::{NativeEnvironment, inspect_native_status};

    #[test]
    fn mcp_startup_uses_only_the_selected_native_profile() {
        let captured = super::super::skills_startup::environment(&[
            ("XDG_CONFIG_HOME".into(), "/selected/config".into()),
            ("XDG_STATE_HOME".into(), "/unselected/state".into()),
            ("HOME".into(), "/unselected/home".into()),
        ]);
        let status = inspect_native_status(&captured);
        let directory = status.config_file_path().and_then(Path::parent);
        assert_eq!(directory, Some(Path::new("/selected/config/machine-god")));
        assert!(prepare(directory).unwrap().is_some());
        let absent = inspect_native_status(&NativeEnvironment::new(None, None, None));
        assert!(
            prepare(absent.config_file_path().and_then(Path::parent))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn mcp_startup_rejects_invalid_explicit_authority_without_fallback() {
        for directory in ["relative", "/", "/selected/../other", "/bad\0profile"] {
            assert!(prepare(Some(Path::new(directory))).is_err());
        }
    }
}
