use super::*;
use crate::mcp::auth::{
    McpAuthClock, McpAuthDestination, McpAuthEntropy, McpAuthError, McpAuthIdentity,
    McpAuthInvalidated, McpAuthInvalidation, McpAuthNetwork, NativeMcpAuthService,
    NativeMcpCredentialStore,
};
use std::sync::Mutex;

#[derive(Default)]
struct Authentication(Mutex<Vec<McpAuthIdentity>>);
impl McpAuthInvalidation for Authentication {
    fn invalidate(&self, event: McpAuthInvalidated) {
        lock(&self.0).push(event.identity);
    }
}
impl McpAuthNetwork for Authentication {
    fn admit<'a>(
        &'a self,
        _: &'a str,
        _: &'a CancellationToken,
        _: Instant,
    ) -> BoxFuture<'a, std::result::Result<McpAuthDestination, McpAuthError>> {
        panic!("missing credentials do not request OAuth network authority")
    }
}
impl McpAuthEntropy for Authentication {
    fn fill(&self, _: &mut [u8]) -> std::result::Result<(), McpAuthError> {
        panic!("startup never opens browser authorization")
    }
}
fn authenticated(fixture: &mut Fixture) -> Arc<Authentication> {
    let events = Arc::new(Authentication::default());
    fixture.options.stored_authentication = Some(Arc::new(NativeMcpAuthService::new(
        Arc::new(NativeMcpCredentialStore::new(fixture.base.join("credentials")).unwrap()),
        events.clone(),
        Arc::new(Clock::default()) as Arc<dyn McpAuthClock>,
        events.clone(),
        events.clone(),
        fixture.options.workers.clone(),
    )));
    events
}
fn required(fixture: &Fixture, listener: &TcpListener, path: &str) {
    fixture.seed(&format!(r#"{{"mcp":{{"optional":{{"type":"http","url":"http://127.0.0.1:{}{}","required":true,"startup_timeout_ms":5000}}}}}}"#, listener.local_addr().unwrap().port(), path));
}
async fn serve(listener: &TcpListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    discovery(&mut socket).await;
}

#[test]
fn changed_and_removed_publication_releases_missing_identity_despite_retained_receipts() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        required(&fixture, &listener, "/old");
        let events = authenticated(&mut fixture);
        let controller = fixture.controller();
        let (original, ()) = join(
            controller.start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            serve(&listener),
        )
        .await;
        let original = original.unwrap();
        assert!(lock(&events.0).is_empty());
        required(&fixture, &listener, "/new");
        let (replacement, ()) = join(
            controller.reload(CancellationToken::new(), deadline()),
            serve(&listener),
        )
        .await;
        let replacement = replacement.unwrap();
        assert_eq!(lock(&events.0).len(), 1);
        assert!(lock(&events.0)[0].endpoint().ends_with("/old"));
        fixture.seed(r#"{"mcp":{}}"#);
        let removal = controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap();
        assert_eq!(lock(&events.0).len(), 2);
        assert!(lock(&events.0)[1].endpoint().ends_with("/new"));
        assert_eq!(
            original.publication(),
            NativeMcpControllerPublication::Published
        );
        assert_eq!(
            replacement.publication(),
            NativeMcpControllerPublication::Published
        );
        assert_eq!(
            removal.publication(),
            NativeMcpControllerPublication::Published
        );
    });
}

#[test]
fn same_identity_reload_keeps_selection_until_current_publication_is_removed() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        required(&fixture, &listener, "/same");
        let events = authenticated(&mut fixture);
        let controller = fixture.controller();
        let (original, ()) = join(
            controller.start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            serve(&listener),
        )
        .await;
        let original = original.unwrap();
        let (replacement, ()) = join(
            controller.reload(CancellationToken::new(), deadline()),
            serve(&listener),
        )
        .await;
        let replacement = replacement.unwrap();
        assert!(lock(&events.0).is_empty());
        fixture.seed(r#"{"mcp":{}}"#);
        controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap();
        assert_eq!(lock(&events.0).len(), 1);
        drop((original, replacement));
    });
}

#[test]
fn failed_replacement_releases_only_rejected_identity() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        required(&fixture, &listener, "/old");
        let events = authenticated(&mut fixture);
        let controller = fixture.controller();
        let (original, ()) = join(
            controller.start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            serve(&listener),
        )
        .await;
        let original = original.unwrap();
        required(&fixture, &listener, "/rejected");
        let reject = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        };
        let (failure, ()) = join(
            controller.reload(CancellationToken::new(), deadline()),
            reject,
        )
        .await;
        let failure = failure.unwrap_err();
        assert_eq!(lock(&events.0).len(), 1);
        assert!(lock(&events.0)[0].endpoint().ends_with("/rejected"));
        assert!(!original.generation.cancellation.is_cancelled());
        controller.close();
        assert!(
            lock(&events.0)
                .iter()
                .any(|identity| identity.endpoint().ends_with("/old"))
        );
        assert!(failure.cleanup_complete());
    });
}

#[test]
fn dropped_controller_releases_missing_selection_while_receipt_remains() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        required(&fixture, &listener, "/drop");
        let events = authenticated(&mut fixture);
        let controller = fixture.controller();
        let (receipt, ()) = join(
            controller.start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            serve(&listener),
        )
        .await;
        let receipt = receipt.unwrap();
        drop(controller);
        assert!(!lock(&events.0).is_empty());
        assert_eq!(
            receipt.publication(),
            NativeMcpControllerPublication::Published
        );
        assert!(receipt.generation.cancellation.is_cancelled());
    });
}
