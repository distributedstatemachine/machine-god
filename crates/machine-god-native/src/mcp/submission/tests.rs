use super::*;
use futures_executor::block_on;
use futures_util::task::noop_waker;
use machine_god_core::*;
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider};
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::task::{Wake, Waker};

use crate::{
    NativeConfiguredPermissionRules, NativePermissionActionPreparer,
    NativePermissionAutomaticOutcome, NativePermissionConfiguredOutcome,
    NativePermissionController, NativePermissionPolicySnapshot, NativePermissionRuleKey,
    NativePermissionSession, NativePermissionTurn, NativePreparedPermissionAction, PermissionMode,
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "http/tests.rs"]
mod http_tests;

#[path = "tool/tests.rs"]
mod tool_tests;

struct Adapter(Mutex<Option<PreparedMcpSubmission>>);
impl NativePermissionActionPreparer for Adapter {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Box<dyn NativePreparedPermissionAction>, PermissionError>>
    {
        Box::pin(async move {
            Ok(Box::new(Action(self.0.lock().unwrap().take().unwrap()))
                as Box<dyn NativePreparedPermissionAction>)
        })
    }
}
struct Action(PreparedMcpSubmission);
impl NativePreparedPermissionAction for Action {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        None
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        None
    }
    fn is_file_mutation(&self) -> bool {
        false
    }
    fn configured_outcome(
        &self,
        _: &NativeConfiguredPermissionRules,
    ) -> std::result::Result<NativePermissionConfiguredOutcome, PermissionError> {
        Ok(NativePermissionConfiguredOutcome::Unresolved)
    }
    fn allows_without_review(&self, _: PermissionMode) -> bool {
        false
    }
    fn automatic_review(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async { Ok(NativePermissionAutomaticOutcome::Ask) })
    }
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> std::result::Result<Box<dyn PermissionExecutionAdmission>, PermissionError> {
        Ok(Box::new(self.0.bind_execution(proof)))
    }
}
struct Prompter;
impl PermissionPrompter for Prompter {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, std::result::Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async { Ok(PermissionPromptDecision::AllowOnce) })
    }
}

