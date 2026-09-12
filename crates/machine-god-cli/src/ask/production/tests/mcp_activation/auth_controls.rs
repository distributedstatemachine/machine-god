//! Actual session admission using production profile-auth composition. These
//! scenarios require the fresh release helper; execute only in the coordinated
//! runtime window. OAuth acceptance uses an explicitly bound local mock browser.

use super::*;
use machine_god_native::{
    NativeConversationRuntimeError, NativeInteractiveControl, NativeInteractiveControlError,
    NativeInteractiveControlOutcome, NativeInteractiveControlReceipt,
    NativeInteractiveInitialSession, NativeInteractiveSession, NativeInteractiveSessionOptions,
    TokioWebSearchRuntime,
    mcp::{
        auth::{McpAuthError, McpAuthLocalRemoval, McpAuthRemoteRevocation},
        commands::McpCommand,
        controller::{
            NativeMcpAuthenticationError, NativeMcpAuthenticationReceipt, NativeMcpControllerError,
        },
    },
};
use std::net::{Ipv4Addr, TcpListener};

mod browser;
mod oauth;
mod server;

struct Fixture {
    host: Option<Arc<NativeReferenceHost>>,
    runtime: TokioWebSearchRuntime,
    provider: Arc<OneShotTransport>,
    listener: TcpListener,
    directory: ScopedTestDirectory,
}

impl Fixture {
    fn new(secret: bool, required: bool) -> Self {
        Self::configured(|endpoint| {
            if secret {
                serde_json::json!({"type":"http", "url":endpoint, "enabled":required, "required":required,
                    "oauth":{"client_id":"selected-client", "client_secret_env":"UNAVAILABLE_MCP_FIXTURE_SECRET"}})
            } else {
                serde_json::json!({"type":"http", "url":endpoint, "enabled":false})
            }
        })
    }

    fn configured(remote: impl FnOnce(&str) -> serde_json::Value) -> Self {
        let directory = ScopedTestDirectory::new("mcp-auth-controls");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!(
            "http://127.0.0.1:{}/mcp",
            listener.local_addr().unwrap().port()
        );
        let remote = remote(&endpoint);
        let config = serde_json::json!({"mcp":{
            "remote":remote, "local":{"command":"/unexecuted-server", "enabled":false}
        }});
        write_private(
            &directory,
            "mcp.json",
            &serde_json::to_vec(&config).unwrap(),
        );
        let (host, provider) = host_with_capture(&directory, true);
        assert!(host.mcp_authentication().is_some());
        assert!(Arc::ptr_eq(
            &host.mcp_authentication().unwrap(),
            &host
                .mcp_controller()
                .unwrap()
                .authentication_service()
                .unwrap()
        ));
        let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        Self {
            host: Some(Arc::new(host)),
            runtime,
            provider,
            listener,
            directory,
        }
    }

    fn host(&self) -> &Arc<NativeReferenceHost> {
        self.host.as_ref().unwrap()
    }

