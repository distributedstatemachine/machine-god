use super::*;
use crate::{NativeOwnedWorkerScope, ToolResultArchive, ToolResultArchiveHandle};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    ContentBlock, Engine, EngineLimits, ManagedAgentState, ManagedInspection, ManagedQueueStatus,
    ManagedQueuedMessage, ManagedRequested, ManagedResultStatus, ManagedSubagentResult, ModelEvent,
    PermissionDecision, PermissionGrantScope, SessionId, SessionIncarnationId, StopReason,
    ToolCallId, ToolName,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler, ScriptedSubagentAuthority, SubagentStep,
};
use serde_json::json;
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct ArchiveFixture {
    path: PathBuf,
    archive: Arc<ToolResultArchive>,
    workers: NativeOwnedWorkerScope,
}
impl ArchiveFixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mg-managed-publication-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let root = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        Self {
            path,
            archive: Arc::new(ToolResultArchive::from_root_descriptor(root)),
            workers: NativeOwnedWorkerScope::new(),
        }
    }
    fn adapter(&self) -> Arc<NativeToolResultArchiveAdapter> {
        Arc::new(
            NativeToolResultArchiveAdapter::new(self.archive.clone())
                .with_worker_scope(self.workers.clone()),
        )
    }
    fn source(&self, context: &ToolContext, reference: &Value) -> Value {
        let handle =
            ToolResultArchiveHandle::parse(reference["archive"]["handle"].as_str().unwrap())
                .unwrap();
        let mut text = String::new();
        for _ in 0..=COMPLETE_OUTPUT_BYTES.div_ceil(16 * 1024) {
            let page = self
                .archive
                .read(context, &handle, text.len() + 1, 16 * 1024)
                .unwrap();
            text.push_str(&page.text);
            if page.end_byte == page.source_total_bytes {
                return serde_json::from_str(&text).unwrap();
            }
            assert!(!page.text.is_empty(), "archive paging must advance");
        }
        panic!("bounded managed archive exceeded its declared source limit");
    }
}
impl Drop for ArchiveFixture {
    fn drop(&mut self) {
        self.workers.close();
        block_on(self.workers.completion().wait());
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

fn large_result() -> ManagedSubagentResult {
    ManagedSubagentResult {
        ok: true,
        operation_id: "operation".into(),
        child_id: Some("child".into()),
        status: ManagedResultStatus::Inspected,
        error_code: None,
        retryable: false,
        cursor: None,
        requested: Some(ManagedRequested::Inspection(Box::new(ManagedInspection {
            child_id: "child".into(),
            generation: 1,
            status: Some(ManagedAgentState::Idle),
            messages: vec![ManagedQueuedMessage {
                id: "work".into(),
                source_id: "parent".into(),
                content: "a".repeat(64 * 1024),
                status: ManagedQueueStatus::Completed,
                cancellation_reason: None,
                created_at_ms: 1,
            }],
            ..ManagedInspection::default()
        }))),
    }
}

#[test]
fn admitted_full_input_and_output_are_losslessly_archived_without_widening_ordinary_limits() {
    let fixture = ArchiveFixture::new();
    let result = large_result();
    result.validate().unwrap();
    let authority = ScriptedSubagentAuthority::new([SubagentStep::Complete(result.clone())]);
    // Six serialized bytes per semantic byte exercises the full declared prompt bound.
    let arguments = json!({"command":{"create":{
        "name":"child", "mode":"one_off", "prompt":"\u{1}".repeat(64 * 1024)
    }}});
    let call = ToolCall {
        id: ToolCallId::new("call").unwrap(),
        name: ToolName::new("subagent").unwrap(),
        arguments: arguments.clone(),
    };
    let engine = Engine::builder()
        .limits(EngineLimits {
            max_cumulative_complete_tool_argument_bytes: bound(MAX_SUBAGENT_ARGUMENT_BYTES),
            max_cumulative_complete_tool_result_bytes: bound(COMPLETE_OUTPUT_BYTES),
            ..EngineLimits::default()
        })
        .provider(ScriptedModelProvider::new(
            "managed-publication",
            [
                ModelProviderStep::events([
                    ModelEvent::ToolCall { call },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ]),
                ModelProviderStep::events([ModelEvent::Stop {
                    reason: StopReason::Completed,
                }]),
            ],
        ))
        .session_store(InMemorySessionStore::default())
        .permission_handler(ScriptedPermissionHandler::new([PermissionStep::Decision(
            PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            },
        )]))
        .tool(NativeManagedSubagentTool::new(
            Arc::new(authority.clone()),
            fixture.adapter(),
        ))
        .build()
        .unwrap();
    assert_eq!(
        engine.limits().max_tool_argument_bytes,
        EngineLimits::default().max_tool_argument_bytes
    );
    let session = engine
        .create_session(
            SessionId::new("parent").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        )
        .unwrap();
    let turn = block_on(session.prompt("task")).unwrap();
    authority.bind_turn(turn.witness());
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    let calls = authority.requests();
    assert_eq!(calls.len(), 1, "{events:?}");
    let context = &calls[0].context;
    let record = session.record();
    assert!(serde_json::to_vec(&record).unwrap().len() < 64 * 1024);
    let mut inputs = 0;
    let mut outputs = 0;
    for block in record.messages.iter().flat_map(|message| &message.content) {
        match block {
            ContentBlock::ToolCall { call } => {
                assert_eq!(call.arguments["type"], "tool_arguments_archive");
                assert_eq!(
                    fixture.source(context, &call.arguments),
                    serde_json::to_value(ToolOutput::success(arguments.clone())).unwrap()
                );
                inputs += 1;
            }
            ContentBlock::ToolResult { output, .. } => {
                assert_eq!(output.content["type"], "tool_result_archive");
                assert_eq!(
                    fixture.source(context, &output.content),
                    serde_json::to_value(ToolOutput::success(
                        serde_json::to_value(&result).unwrap()
                    ))
                    .unwrap()
                );
                outputs += 1;
            }
            _ => {}
        }
    }
    assert_eq!((inputs, outputs), (1, 1));
}