pub(crate) struct Fixture {
    _engine: Engine,
    session: Session,
    turn: Turn,
    owner: Arc<NativePermissionSession>,
    _permission_turn: NativePermissionTurn,
    registry: Arc<McpSubmissionRegistry>,
    registration: Option<McpSubmissionTurnRegistration>,
    runtime_owner: McpSubmissionRuntimeOwner,
    pub(crate) runtime: Arc<McpSubmissionRuntime>,
    controller: Arc<NativePermissionController>,
    adapter: Arc<Adapter>,
}
fn binding() -> McpSubmissionRuntimeBinding {
    McpSubmissionRuntimeBinding::new(
        "secret-server",
        ToolName::new("mcp_secret_tool").unwrap(),
        "secret-tool",
        b"secret-config",
        b"secret-schema",
        b"secret-auth",
    )
    .unwrap()
}
impl Fixture {
    pub(crate) fn revoke(&self) {
        self.owner.reset().unwrap();
    }
    pub(crate) fn turn_handle(&self) -> TurnHandle {
        self.turn.handle()
    }
    pub(crate) fn close(&mut self) {
        drop(self.registration.take());
    }
    pub(crate) fn new() -> Self {
        let adapter = Arc::new(Adapter(Mutex::new(None)));
        let controller = Arc::new(NativePermissionController::new(
            adapter.clone(),
            Arc::new(Prompter),
        ));
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::default())
            .provider(ScriptedModelProvider::new("test", []))
            .shared_permission_handler(controller.clone())
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("session").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        let owner = controller
            .register(
                session.clone(),
                NativePermissionPolicySnapshot::new(PermissionMode::Ask, Arc::default()),
            )
            .unwrap();
        let turn = block_on(session.prompt("request")).unwrap();
        let permission_turn = owner.begin_turn(&turn, owner.snapshot().unwrap()).unwrap();
        let (registry, registration) =
            McpSubmissionRegistry::register_turn(&session, &turn).unwrap();
        let runtime_owner = McpSubmissionRuntimeOwner::new();
        let runtime = runtime_owner.install(binding()).unwrap();
        Self {
            _engine: engine,
            session,
            turn,
            owner,
            _permission_turn: permission_turn,
            registry,
            registration: Some(registration),
            runtime_owner,
            runtime,
            controller,
            adapter,
        }
    }
    fn request(&self, call: &str) -> PermissionRequest {
        PermissionRequest {
            id: PermissionRequestId::new(format!("permission-{call}")).unwrap(),
            session_id: self.session.id(),
            session_incarnation_id: self.session.incarnation_id(),
            turn_id: self.turn.id().clone(),
            capability: Capability::Tool {
                name: self.tool(),
                call_id: ToolCallId::new(call).unwrap(),
                arguments: json!({"secret": 1}),
            },
            risk: PermissionRisk::Low,
            reason: "secret-reason".to_owned(),
        }
    }
    fn tool(&self) -> ToolName {
        self.runtime.binding.tool_name().clone()
    }
    fn context(&self, call: &str) -> ToolContext {
        ToolContext {
            session_id: self.session.id(),
            session_incarnation_id: self.session.incarnation_id(),
            turn_id: self.turn.id().clone(),
            call_id: ToolCallId::new(call).unwrap(),
        }
    }
    fn wire(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":"rpc-secret", "method":"tools/call", "params":{"name":self.runtime.binding.remote_tool(), "arguments":{"secret":1}}})).unwrap()
    }
    fn prepare_request(
        &self,
        request: &PermissionRequest,
        wire: &[u8],
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedMcpSubmission>> {
        let Capability::Tool {
            name,
            call_id,
            arguments,
        } = &request.capability
        else {
            panic!()
        };
        self.registry.prepare(
            request,
            PermissionInvocation {
                tool_name: name,
                call_id,
                arguments,
            },
            self.runtime.clone(),
            wire,
            cancellation,
        )
    }
    fn prepare(&self, call: &str, cancellation: CancellationToken) -> PreparedMcpSubmission {
        block_on(self.prepare_request(&self.request(call), &self.wire(), cancellation)).unwrap()
    }
    fn admission(
        &self,
        call: &str,
        prepared: PreparedMcpSubmission,
    ) -> Box<dyn PermissionExecutionAdmission> {
        *self.adapter.0.lock().unwrap() = Some(prepared);
        let request = self.request(call);
        let Capability::Tool {
            name,
            call_id,
            arguments,
        } = &request.capability
        else {
            panic!()
        };
        block_on(self.controller.authorize_invocation(
            request.clone(),
            PermissionInvocation {
                tool_name: name,
                call_id,
                arguments,
            },
        ))
        .unwrap()
        .admission
        .unwrap()
    }
    pub(crate) fn ready(&self, call: &str) {
        self.admission(call, self.prepare(call, CancellationToken::new()))
            .admit()
            .unwrap();
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn ready_http(&self, call: &str, head: &McpSubmissionHttpHead) {
        self.ready_http_with_id(call, head, RpcId::String("rpc-secret".into()));
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn ready_http_with_id(&self, call: &str, head: &McpSubmissionHttpHead, id: RpcId) {
        self.admission(call, self.prepare_http_with_id(call, head, id))
            .admit()
            .unwrap();
    }
    #[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn ready_http_with_reservation(
        &self,
        call: &str,
        head: &McpSubmissionHttpHead,
        lease: McpToolReservation,
    ) {
        let mut prepared = self.prepare_http_with_id(call, head, lease.rpc_id().clone());
        prepared.data.tool_reservation = Some(lease);
        self.admission(call, prepared).admit().unwrap();
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn prepare_http_with_id(
        &self,
        call: &str,
        head: &McpSubmissionHttpHead,
        id: RpcId,
    ) -> PreparedMcpSubmission {
        let request = self.request(call);
        let Capability::Tool {
            name,
            call_id,
            arguments,
        } = &request.capability
        else {
            panic!()
        };
        let mut wire: Value = serde_json::from_slice(&self.wire()).unwrap();
        wire["id"] = match id {
            RpcId::Integer(id) => Value::from(id),
            RpcId::String(id) => Value::String(id),
            RpcId::Null => panic!("tool fixture requires a request ID"),
        };
        let wire = serde_json::to_vec(&wire).unwrap();
        block_on(self.registry.prepare_http(
            &request,
            PermissionInvocation {
                tool_name: name,
                call_id,
                arguments,
            },
            self.runtime.clone(),
            head,
            &wire,
            CancellationToken::new(),
        ))
        .unwrap()
    }
    pub(crate) fn claim(
        &self,
        call: &str,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<McpSubmission>> {
        self.registry.claim(
            self.context(call),
            &self.tool(),
            &json!({"secret":1}),
            self.runtime.clone(),
            cancellation,
        )
    }
}

#[test]
fn preparation_and_claim_are_inert_before_poll_and_precancelled() {
    let fixture = Fixture::new();
    drop(fixture.prepare_request(
        &fixture.request("call"),
        &fixture.wire(),
        CancellationToken::new(),
    ));
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        block_on(fixture.prepare_request(
            &fixture.request("call"),
            &fixture.wire(),
            cancellation.clone()
        )),
        Err(McpSubmissionError::Cancelled)
    ));
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    fixture.ready("call");
    drop(fixture.claim("call", CancellationToken::new()));
    assert!(matches!(
        block_on(fixture.claim("call", cancellation)),
        Err(McpSubmissionError::Cancelled)
    ));
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_ok());
}