    fn run(&self, future: impl Future<Output = ()>) {
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), future)
                .await
                .unwrap();
        });
        assert!(self.provider.request_bodies().is_empty());
        assert_eq!(
            self.listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    async fn session(&self) -> NativeInteractiveSession {
        self.session_with_options(|options| options).await
    }

    async fn session_with_options(
        &self,
        select: impl FnOnce(NativeInteractiveSessionOptions) -> NativeInteractiveSessionOptions,
    ) -> NativeInteractiveSession {
        NativeInteractiveSession::open(
            self.host().clone(),
            select(
                NativeInteractiveSessionOptions::new(
                    self.host().workspace_root().to_owned(),
                    self.host().loaded_config().config().model_preferences(),
                )
                .unwrap(),
            ),
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap()
    }

    fn credentials(&self) -> PathBuf {
        self.directory.path().join("profile/mcp-credentials.json")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let host = self.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        mcp_startup::settle(&host, &self.runtime).unwrap();
        drop(host);
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
    }
}

fn write_private(directory: &ScopedTestDirectory, name: &str, bytes: &[u8]) {
    let profile = directory.path().join("profile");
    fs::create_dir_all(&profile).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
    let path = profile.join(name);
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn authenticate(server: &str) -> McpCommand {
    McpCommand::Authenticate {
        server: server.into(),
        open_browser: false,
    }
}

async fn outcome(session: &mut NativeInteractiveSession) -> NativeInteractiveControlOutcome {
    poll_fn(|cx| {
        let _ = session.poll_progress(cx, 2);
        session
            .take_control_outcome()
            .map_or(Poll::Pending, Poll::Ready)
    })
    .await
}

async fn shutdown(mut session: NativeInteractiveSession) {
    session.request_shutdown();
    poll_fn(|cx| {
        let _ = session.poll_progress(cx, 3);
        if session.is_closed() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

fn assert_authentication_idle(fixture: &Fixture) {
    let cleanup = fixture
        .host()
        .mcp_authentication()
        .unwrap()
        .cleanup_status();
    assert_eq!(cleanup.pending_operations, 0);
    assert_eq!(cleanup.pending_workers, 0);
    // A live interactive host retains its service; complete means shut down.
    assert!(!cleanup.complete);
}

#[test]
fn mcp_auth_bare_confirms_disabled_and_failed_remote_without_credentials_or_network() {
    for required in [false, true] {
        let fixture = Fixture::new(true, required);
        let untouched = b"malformed selected credentials must not be loaded by bare auth";
        write_private(&fixture.directory, "mcp-credentials.json", untouched);
        fixture.run(async {
            let (_sender, receiver) = tokio::sync::mpsc::channel(1);
            let mut signals = AskSignals::new(receiver);
            assert_eq!(mcp_startup::activate_interactive(fixture.host(), &mut signals).await.unwrap().is_some(), required);
            let mut session = fixture.session().await;
            assert_authentication_idle(&fixture);
            let observed = management(&mut session, authenticate("remote")).await;
            assert!(!observed.failed());
            assert!(matches!(observed.result,
                Ok(NativeInteractiveControlReceipt::McpAuthentication(NativeMcpAuthenticationReceipt::ConfirmationRequired { server })) if server.as_ref() == "remote"));
            assert_eq!(fs::read(fixture.credentials()).unwrap(), untouched);
            assert_authentication_idle(&fixture);
            assert!(!session.is_closed());
            shutdown(session).await;
        });
    }
}

#[test]
fn mcp_logout_empty_retains_independent_local_and_remote_receipts_without_network() {
    let fixture = Fixture::new(false, false);
    fixture.run(async {
        let mut session = fixture.session().await;
        let observed = management(&mut session, McpCommand::Logout { server: "remote".into() }).await;
        assert!(!observed.failed());
        assert!(matches!(observed.result,
            Ok(NativeInteractiveControlReceipt::McpAuthentication(NativeMcpAuthenticationReceipt::LoggedOut { server, outcome }))
            if server.as_ref() == "remote" && outcome.local == McpAuthLocalRemoval::Unchanged && outcome.remote == McpAuthRemoteRevocation::NotAttempted));
        assert!(!fixture.credentials().exists());
        assert!(fixture.host().mcp_controller().unwrap().required_readiness().is_err(), "logout must not implicitly activate the runtime");
        shutdown(session).await;
    });
}

#[test]
fn mcp_auth_and_logout_reject_unknown_and_stdio_through_actual_control_admission() {
    let fixture = Fixture::new(false, false);
    fixture.run(async {
        let mut session = fixture.session().await;
        for server in ["unknown", "local"] {
            for command in [authenticate(server), McpCommand::Logout { server: server.into() }] {
                let observed = management(&mut session, command).await;
                assert!(matches!(observed.result,
                    Err(NativeInteractiveControlError::McpAuthentication(NativeMcpAuthenticationError::Selection(error))) if error.kind() == NativeMcpControllerError::Invalid));
                assert!(!session.is_closed());
            }
        }
        assert!(!fixture.credentials().exists());
        shutdown(session).await;
    });
}

#[test]
fn mcp_auth_controls_observe_cancellation_and_actual_conversation_quiescence() {
    let fixture = Fixture::new(false, false);
    fixture.run(async {
        let mut session = fixture.session().await;
        for command in [
            authenticate("remote"),
            McpCommand::Logout {
                server: "remote".into(),
            },
        ] {
            session
                .request_control(NativeInteractiveControl::Mcp { command }, 2)
                .unwrap();
            assert!(session.request_cancel());
            assert!(matches!(
                outcome(&mut session).await.result,
                Err(NativeInteractiveControlError::McpAuthentication(
                    NativeMcpAuthenticationError::Authorization(McpAuthError::Cancelled)
                ))
            ));
        }
        session
            .request_control(
                NativeInteractiveControl::Mcp {
                    command: authenticate("remote"),
                },
                2,
            )
            .unwrap();
        let quiescence = session.runtime().begin_quiescence().unwrap();
        assert!(matches!(
            outcome(&mut session).await.result,
            Err(NativeInteractiveControlError::Runtime(
                NativeConversationRuntimeError::Quiescing
            ))
        ));
        assert!(quiescence.selection_snapshot().is_ok());
        drop(quiescence);
        assert!(
            !management(&mut session, authenticate("remote"))
                .await
                .failed()
        );
        assert!(!fixture.credentials().exists());
        shutdown(session).await;
    });
}
