use super::*;
use crate::mcp::interaction::{
    McpClientUrlCompletion, McpClientUrlCompletionObserver, McpClientUrlEndpoint,
    McpClientUrlOutcome, McpElicitationAnswer, McpElicitationPresenter, McpElicitationPromptError,
    McpElicitationPromptRequest,
};
use std::result::Result;

type ClientFixture = (
    Fixture,
    NativeInteractivePromptInbox,
    Arc<Mutex<Vec<McpClientUrlOutcome>>>,
);

struct Client {
    bridge: Arc<NativeInteractivePromptBridge>,
    finished: Arc<Mutex<Vec<McpClientUrlOutcome>>>,
}
struct Completion(Arc<Mutex<Vec<McpClientUrlOutcome>>>);
impl McpClientUrlCompletionObserver for Completion {
    fn finish(self: Box<Self>, outcome: McpClientUrlOutcome) {
        self.0.lock().unwrap().push(outcome);
    }
}
impl McpClientUrlEndpoint for Client {
    fn register(
        &self,
        request: &McpElicitationPromptRequest,
    ) -> Result<McpClientUrlCompletion, McpElicitationPromptError> {
        assert_eq!(request.request().url(), Some("https://example.test/auth"));
        Ok(McpClientUrlCompletion::new(Box::new(Completion(
            self.finished.clone(),
        ))))
    }
}
impl McpElicitationPresenter for Client {
    fn client_urls(&self) -> Option<&dyn McpClientUrlEndpoint> {
        Some(self)
    }
    fn present(
        &self,
        request: McpElicitationPromptRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpElicitationAnswer, McpElicitationPromptError>> {
        Box::pin(async move {
            assert!(self.finished.lock().unwrap().is_empty());
            let answer = self.bridge.present(request, cancellation).await;
            // Accepting a URL is not completion of its native continuation.
            assert!(self.finished.lock().unwrap().is_empty());
            answer
        })
    }
}

fn configured_client(archive: &Archive) -> ClientFixture {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let bridge = inbox.router();
    let principal = inbox
        .register(BackgroundOutputOwner::new(
            SessionId::new("runtime").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        ))
        .unwrap();
    let finished = Arc::new(Mutex::new(Vec::new()));
    let client = Arc::new(Client {
        bridge,
        finished: finished.clone(),
    });
    let executor = Arc::new(
        NativeMcpArchivedToolExecutor::new(Arc::new(NativeToolResultArchiveAdapter::new(
            archive.storage.clone(),
        )))
        .unwrap()
        .with_form_responder(client),
    );
    // No local browser authority has been supplied at all.
    assert!(executor.execution_policy().url);
    let mut fixture = Fixture::with_executor(
        &[json!({})],
        PermissionMode::Auto,
        executor.clone(),
        executor.execution_policy(),
        false,
        |runtime, writes| {
            scripted_candidate(
                runtime,
                writes,
                &json!({"name":"lookup","inputSchema":{"type":"object"}}),
                |id| {
                    if id == 1 {
                        envelope(id, url::URL)
                    } else {
                        // Exercise actual archive publication, not the
                        // small-result inline path, before completion.
                        envelope(
                            id,
                            &format!(
                                r#""result":{{"resultType":"complete","content":[{{"type":"text","text":"{}"}}]}}"#,
                                "x".repeat(70_000),
                            ),
                        )
                    }
                },
            )
        },
    );
    fixture._prompt_principal = Some(principal);
    (fixture, inbox, finished)
}

#[test]
fn client_url_finishes_after_continuation_and_archive_without_local_browser() {
    let archive = Archive::new();
    let (fixture, mut inbox, finished) = configured_client(&archive);
    let (events, prompts) = run_answered(&fixture, &mut inbox, r#"{"action":"accept"}"#);
    assert_eq!(prompts, 1);
    assert!(!result(&events).is_error);
    let stored = persisted(&record(&fixture), "call-0");
    assert_eq!(archive.read(&stored.content), *result(&events));
    assert_eq!(*finished.lock().unwrap(), [McpClientUrlOutcome::Completed]);
    assert_eq!(wires(&fixture).len(), 2);
}

#[test]
fn declined_client_url_invalidates_registration_instead_of_reporting_completion() {
    for action in ["decline", "cancel"] {
        let archive = Archive::new();
        let (fixture, mut inbox, finished) = configured_client(&archive);
        let (_, prompts) =
            run_answered(&fixture, &mut inbox, &format!(r#"{{"action":"{action}"}}"#));
        assert_eq!(prompts, 1);
        assert_eq!(*finished.lock().unwrap(), [McpClientUrlOutcome::Abandoned]);
    }
}

#[test]
fn cancellation_while_client_is_answering_abandons_exact_registration() {
    let archive = Archive::new();
    let (fixture, mut inbox, finished) = configured_client(&archive);
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut future = Box::pin(turn.collect::<Vec<_>>());
    let view = queued_prompt(&mut future, &mut inbox);
    assert!(view.elicitation().is_some());
    assert!(finished.lock().unwrap().is_empty());
    drop(future);
    assert_eq!(*finished.lock().unwrap(), [McpClientUrlOutcome::Abandoned]);
}