#[test]
fn reservation_ready_claimed_are_one_shot_and_request_ids_cannot_alias() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare("call", CancellationToken::new());
    assert!(matches!(
        block_on(fixture.prepare_request(
            &fixture.request("call"),
            &fixture.wire(),
            CancellationToken::new()
        )),
        Err(McpSubmissionError::Duplicate)
    ));
    let mut same_permission = fixture.request("different-call");
    same_permission.id = fixture.request("call").id;
    assert!(matches!(
        block_on(fixture.prepare_request(
            &same_permission,
            &fixture.wire(),
            CancellationToken::new()
        )),
        Err(McpSubmissionError::Duplicate)
    ));
    let before_admission = fixture.claim("call", CancellationToken::new());
    fixture.admission("call", prepared).admit().unwrap();
    assert!(matches!(
        block_on(before_admission),
        Err(McpSubmissionError::Denied)
    ));
    let first = fixture.claim("call", CancellationToken::new());
    let second = fixture.claim("call", CancellationToken::new());
    drop(block_on(first).unwrap());
    assert!(matches!(block_on(second), Err(McpSubmissionError::Denied)));
    assert!(matches!(
        block_on(fixture.prepare_request(
            &fixture.request("call"),
            &fixture.wire(),
            CancellationToken::new()
        )),
        Err(McpSubmissionError::Duplicate)
    ));
    assert!(matches!(
        block_on(fixture.prepare_request(
            &same_permission,
            &fixture.wire(),
            CancellationToken::new()
        )),
        Err(McpSubmissionError::Duplicate)
    ));
}

#[test]
fn dropped_reservation_is_replaceable_but_old_missing_ticket_stays_missing() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare("call", CancellationToken::new());
    let generation = prepared.data.reservation.generation;
    let old = fixture.claim("call", CancellationToken::new());
    drop(prepared);
    let replacement = fixture.prepare("call", CancellationToken::new());
    assert!(replacement.data.reservation.generation > generation);
    fixture.admission("call", replacement).admit().unwrap();
    assert!(matches!(block_on(old), Err(McpSubmissionError::Denied)));
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_ok());
}

#[test]
fn captured_ready_generation_cannot_consume_a_replacement_slot() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let stale = fixture.claim("call", CancellationToken::new());
    // White-box replacement exercises the anti-ABA stamp. Public operations
    // cannot erase a ready slot without also invalidating the whole scope.
    let removed = fixture
        .registry
        .state
        .lock()
        .unwrap()
        .slots
        .remove(&ToolCallId::new("call").unwrap());
    drop(removed);
    fixture.ready("call");
    assert!(matches!(block_on(stale), Err(McpSubmissionError::Denied)));
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_ok());
}

#[test]
fn admission_drop_releases_reservation_and_revocation_prevents_publication() {
    let fixture = Fixture::new();
    drop(fixture.admission("call", fixture.prepare("call", CancellationToken::new())));
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    let admission = fixture.admission("call", fixture.prepare("call", CancellationToken::new()));
    fixture.owner.reset().unwrap();
    assert!(admission.admit().is_err());
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
}

