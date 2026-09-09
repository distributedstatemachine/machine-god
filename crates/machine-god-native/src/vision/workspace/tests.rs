use super::*;
use crate::vision::*;
use crate::vision_portable::{VisionBatchResponse, VisionImageResult};
use crate::{
    NativeConversation, NativeConversationTurn, NativeWorkspaceAuthority, NativeWorkspaceEntrySpec,
    NativeWorkspaceSource,
};
use futures_executor::block_on;
use machine_god_core::{
    Engine, ModelEvent, PermissionDecision, PermissionError, PermissionGrantScope,
    PermissionHandler, PermissionRequest, SessionId, SessionIncarnationId, SessionRecord,
    StopReason, ToolCallId,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use std::path::PathBuf;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    primary: PathBuf,
    additional: PathBuf,
    authority: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-vision-scope-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let additional = base.join("additional");
        std::fs::create_dir(&primary).unwrap();
        std::fs::create_dir(&additional).unwrap();
        let root =
            rustix::fs::open(&primary, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty()).unwrap();
        let authority = NativeWorkspaceAuthority::open_blocking(
            root,
            primary.clone(),
            None,
            base.join("state"),
            vec![
                NativeWorkspaceEntrySpec::new(
                    NativeWorkspaceSource::new(additional.clone(), additional.clone(), true)
                        .unwrap(),
                    true,
                    false,
                )
                .unwrap(),
            ],
            false,
        )
        .unwrap();
        Self {
            base,
            primary,
            additional,
            authority,
            contexts: Arc::new(NativeWorkspaceContexts::new()),
        }
    }
    fn tool(&self, transport: Arc<Transport>) -> VisionTool {
        VisionTool::with_transport(&self.primary, target(), transport, Arc::new(NeverDeadline))
            .unwrap()
            .with_workspace_contexts(self.contexts.clone())
    }
    fn conversation(&self) -> NativeConversation {
        let mut record = SessionRecord::empty(
            SessionId::new("vision-session").unwrap(),
            SessionIncarnationId::new("vision-incarnation").unwrap(),
        );
        record.revision = machine_god_core::SessionRevision(1);
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
                record.id.clone(),
                record,
            )])))
            .provider(ScriptedModelProvider::new(
                "test",
                [ModelProviderStep::events([ModelEvent::Stop {
                    reason: StopReason::Completed,
                }])],
            ))
            .permission_handler(Allow)
            .build()
            .unwrap();
        let session = block_on(engine.load_session(SessionId::new("vision-session").unwrap()))
            .unwrap()
            .unwrap();
        NativeConversation::from_session(session)
            .unwrap()
            .with_workspace_contexts(self.authority.clone(), &self.contexts)
            .unwrap()
    }
    fn image(&self, additional: bool, name: &str, size: usize) -> String {
        use std::io::Write as _;
        let path = if additional {
            &self.additional
        } else {
            &self.primary
        }
        .join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        file.set_len(size as u64).unwrap();
        if additional {
            path.to_str().unwrap().to_owned()
        } else {
            name.to_owned()
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}
struct Allow;
impl PermissionHandler for Allow {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async {
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}
struct NeverDeadline;
impl VisionDeadline for NeverDeadline {
    fn wait_until(&self, _: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        Box::pin(std::future::pending())
    }
}
#[derive(Default)]
struct Transport {
    batches: Mutex<Vec<Vec<(u64, usize)>>>,
    expire: Mutex<Option<NativeConversationTurn>>,
}
impl VisionTransport for Transport {
    fn analyze(
        &self,
        request: VisionBatchRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<VisionBatchResponse, VisionTransportError>> {
        Box::pin(async move {
            self.batches.lock().unwrap().push(
                request
                    .images()
                    .iter()
                    .map(|image| (image.image_id(), image.bytes().len()))
                    .collect(),
            );
            drop(self.expire.lock().unwrap().take());
            VisionBatchResponse::new(
                request
                    .images()
                    .iter()
                    .rev()
                    .map(|image| {
                        VisionImageResult::new(
                            image.image_id(),
                            VisionImageOutcome::Ok {
                                summary: format!("bytes:{}", image.bytes().len()),
                                visible_text: Vec::new(),
                                details: Vec::new(),
                            },
                        )
                        .unwrap()
                    })
                    .collect(),
            )
        })
    }
}
fn target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".into(),
        host: "ai-gateway.vercel.sh".into(),
        port: None,
    }
}
fn context(conversation: &NativeConversation, turn: &NativeConversationTurn) -> ToolContext {
    ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("vision-call").unwrap(),
    }
}
fn call(paths: &[String]) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("vision-call").unwrap(),
        name: vision_name(),
        arguments: json!({"focus":"inspect", "paths":paths}),
    }
}
fn execute(
    tool: &VisionTool,
    context: &ToolContext,
    paths: &[String],
) -> Result<ToolOutput, ToolError> {
    let prepared = tool.prepare_for_turn(context, call(paths))?;
    block_on(tool.execute(
        context.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

#[test]
fn mixed_roots_keep_order_capability_and_retained_identity() {
    let fixture = Fixture::new();
    let transport = Arc::new(Transport::default());
    let tool = fixture.tool(transport.clone());
    let paths = vec![
        fixture.image(false, "same.png", 8),
        fixture.image(true, "same.png", 18),
    ];
    let conversation = fixture.conversation();
    let turn = block_on(conversation.prompt("inspect".into(), 1)).unwrap();
    let context = context(&conversation, &turn);
    let prepared = tool.prepare_for_turn(&context, call(&paths)).unwrap();
    assert_eq!(
        prepared.capability(),
        Some(&Capability::Vision {
            paths: paths.clone(),
            target: target()
        })
    );
    assert!(transport.batches.lock().unwrap().is_empty());
    std::fs::rename(&fixture.additional, fixture.base.join("moved")).unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    let output = execute(&tool, &context, &paths).unwrap();
    assert_eq!(output.content["images"][0]["summary"], "bytes:8");
    assert_eq!(output.content["images"][1]["summary"], "bytes:18");
    assert_eq!(
        *transport.batches.lock().unwrap(),
        vec![vec![(1, 8), (2, 18)]]
    );
}

#[test]
fn global_byte_and_batch_budgets_do_not_multiply_by_roots() {
    let fixture = Fixture::new();
    let transport = Arc::new(Transport::default());
    let tool = fixture.tool(transport.clone());
    let paths = (0..10)
        .map(|index| {
            fixture.image(
                index % 2 == 1,
                &format!("{index}.png"),
                MAX_VISION_IMAGE_BYTES,
            )
        })
        .collect::<Vec<_>>();
    let conversation = fixture.conversation();
    let turn = block_on(conversation.prompt("inspect".into(), 1)).unwrap();
    let output = execute(&tool, &context(&conversation, &turn), &paths).unwrap();
    let batches = transport.batches.lock().unwrap();
    assert_eq!(batches.len(), 8);
    assert_eq!(
        batches
            .iter()
            .flatten()
            .map(|(_, bytes)| bytes)
            .sum::<usize>(),
        MAX_VISION_TOTAL_IMAGE_BYTES
    );
    assert_eq!(
        output.content["images"][8]["error"]["code"],
        "image_unavailable"
    );
    assert_eq!(
        output.content["images"][9]["error"]["code"],
        "image_unavailable"
    );
}

#[test]
fn expiry_after_dispatch_preserves_received_evidence_and_stops_later_images() {
    let fixture = Fixture::new();
    let transport = Arc::new(Transport::default());
    let tool = fixture.tool(transport.clone());
    let paths = (0..10)
        .map(|index| fixture.image(index % 2 == 1, &format!("{index}.png"), 8))
        .collect::<Vec<_>>();
    let conversation = fixture.conversation();
    let turn = block_on(conversation.prompt("inspect".into(), 1)).unwrap();
    let context = context(&conversation, &turn);
    *transport.expire.lock().unwrap() = Some(turn);
    let output = execute(&tool, &context, &paths).unwrap();
    assert_eq!(transport.batches.lock().unwrap().len(), 1);
    assert_eq!(output.content["images"][7]["summary"], "bytes:8");
    assert_eq!(
        output.content["images"][8]["error"]["code"],
        "image_unavailable"
    );
    assert_eq!(
        output.content["images"][9]["error"]["code"],
        "image_unavailable"
    );
}

#[test]
fn foreign_expired_cancelled_and_unpolled_calls_never_fallback() {
    let fixture = Fixture::new();
    let transport = Arc::new(Transport::default());
    let tool = fixture.tool(transport.clone());
    let paths = vec![fixture.image(false, "same.png", 8)];
    let conversation = fixture.conversation();
    let turn = block_on(conversation.prompt("inspect".into(), 1)).unwrap();
    let context = context(&conversation, &turn);
    drop(tool.execute(
        context.clone(),
        call(&paths).arguments,
        CancellationToken::new(),
    ));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        block_on(tool.execute(context.clone(), call(&paths).arguments, cancellation))
            .unwrap_err()
            .code,
        "vision_cancelled"
    );
    let mut foreign = context.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert_eq!(
        execute(&tool, &foreign, &paths).unwrap_err().code,
        "workspace_context_unavailable"
    );
    drop(turn);
    assert_eq!(
        execute(&tool, &context, &paths).unwrap_err().code,
        "workspace_context_unavailable"
    );
    let ids = json!({"focus":"inspect", "image_ids":[9]});
    let output = block_on(tool.execute(foreign, ids, CancellationToken::new())).unwrap();
    assert_eq!(output.content["images"][0]["image_id"], 9);
    assert!(transport.batches.lock().unwrap().is_empty());
}

#[test]
fn strict_image_grammar_and_outside_state_symlinks_remain_bounded() {
    let fixture = Fixture::new();
    let transport = Arc::new(Transport::default());
    let tool = fixture.tool(transport.clone());
    let conversation = fixture.conversation();
    let turn = block_on(conversation.prompt("inspect".into(), 1)).unwrap();
    let context = context(&conversation, &turn);
    for path in [
        "./one.png".into(),
        "sub//one.png".into(),
        "../one.png".into(),
        "~/one.png".into(),
        "a\0.png".into(),
        "a".repeat(MAX_VISION_PATH_BYTES + 1),
        fixture.base.join("state/one.png").to_str().unwrap().into(),
        format!("{}/./one.png", fixture.additional.display()),
        format!("{}/one\n.png", fixture.additional.display()),
    ] {
        assert!(tool.prepare_for_turn(&context, call(&[path])).is_err());
    }
    let path = fixture.image(true, "one.png", 8);
    assert!(
        tool.prepare_for_turn(&context, call(&[path.clone(), path]))
            .is_err()
    );
    std::os::unix::fs::symlink(
        fixture.additional.join("one.png"),
        fixture.primary.join("linked.png"),
    )
    .unwrap();
    let output = execute(&tool, &context, &["linked.png".into()]).unwrap();
    assert_eq!(
        output.content["images"][0]["error"]["code"],
        "image_unavailable"
    );
    assert!(transport.batches.lock().unwrap().is_empty());
}
