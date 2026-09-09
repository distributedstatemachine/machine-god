//! Inject only native transport authority and the real private input helper.

use super::super::super::{AskSignalControlSender, AskSignalController, PreparedConversationHost};
use crate::ask::{
    AskCommandExecution, AskCommandHost, AskCommandOutcome, InteractiveSessionSelection,
    SessionSelection,
};
use machine_god_core::NetworkTarget;
use machine_god_native::{
    AiGatewayBearerToken, AiGatewayHttpEndpoint, AiGatewayHttpLimits, AiGatewayHttpTransport,
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogHttpEndpoint,
    AiGatewayModelCatalogHttpLimits, AiGatewayModelCatalogHttpTransport,
    AiGatewayModelCatalogProvider, FileUndoTracker, NativeConversationModelRoutes,
    NativeConversationObservations, NativeEnvironment, NativeInteractiveInputHelper,
    NativeInteractiveInputSource, NativeInteractivePromptBridge, NativeInteractiveTerminal,
    NativeModelCatalogCache, NativeReferenceHost, NativeReferenceHostConversationOptions,
    NativeReferenceHostTerminalOptions, NativeRootSelection, NativeWorkspaceContexts,
    PreparedNativeRoots, TokioWebSearchDeadline, load_native_config,
};
use std::{fs::File, net::SocketAddr, os::fd::AsFd, path::PathBuf, sync::Arc};

pub(super) struct Host {
    pub required: bool,
}

impl AskCommandHost for Host {
    fn execute_interactive(
        &self,
        selection: InteractiveSessionSelection,
        output: &mut dyn std::io::Write,
    ) -> AskCommandExecution {
        let controller = AskSignalController::spawn().unwrap();
        assert!(controller.registration_complete());
        let (outcome, controller) = super::super::execute_with_preparation(
            self.required,
            selection,
            output,
            controller,
            prepare,
            capture_input,
        );
        AskCommandExecution::with_finalizer(outcome, controller)
    }

    fn execute(
        &self,
        _: SessionSelection,
        _: String,
        _: &mut dyn std::io::Write,
    ) -> AskCommandExecution {
        AskCommandExecution::without_finalizer(AskCommandOutcome::OperationalFailure)
    }
}

fn helper() -> PathBuf {
    let path = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/release/machine-god"),
        PathBuf::from,
    );
    assert!(
        path.is_file(),
        "select the real CLI private helper before recording subprocess tests"
    );
    path
}

fn capture_input() -> Result<(NativeInteractiveInputSource, NativeInteractiveTerminal), ()> {
    let path = helper();
    let helper = NativeInteractiveInputHelper::new(&path, File::open(&path).map_err(|_| ())?)
        .map_err(|_| ())?;
    let input = File::from(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| ())?,
    );
    let terminal = NativeInteractiveTerminal::new(input.try_clone().map_err(|_| ())?);
    Ok((
        NativeInteractiveInputSource::PreserveShared { input, helper },
        terminal,
    ))
}

fn prepare(
    bridge: Arc<NativeInteractivePromptBridge>,
    control: &AskSignalControlSender,
) -> Result<PreparedConversationHost, ()> {
    control.activate_turn()?;
    let environment = NativeEnvironment::from_process();
    let root_selection = NativeRootSelection::from_current_process(&environment).map_err(|_| ())?;
    let prepared_roots = PreparedNativeRoots::prepare(root_selection.clone()).map_err(|_| ())?;
    let workspace = prepared_roots.workspace_root().to_owned();
    let state_path = prepared_roots.state_root().to_owned();
    let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().map_err(|_| ())?;
    let address: SocketAddr = std::env::var("RECORDING_TEST_GATEWAY")
        .map_err(|_| ())?
        .parse()
        .map_err(|_| ())?;
    assert!(address.ip().is_loopback());
    let catalog_transport = AiGatewayModelCatalogHttpTransport::with_endpoint_and_limits(
        Some(AiGatewayBearerToken::new("local-fixture-token").unwrap()),
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&format!("http://{address}/catalog"))
            .unwrap(),
        AiGatewayModelCatalogHttpLimits::default(),
    )
    .map_err(|_| ())?;
    let cache = Arc::new(NativeModelCatalogCache::new(Arc::new(
        AiGatewayModelCatalogProvider::new(
            AiGatewayModelCatalogAccessMode::Authenticated,
            Arc::new(catalog_transport),
        ),
    )));
    let catalog = runtime.block_on(super::super::super::load_conversation_catalog(&cache))?;
    assert!(
        catalog.is_some(),
        "the local Gateway catalog must be usable"
    );
    let transport = AiGatewayHttpTransport::with_endpoint_and_limits(
        AiGatewayBearerToken::new("local-fixture-token").unwrap(),
        AiGatewayHttpEndpoint::loopback_http(&format!("http://{address}/inference")).unwrap(),
        AiGatewayHttpLimits::default(),
    )
    .map_err(|_| ())?;
    let model_routes = Arc::new(NativeConversationModelRoutes::new());
    let observations = Arc::new(NativeConversationObservations::new());
    let authority = super::super::super::prepare_launch_workspace(
        &runtime,
        root_selection,
        None,
        &crate::workspace::launch::LaunchWorkspaceOptions::EMPTY,
    )?;
    let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_workspace(authority, Arc::new(NativeWorkspaceContexts::new()))
        .with_model_routes(Arc::clone(&model_routes))
        .with_observations(Arc::clone(&observations))
        .with_terminal(
            NativeReferenceHostTerminalOptions::new(
                helper(),
                Some("/bin/bash".into()),
                std::env::vars_os().collect(),
            )
            .map_err(|_| ())?,
        )
        .with_permissions(super::super::super::capture_permission_options());
    let host =
        NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            load_native_config(&environment).map_err(|_| ())?,
            Arc::new(transport),
            NetworkTarget {
                scheme: "http".into(),
                host: address.ip().to_string(),
                port: Some(address.port()),
            },
            prepared_roots,
            bridge.clone(),
            bridge,
            Arc::new(deadline),
            options,
        )
        .map_err(|_| ())?;
    Ok(PreparedConversationHost {
        host,
        runtime,
        workspace,
        state_path,
        model_routes,
        observations,
        catalog,
        catalog_cache: cache,
        user_config: None,
    })
}