#[test]
fn all_context_fields_tool_arguments_and_runtime_allocation_are_exact() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let original = fixture.context("call");
    for index in 0..4 {
        let mut context = original.clone();
        match index {
            0 => context.session_id = SessionId::new("foreign").unwrap(),
            1 => context.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap(),
            2 => context.turn_id = TurnId::new("foreign").unwrap(),
            _ => context.call_id = ToolCallId::new("foreign").unwrap(),
        }
        assert!(
            block_on(fixture.registry.claim(
                context,
                &fixture.tool(),
                &json!({"secret":1}),
                fixture.runtime.clone(),
                CancellationToken::new()
            ))
            .is_err()
        );
    }
    assert!(
        block_on(fixture.registry.claim(
            original.clone(),
            &ToolName::new("foreign").unwrap(),
            &json!({"secret":1}),
            fixture.runtime.clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    assert!(
        block_on(fixture.registry.claim(
            original.clone(),
            &fixture.tool(),
            &json!({"secret":2}),
            fixture.runtime.clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    let other_owner = McpSubmissionRuntimeOwner::new();
    let other = other_owner.install(binding()).unwrap();
    assert_eq!(other.generation(), fixture.runtime.generation());
    assert!(
        block_on(fixture.registry.claim(
            original,
            &fixture.tool(),
            &json!({"secret":1}),
            other,
            CancellationToken::new()
        ))
        .is_err()
    );
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_ok());
}

#[test]
fn submission_runtime_membership_is_allocation_identity_not_a_grant() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert!(submission.belongs_to_runtime(&fixture.runtime));
    assert!(submission.belongs_to_runtime(&fixture.runtime.clone()));
    let other_owner = McpSubmissionRuntimeOwner::new();
    let other = other_owner.install(binding()).unwrap();
    assert_eq!(other.generation(), fixture.runtime.generation());
    assert!(!submission.belongs_to_runtime(&other));
    fixture.runtime_owner.retire();
    assert!(submission.belongs_to_runtime(&fixture.runtime));
    let mut writer = submission.into_writer(Writer(Arc::default()));
    assert!(matches!(poll(&mut writer), Poll::Ready(Err(_))));
    assert!(!writer.was_attempted());
}

#[test]
fn request_semantics_duplicate_keys_control_methods_and_framing_fail_closed() {
    let fixture = Fixture::new();
    let wire = fixture.wire();
    for invalid in [
        String::from_utf8(wire.clone())
            .unwrap()
            .replace("tools/call", "initialize"),
        String::from_utf8(wire.clone())
            .unwrap()
            .replace("secret-tool", "foreign"),
        String::from_utf8(wire.clone())
            .unwrap()
            .replace("\"secret\":1", "\"secret\":2"),
        String::from_utf8(wire.clone())
            .unwrap()
            .replace("\"secret\":1", "\"secret\":1,\"secret\":1"),
        format!("{}\n", String::from_utf8(wire).unwrap()),
    ] {
        assert!(
            block_on(fixture.prepare_request(
                &fixture.request("call"),
                invalid.as_bytes(),
                CancellationToken::new()
            ))
            .is_err()
        );
    }
    let original = fixture.request("call");
    let Capability::Tool {
        name,
        call_id,
        arguments,
    } = &original.capability
    else {
        panic!()
    };
    for index in 0..6 {
        let mut request = original.clone();
        match index {
            0 => request.session_id = SessionId::new("foreign").unwrap(),
            1 => request.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap(),
            2 => request.turn_id = TurnId::new("foreign").unwrap(),
            3 => {
                request.capability = Capability::Tool {
                    name: name.clone(),
                    call_id: ToolCallId::new("foreign").unwrap(),
                    arguments: arguments.clone(),
                }
            }
            4 => {
                request.capability = Capability::Tool {
                    name: ToolName::new("foreign").unwrap(),
                    call_id: call_id.clone(),
                    arguments: arguments.clone(),
                }
            }
            _ => {
                request.capability = Capability::Tool {
                    name: name.clone(),
                    call_id: call_id.clone(),
                    arguments: json!({"secret":2}),
                }
            }
        }
        assert!(
            block_on(fixture.registry.prepare(
                &request,
                PermissionInvocation {
                    tool_name: name,
                    call_id,
                    arguments
                },
                fixture.runtime.clone(),
                &fixture.wire(),
                CancellationToken::new()
            ))
            .is_err()
        );
    }
}

#[derive(Default)]
struct WriterState {
    bytes: Vec<u8>,
    calls: usize,
    flushes: usize,
    pending: bool,
    fail: bool,
    invalid_count: bool,
    chunk: usize,
}
struct Writer(Arc<Mutex<WriterState>>);
impl McpSubmissionWriter for Writer {
    fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        let mut state = self.0.lock().unwrap();
        state.calls += 1;
        if state.fail {
            return Poll::Ready(Err(io::Error::other("secret writer failure")));
        }
        if state.pending {
            return Poll::Pending;
        }
        if state.invalid_count {
            return Poll::Ready(Ok(bytes.len() + 1));
        }
        let count = if state.chunk == 0 {
            bytes.len()
        } else {
            state.chunk.min(bytes.len())
        };
        state.bytes.extend_from_slice(&bytes[..count]);
        Poll::Ready(Ok(count))
    }
    fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut state = self.0.lock().unwrap();
        state.flushes += 1;
        Poll::Ready(Ok(()))
    }
}
fn poll<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(&noop_waker()))
}

