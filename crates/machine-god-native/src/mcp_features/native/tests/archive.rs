use super::*;
use crate::{ArchivedToolResult, TOOL_RESULT_ARCHIVE_MAX_PAGE_BYTES, ToolResultArchive};
use futures_executor::block_on;
use machine_god_core::{SessionId, SessionIncarnationId, ToolCallId, ToolName, TurnId};
use std::{os::unix::fs::DirBuilderExt, path::PathBuf};

struct Archive {
    path: PathBuf,
    storage: Arc<ToolResultArchive>,
    adapter: Arc<NativeToolResultArchiveAdapter>,
}
impl Archive {
    fn new() -> Self {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!("machine-god-features-projection-{nonce:x?}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let storage = Arc::new(ToolResultArchive::from_root_descriptor(
            rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .unwrap(),
        ));
        let adapter = Arc::new(NativeToolResultArchiveAdapter::new(storage.clone()));
        Self {
            path,
            storage,
            adapter,
        }
    }
}
impl Drop for Archive {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("features-owner").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        call_id: ToolCallId::new("feature-call").unwrap(),
    }
}

#[test]
fn large_projection_shares_actual_archive_and_preserves_complete_content() {
    let archive = Archive::new();
    let body = format!(
        r#""result":{{"contents":[{{"uri":"test://fixed","text":"{}"}}],"number":1e400}}"#,
        "x".repeat(96 * 1024)
    );
    let (publication, reply) = response("resource read srv test://fixed", &body);
    let (output, _) = projection::project(&publication, &reply, &CancellationToken::new()).unwrap();
    let execution = block_on(archive.adapter.publish(
        context(),
        output.clone(),
        NativeToolResultArchiveLimits {
            compact_bytes: OUTPUT_BYTES,
            json_nodes: OUTPUT_NODES,
        },
    ))
    .unwrap();
    assert_eq!(execution.tool_output(), &output);
    let reference: ArchivedToolResult =
        serde_json::from_value(execution.persisted_output().unwrap().content["archive"].clone())
            .unwrap();
    let mut source = String::new();
    loop {
        let page = archive
            .storage
            .read(
                &reference.source_context,
                &reference.handle,
                source.len() + 1,
                TOOL_RESULT_ARCHIVE_MAX_PAGE_BYTES,
            )
            .unwrap();
        source.push_str(&page.text);
        if page.end_byte == page.source_total_bytes {
            break;
        }
    }
    let decoded: ToolOutput = serde_json::from_str(&source).unwrap();
    assert_eq!(decoded, output);
    assert_eq!(
        decoded.content["untrusted"]["response"]["result"]["number"].to_string(),
        "1e400"
    );
}

#[test]
fn constructor_prepare_and_unpolled_execute_acquire_no_runtime_or_archive_work() {
    let archive = Archive::new();
    let tool = NativeMcpFeaturesTool::new(Weak::new(), archive.adapter.clone());
    let call = ToolCall {
        id: ToolCallId::new("feature-call").unwrap(),
        name: ToolName::new("mcp_features").unwrap(),
        arguments: serde_json::json!({"action":"resource_list","server":"srv"}),
    };
    let prepared = tool.prepare(call.clone()).unwrap();
    assert_eq!(prepared.arguments(), &call.arguments);
    let operation =
        tool.execute_for_turn(context(), call.arguments.clone(), CancellationToken::new());
    drop(operation);
    assert_eq!(std::fs::read_dir(&archive.path).unwrap().count(), 0);
    let error =
        block_on(tool.execute_for_turn(context(), call.arguments, CancellationToken::new()))
            .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Unavailable);
    assert_eq!(std::fs::read_dir(&archive.path).unwrap().count(), 0);
    assert_eq!(
        tool.complete_output_limits()
            .unwrap()
            .max_serialized_bytes
            .get(),
        OUTPUT_BYTES
    );
}
