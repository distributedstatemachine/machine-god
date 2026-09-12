use super::*;
use crate::{
    conversation_lifecycle::{LifecycleError, LifecycleGate},
    mcp::auth::{
        McpAuthClock, McpAuthDestination, McpAuthEntropy, McpAuthError, McpAuthInvalidated,
        McpAuthInvalidation, McpAuthLocalRemoval, McpAuthNetwork, McpAuthRemoteRevocation,
        NativeMcpAuthService, NativeMcpCredentialStore,
    },
};
use std::result::Result;

struct NoEffects;
impl McpAuthNetwork for NoEffects {
    fn admit<'a>(
        &'a self,
        _: &'a str,
        _: &'a CancellationToken,
        _: Instant,
    ) -> BoxFuture<'a, Result<McpAuthDestination, McpAuthError>> {
        panic!("selection or empty logout must not perform OAuth network work")
    }
}
impl McpAuthEntropy for NoEffects {
    fn fill(&self, _: &mut [u8]) -> Result<(), McpAuthError> {
        panic!("selection or logout must not create browser entropy")
    }
}
impl McpAuthInvalidation for NoEffects {
    fn invalidate(&self, _: McpAuthInvalidated) {}
}
impl McpAuthClock for Clock {
    fn unix_millis(&self) -> i64 {
        1_000_000
    }
}
fn configured() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.options.stored_authentication = Some(Arc::new(NativeMcpAuthService::new(
        Arc::new(NativeMcpCredentialStore::new(fixture.base.join("credentials")).unwrap()),
        Arc::new(NoEffects),
        Arc::new(Clock::default()),
        Arc::new(NoEffects),
        Arc::new(NoEffects),
        fixture.options.workers.clone(),
    )));
    fixture
}
fn fence(gate: &Arc<LifecycleGate>) -> Arc<ControlFence> {
    ControlFence::new(gate.acquire().unwrap())
}
fn assert_error(
    result: Result<NativeMcpAuthSelection, NativeMcpControllerFailure>,
    expected: NativeMcpControllerError,
) {
    match result {
        Err(error) => assert_eq!(error.kind(), expected),
        Ok(_) => panic!("unexpected selection"),
    }
}

#[test]
fn auth_selection_unpolled_and_precancelled_never_loads_profile() {
    let fixture = configured();
    let controller = fixture.controller();
    let gate = LifecycleGate::new();
    drop(controller.prepare_authentication(
        "srv".into(),
        fence(&gate),
        CancellationToken::new(),
        deadline(),
    ));
    assert!(lock(&controller.inner.state).generations.is_empty());
    assert!(!fixture.base.join("profile").exists());
    run(async {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_error(
            controller
                .prepare_authentication("srv".into(), fence(&gate), cancel, deadline())
                .await,
            NativeMcpControllerError::Cancelled,
        );
    });
    assert!(lock(&controller.inner.state).generations.is_empty());
    assert!(!fixture.base.join("credentials").exists());
    assert_eq!(gate.begin_quiescence().unwrap().check_idle(), Ok(()));
}

#[test]
fn auth_selection_accepts_disabled_remote_without_loading_oauth_secrets() {
    run(async {
        let fixture = configured();
        fixture.seed(r#"{"mcp":{"srv":{"type":"http","url":"https://example.com/mcp","enabled":false,"oauth":{"client_id":"test-client","client_secret_env":"UNAVAILABLE_SECRET"}}}}"#);
        let controller = fixture.controller();
        let gate = LifecycleGate::new();
        let selected = controller
            .prepare_authentication(
                "srv".into(),
                fence(&gate),
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        assert_eq!(&*selected.server, "srv");
        assert!(selected.check().is_ok());
        assert!(!fixture.base.join("credentials").exists());
        assert_eq!(
            controller
                .reload_configured(CancellationToken::new())
                .await
                .unwrap_err()
                .kind(),
            NativeMcpControllerError::Busy
        );
        assert_error(
            controller
                .prepare_authentication(
                    "srv".into(),
                    fence(&gate),
                    CancellationToken::new(),
                    deadline(),
                )
                .await,
            NativeMcpControllerError::Busy,
        );
        let quiescence = gate.begin_quiescence().unwrap();
        assert_eq!(selected.check(), Err(McpAuthError::Cancelled));
        assert_eq!(quiescence.check_idle(), Err(LifecycleError::Busy));
        drop(selected);
        assert_eq!(quiescence.check_idle(), Ok(()));
    });
}

#[test]
fn auth_selection_rejects_unknown_stdio_closed_and_expired_commands() {
    run(async {
        let fixture = configured();
        fixture.seed(r#"{"mcp":{"stdio":{"command":["never-execute"]}}}"#);
        let controller = fixture.controller();
        let gate = LifecycleGate::new();
        for name in ["missing", "stdio", "bad/name"] {
            assert_error(
                controller
                    .prepare_authentication(
                        name.into(),
                        fence(&gate),
                        CancellationToken::new(),
                        deadline(),
                    )
                    .await,
                NativeMcpControllerError::Invalid,
            );
        }
        assert_error(
            controller
                .prepare_authentication(
                    "stdio".into(),
                    fence(&gate),
                    CancellationToken::new(),
                    Instant::now(),
                )
                .await,
            NativeMcpControllerError::Deadline,
        );
        controller.close();
        assert_error(
            controller
                .prepare_authentication(
                    "stdio".into(),
                    fence(&gate),
                    CancellationToken::new(),
                    deadline(),
                )
                .await,
            NativeMcpControllerError::Closed,
        );
        assert!(!fixture.base.join("credentials").exists());
    });
}

#[test]
fn auth_empty_logout_has_independent_receipts_and_does_not_contact_network() {
    run(async {
        let fixture = configured();
        fixture.seed(r#"{"mcp":{"srv":{"type":"http","url":"https://example.com/mcp"}}}"#);
        let controller = fixture.controller();
        let gate = LifecycleGate::new();
        let selected = controller
            .prepare_authentication(
                "srv".into(),
                fence(&gate),
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let receipt = selected.logout().await.unwrap();
        assert_eq!(receipt.local, McpAuthLocalRemoval::Unchanged);
        assert_eq!(receipt.remote, McpAuthRemoteRevocation::NotAttempted);
        assert!(!fixture.base.join("credentials").exists());
        drop(selected);
        assert_eq!(gate.begin_quiescence().unwrap().check_idle(), Ok(()));
    });
}

#[test]
fn auth_logout_rejects_changed_profile_before_credential_write() {
    run(async {
        let fixture = configured();
        fixture.seed(r#"{"mcp":{"srv":{"type":"http","url":"https://example.com/mcp"}}}"#);
        let controller = fixture.controller();
        let gate = LifecycleGate::new();
        let selected = controller
            .prepare_authentication(
                "srv".into(),
                fence(&gate),
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture.seed(r#"{"mcp":{}}"#);
        assert_eq!(selected.logout().await, Err(McpAuthError::Conflict));
        assert!(!fixture.base.join("credentials").exists());
    });
}