#[test]
fn partial_writer_retains_exact_bytes_and_one_final_newline_without_replay() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert_eq!(submission.rpc_id(), &RpcId::String("rpc-secret".to_owned()));
    assert_eq!(
        submission.permission_request_id(),
        &fixture.request("call").id
    );
    let state = Arc::new(Mutex::new(WriterState {
        chunk: 3,
        ..WriterState::default()
    }));
    let mut writer = submission.into_writer(Writer(state.clone()));
    assert!(!writer.was_attempted());
    assert_eq!(state.lock().unwrap().calls, 0);
    block_on(&mut writer).unwrap();
    let mut expected = fixture.wire();
    expected.push(b'\n');
    assert_eq!(state.lock().unwrap().bytes, expected);
    assert_eq!(state.lock().unwrap().flushes, 1);
    assert_eq!(writer.acknowledged_bytes(), expected.len());
    assert!(writer.was_attempted());
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
    );
}

#[test]
fn writer_failure_pending_and_invalid_counts_never_enable_full_request_replay() {
    for invalid_count in [false, true] {
        let fixture = Fixture::new();
        fixture.ready("call");
        let state = Arc::new(Mutex::new(WriterState {
            fail: !invalid_count,
            invalid_count,
            ..WriterState::default()
        }));
        let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
            .unwrap()
            .into_writer(Writer(state.clone()));
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::WriterFailed))
        );
        assert!(writer.was_attempted());
        assert_eq!(writer.acknowledged_bytes(), 0);
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
        );
        assert_eq!(state.lock().unwrap().calls, 1);
    }
    let fixture = Fixture::new();
    fixture.ready("call");
    let state = Arc::new(Mutex::new(WriterState {
        pending: true,
        ..WriterState::default()
    }));
    let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
        .unwrap()
        .into_writer(Writer(state.clone()));
    assert!(poll(&mut writer).is_pending());
    assert!(writer.was_attempted());
    fixture.owner.reset().unwrap();
    assert!(matches!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::Denied))
    ));
    assert_eq!(state.lock().unwrap().calls, 1);
}

#[test]
fn final_checkpoint_revalidates_before_first_write_each_suffix_and_flush() {
    for stage in 0..3 {
        let fixture = Fixture::new();
        fixture.ready("call");
        let state = Arc::new(Mutex::new(WriterState {
            chunk: usize::from(stage == 1),
            ..WriterState::default()
        }));
        let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
            .unwrap()
            .into_writer(Writer(state.clone()));
        if stage > 0 {
            assert!(poll(&mut writer).is_pending());
        }
        let calls = state.lock().unwrap().calls;
        fixture.owner.reset().unwrap();
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::Denied))
        );
        assert_eq!(state.lock().unwrap().calls, calls);
        assert_eq!(state.lock().unwrap().flushes, 0);
    }
}

#[test]
fn writer_panic_is_attempted_and_terminal() {
    struct PanicWriter;
    impl McpSubmissionWriter for PanicWriter {
        fn poll_write(&mut self, _: &mut Context<'_>, _: &[u8]) -> Poll<io::Result<usize>> {
            panic!("writer panic")
        }
        fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            unreachable!()
        }
    }
    let fixture = Fixture::new();
    fixture.ready("call");
    let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
        .unwrap()
        .into_writer(PanicWriter);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll(&mut writer))).is_err());
    assert!(writer.was_attempted());
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
    );
}

#[test]
fn cancellation_during_successful_flush_wins_without_replaying_acknowledged_bytes() {
    struct CancelOnFlush(CancellationToken);
    impl McpSubmissionWriter for CancelOnFlush {
        fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.0.cancel();
            Poll::Ready(Ok(()))
        }
    }
    let fixture = Fixture::new();
    fixture.ready("call");
    let cancellation = CancellationToken::new();
    let mut writer = block_on(fixture.claim("call", cancellation.clone()))
        .unwrap()
        .into_writer(CancelOnFlush(cancellation));
    assert!(poll(&mut writer).is_pending());
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::Cancelled))
    );
    assert_eq!(writer.acknowledged_bytes(), fixture.wire().len() + 1);
    assert!(writer.was_attempted());
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
    );
}

#[test]
fn bare_core_turn_cancellation_wakes_an_independent_pending_writer() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let state = Arc::new(Mutex::new(WriterState {
        pending: true,
        ..WriterState::default()
    }));
    let independent = CancellationToken::new();
    let mut writer = block_on(fixture.claim("call", independent.clone()))
        .unwrap()
        .into_writer(Writer(state.clone()));
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    assert!(
        Pin::new(&mut writer)
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert!(fixture.turn.handle().cancel());
    assert!(!independent.is_cancelled());
    assert!(counter.0.load(Ordering::SeqCst) > 0);
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::Cancelled))
    );
    assert_eq!(state.lock().unwrap().calls, 1);
}

struct Counter(AtomicUsize);
impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn registry_observer_does_not_retain_registry_and_observes_both_closures() {
    for core_turn in [false, true] {
        let fixture = Fixture::new();
        let weak = Arc::downgrade(&fixture.registry);
        let count = Arc::strong_count(&fixture.registry);
        let mut waiting = std::pin::pin!(fixture.registry.cancelled_owned());
        assert_eq!(Arc::strong_count(&fixture.registry), count);
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert!(fixture.registry.revalidate().is_ok());
        if core_turn {
            assert!(fixture.turn.handle().cancel());
        } else {
            fixture.registry.retire();
            fixture.registry.retire();
        }
        assert_eq!(
            fixture.registry.revalidate(),
            Err(McpSubmissionError::Cancelled)
        );
        assert!(counter.0.load(Ordering::SeqCst) > 0);
        drop(fixture);
        assert!(weak.upgrade().is_none());
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
    }
}

#[test]
fn unpolled_registry_observer_sees_retirement_with_retained_registration() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let mut waiting = std::pin::pin!(fixture.registry.cancelled_owned());
    fixture.registry.retire();
    assert!(fixture.registration.is_some());
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(&noop_waker()))
            .is_ready()
    );
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_err());
}

#[test]
fn owned_response_cancellation_survives_consumed_submission() {
    for source in 0..5 {
        let mut fixture = Fixture::new();
        let preparation = CancellationToken::new();
        fixture
            .admission("call", fixture.prepare("call", preparation.clone()))
            .admit()
            .unwrap();
        let execution = CancellationToken::new();
        let submission = block_on(fixture.claim("call", execution.clone())).unwrap();
        let mut waiting = submission.cancelled_owned();
        drop(submission);
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        match source {
            0 => {
                execution.cancel();
            }
            1 => {
                preparation.cancel();
            }
            2 => {
                drop(fixture.registration.take());
            }
            3 => {
                fixture.runtime_owner.retire();
            }
            _ => {
                assert!(fixture.turn.handle().cancel());
            }
        }
        assert!(counter.0.load(Ordering::SeqCst) > 0);
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
    }
}

#[test]
fn cancellation_and_retirement_wake_queue_and_pending_writer_and_clear_scope() {
    for source in 0..5 {
        let mut fixture = Fixture::new();
        let preparation = CancellationToken::new();
        fixture
            .admission("call", fixture.prepare("call", preparation.clone()))
            .admit()
            .unwrap();
        let execution = CancellationToken::new();
        let submission = block_on(fixture.claim("call", execution.clone())).unwrap();
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        {
            let mut waiting = submission.cancelled();
            assert!(
                waiting
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            match source {
                0 => {
                    execution.cancel();
                }
                1 => {
                    preparation.cancel();
                }
                2 => {
                    drop(fixture.registration.take());
                }
                3 => {
                    fixture.runtime_owner.retire();
                }
                _ => {
                    assert!(fixture.turn.handle().cancel());
                }
            }
            assert!(counter.0.load(Ordering::SeqCst) > 0);
            assert!(
                waiting
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_ready()
            );
        }
        let state = Arc::new(Mutex::new(WriterState::default()));
        let mut writer = submission.into_writer(Writer(state.clone()));
        assert!(matches!(poll(&mut writer), Poll::Ready(Err(_))));
        assert!(!writer.was_attempted());
        assert_eq!(state.lock().unwrap().calls, 0);
        if source == 2 {
            assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
        }
    }
    let fixture = Fixture::new();
    fixture.ready("call");
    let state = Arc::new(Mutex::new(WriterState {
        pending: true,
        ..WriterState::default()
    }));
    let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
        .unwrap()
        .into_writer(Writer(state));
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    assert!(
        Pin::new(&mut writer)
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    fixture.runtime_owner.retire();
    assert!(counter.0.load(Ordering::SeqCst) > 0);
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::Cancelled))
    );
}

#[test]
fn closure_cancels_admitted_unpolled_claims_and_reserved_admission() {
    let mut fixture = Fixture::new();
    fixture.ready("call");
    let future = fixture.claim("call", CancellationToken::new());
    let admission = fixture.admission(
        "reserved",
        fixture.prepare("reserved", CancellationToken::new()),
    );
    drop(fixture.registration.take());
    assert!(matches!(
        block_on(future),
        Err(McpSubmissionError::Cancelled)
    ));
    assert!(admission.admit().is_err());
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    assert!(
        block_on(fixture.prepare_request(
            &fixture.request("later"),
            &fixture.wire(),
            CancellationToken::new()
        ))
        .is_err()
    );
}

#[test]
fn limits_generation_exhaustion_and_runtime_replacement_are_terminal_and_bounded() {
    let fixture = Fixture::new();
    fixture.registry.state.lock().unwrap().next_generation = Some(u64::MAX);
    drop(fixture.prepare("last", CancellationToken::new()));
    assert!(matches!(
        block_on(fixture.prepare_request(
            &fixture.request("exhausted"),
            &fixture.wire(),
            CancellationToken::new()
        )),
        Err(McpSubmissionError::GenerationExhausted)
    ));
    let fixture = Fixture::new();
    let mut reserved = Vec::new();
    for index in 0..MAX_MCP_SUBMISSION_SLOTS {
        reserved.push(fixture.prepare(&format!("call-{index}"), CancellationToken::new()));
    }
    assert!(matches!(
        block_on(fixture.prepare_request(
            &fixture.request("over"),
            &fixture.wire(),
            CancellationToken::new()
        )),
        Err(McpSubmissionError::Limit)
    ));
    drop(reserved);
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    assert!(matches!(
        canonical_arguments(&json!("a".repeat(MAX_MCP_SUBMISSION_ARGUMENT_BYTES))),
        Err(McpSubmissionError::Limit)
    ));
    assert!(matches!(
        canonical_arguments(&Value::Array(vec![Value::Null; MAX_ARGUMENT_NODES])),
        Err(McpSubmissionError::Limit)
    ));
    assert!(matches!(
        McpSubmissionRuntimeBinding::new(
            "server",
            fixture.tool(),
            "tool",
            &vec![0; MAX_MCP_SUBMISSION_BINDING_BYTES],
            b"",
            b""
        ),
        Err(McpSubmissionError::Limit)
    ));
    let old = fixture.runtime.clone();
    let replacement = fixture.runtime_owner.install(binding()).unwrap();
    assert!(replacement.generation() > old.generation());
    assert!(old.live().is_err());
    assert!(replacement.live().is_ok());
}

#[test]
fn redacted_debug_and_errors_do_not_disclose_any_bound_content() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare("call", CancellationToken::new());
    for text in [
        format!("{prepared:?}"),
        format!("{:?}", fixture.runtime),
        format!("{:?}", binding()),
        format!("{:?}", fixture.registry),
        format!("{:?}", fixture.runtime_owner),
        format!("{}", McpSubmissionError::WriterFailed),
    ] {
        assert!(!text.contains("secret"));
    }
}

#[test]
fn scope_close_and_runtime_retirement_wake_reentrant_callbacks_outside_locks() {
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    for runtime in [false, true] {
        let mut fixture = Fixture::new();
        fixture.ready("call");
        let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
        let registry = fixture.registry.clone();
        let (waker, state) = reentrant_waker(Callback::Wake, move || {
            assert!(registry.state.try_lock().is_ok());
        });
        let mut waiting = submission.cancelled();
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        if runtime {
            fixture.runtime_owner.retire();
        } else {
            drop(fixture.registration.take());
        }
        assert!(state.calls() > 0);
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
    }
}
