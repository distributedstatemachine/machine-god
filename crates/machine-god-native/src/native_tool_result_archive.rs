//! Owned-worker binding for terminal result publication and historical paging.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::owned_worker::NativeOwnedWorkerSpawner;
use crate::session_store::JsonValueOwner;
use crate::terminal_action_tool::{
    MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES, TerminalActionResultPublisher,
};
use crate::tool_output_serializer::{
    CompactJsonScratch, CompactToolOutputLimits, serialize_tool_output_compact_with_scratch,
};
use crate::tool_result_archive::{
    ArchivedToolResult, ToolResultArchive, ToolResultArchiveError, ToolResultArchivePage,
};
use machine_god_core::{
    BoxFuture, CancellationToken, MAX_SAFE_JSON_DEPTH, ToolContext, ToolError, ToolErrorKind,
    ToolExecution, ToolOutput,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const INLINE_BYTES: usize = 64 * 1024;
const REFERENCE_BYTES: usize = 16 * 1024;
const PREVIEW_BYTES: usize = 1024;
const ACTIVE_OPERATIONS: usize = 2;

/// One shared, bounded native binding for both full-result publication and reads.
/// Construction retains explicit archive authority but performs no I/O or spawn.
pub struct NativeToolResultArchiveAdapter {
    archive: Arc<ToolResultArchive>,
    active: Arc<AtomicUsize>,
}
impl std::fmt::Debug for NativeToolResultArchiveAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeToolResultArchiveAdapter")
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ArchivedReference {
    ToolResultArchive {
        archive: ArchivedToolResult,
        preview: String,
    },
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Receipt<T> {
    result: T,
    _permit: Permit,
}
struct OutputOwner(Option<ToolOutput>);
impl OutputOwner {
    fn get(&self) -> &ToolOutput {
        self.0.as_ref().expect("owned output is present")
    }
    fn take(mut self) -> ToolOutput {
        self.0.take().expect("owned output is present")
    }
}
impl Drop for OutputOwner {
    fn drop(&mut self) {
        if let Some(output) = &mut self.0 {
            drop(JsonValueOwner::new(std::mem::take(&mut output.content)));
        }
    }
}

impl NativeToolResultArchiveAdapter {
    /// Retains the dedicated archive capability with two operation/receipt slots.
    #[must_use]
    pub fn new(archive: Arc<ToolResultArchive>) -> Self {
        Self {
            archive,
            active: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn acquire(active: Arc<AtomicUsize>, read: bool) -> Result<Permit, ToolError> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < ACTIVE_OPERATIONS).then_some(count + 1)
            })
            .map_err(|_| archive_error(ToolResultArchiveError::Busy, read))?;
        Ok(Permit(active))
    }

    pub(crate) fn read(
        &self,
        current: ToolContext,
        reference: ArchivedToolResult,
        start: usize,
        count: usize,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<ToolOutput, ToolError>> {
        let archive = Arc::clone(&self.archive);
        let active = Arc::clone(&self.active);
        Box::pin(async move {
            check_reader(&current, &reference, &cancellation)?;
            let permit = Self::acquire(active, true)?;
            let worker_cancellation = cancellation.clone();
            let result = NativeOwnedWorkerSpawner::new()
                .run(move || {
                    let result = (|| {
                        check_reader(&current, &reference, &worker_cancellation)?;
                        let page = archive
                            .read(&reference.source_context, &reference.handle, start, count)
                            .map_err(|error| archive_error(error, true))?;
                        if page.source_total_bytes != reference.source_total_bytes {
                            return Err(archive_error(ToolResultArchiveError::Corrupt, true));
                        }
                        check_reader(&current, &reference, &worker_cancellation)?;
                        Ok(page_output(&reference, page))
                    })();
                    Receipt {
                        result,
                        _permit: permit,
                    }
                })
                .await
                .map_err(|_| archive_error(ToolResultArchiveError::Unavailable, true))?;
            if cancellation.is_cancelled() {
                return Err(archive_error(ToolResultArchiveError::Cancelled, true));
            }
            result.result
        })
    }
}

impl TerminalActionResultPublisher for NativeToolResultArchiveAdapter {
    fn publish(
        &self,
        context: ToolContext,
        output: ToolOutput,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let archive = Arc::clone(&self.archive);
        let active = Arc::clone(&self.active);
        let output = OutputOwner(Some(output));
        Box::pin(async move {
            let permit = Self::acquire(active, false)?;
            let receipt = NativeOwnedWorkerSpawner::new()
                .run(move || {
                    let result = publish_owned(&archive, &context, output);
                    Receipt {
                        result,
                        _permit: permit,
                    }
                })
                .await
                .map_err(|_| archive_error(ToolResultArchiveError::Unavailable, false))?;
            receipt.result
        })
    }
}

fn publish_owned(
    archive: &ToolResultArchive,
    context: &ToolContext,
    output: OutputOwner,
) -> Result<ToolExecution, ToolError> {
    let mut compact = Vec::new();
    serialize(
        output.get(),
        &mut compact,
        MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES,
    )?;
    if compact.len() <= INLINE_BYTES {
        return Ok(ToolExecution::output(output.take()));
    }
    archive
        .prepare()
        .map_err(|error| archive_error(error, false))?;
    let archived = archive
        .publish(context, &compact, &CancellationToken::new())
        .map_err(|error| archive_error(error, false))?;
    let mut end = PREVIEW_BYTES.min(compact.len());
    let source = std::str::from_utf8(&compact)
        .map_err(|_| archive_error(ToolResultArchiveError::Corrupt, false))?;
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let reference = ToolOutput {
        content: serde_json::to_value(ArchivedReference::ToolResultArchive {
            archive: archived,
            preview: source[..end].to_owned(),
        })
        .map_err(|_| archive_error(ToolResultArchiveError::Invalid, false))?,
        is_error: output.get().is_error,
    };
    // Keep the durable reference below Gateway's preview threshold; the model
    // must see the archive handle directly rather than a preview of a preview.
    compact.clear();
    serialize(&reference, &mut compact, REFERENCE_BYTES)?;
    Ok(ToolExecution::with_persisted_output(
        output.take(),
        reference,
    ))
}

fn serialize(
    output: &ToolOutput,
    destination: &mut Vec<u8>,
    maximum: usize,
) -> Result<(), ToolError> {
    serialize_tool_output_compact_with_scratch(
        output,
        destination,
        &mut CompactJsonScratch::new(),
        CompactToolOutputLimits {
            output_bytes: maximum,
            json_depth: MAX_SAFE_JSON_DEPTH,
            json_nodes: maximum,
        },
        &CancellationToken::new(),
    )
    .map_err(|_| archive_error(ToolResultArchiveError::Invalid, false))
}

fn check_reader(
    current: &ToolContext,
    source: &ArchivedToolResult,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        return Err(archive_error(ToolResultArchiveError::Cancelled, true));
    }
    if current.session_id != source.source_context.session_id
        || current.session_incarnation_id != source.source_context.session_incarnation_id
    {
        return Err(archive_error(ToolResultArchiveError::Denied, true));
    }
    Ok(())
}

fn page_output(reference: &ArchivedToolResult, page: ToolResultArchivePage) -> ToolOutput {
    let mut content = json!({
        "handle": reference.handle.as_str(), "start_byte": page.start_byte,
        "end_byte": page.end_byte, "total_bytes": page.source_total_bytes,
        "has_more": page.end_byte < page.source_total_bytes,
    });
    content["serialized_tool_output"] = serde_json::Value::String(page.text);
    ToolOutput::success(content)
}

fn archive_error(error: ToolResultArchiveError, read: bool) -> ToolError {
    ToolError::new(
        if error == ToolResultArchiveError::Cancelled {
            ToolErrorKind::Cancelled
        } else if read && error == ToolResultArchiveError::Invalid {
            ToolErrorKind::InvalidInput
        } else {
            ToolErrorKind::Execution
        },
        "tool_result_archive_unavailable",
        "tool result archive operation unavailable",
        read && matches!(
            error,
            ToolResultArchiveError::Busy | ToolResultArchiveError::Unavailable
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReadToolResultTool;
    use futures_executor::block_on;
    use machine_god_core::{
        ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
        Tool, ToolCall, ToolCallId, ToolName, TurnId,
    };
    use machine_god_testkit::InMemorySessionStore;
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "machine-god-result-binding-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn adapter(&self) -> Arc<NativeToolResultArchiveAdapter> {
            let fd = rustix::fs::open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap();
            Arc::new(NativeToolResultArchiveAdapter::new(Arc::new(
                ToolResultArchive::from_root_descriptor(fd),
            )))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("owner").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("original-turn").unwrap(),
            call_id: ToolCallId::new("original-call").unwrap(),
        }
    }
    fn reference(execution: &ToolExecution) -> ArchivedToolResult {
        let ArchivedReference::ToolResultArchive { archive, .. } =
            serde_json::from_value(execution.persisted_output().unwrap().content.clone()).unwrap();
        archive
    }
    fn reader(
        adapter: Arc<NativeToolResultArchiveAdapter>,
        output: ToolOutput,
        source_call: ToolCallId,
    ) -> ReadToolResultTool {
        let source = context();
        let store = InMemorySessionStore::new();
        let mut record = SessionRecord::empty(source.session_id, source.session_incarnation_id);
        record.messages.push(Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: source_call,
                output,
            }],
        });
        block_on(store.save(record, None)).unwrap();
        ReadToolResultTool::shared_session_store(Arc::new(store)).with_archive(adapter)
    }

    #[test]
    fn construction_unpolled_publication_and_small_results_do_not_create_archive_files() {
        let directory = Directory::new();
        let adapter = directory.adapter();
        drop(adapter.publish(context(), ToolOutput::success("x".repeat(90 * 1024))));
        let execution = block_on(adapter.publish(context(), ToolOutput::success("small"))).unwrap();
        assert!(execution.persisted_output().is_none());
        assert_eq!(execution.tool_output().content, "small");
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn complete_result_is_durable_reopenable_and_paged_by_the_real_reader() {
        let directory = Directory::new();
        let adapter = directory.adapter();
        let output = ToolOutput {
            content: json!({"text":format!("{}🦀\u{1b}", "x".repeat(90 * 1024))}),
            is_error: true,
        };
        let source = serde_json::to_string(&output).unwrap();
        let execution = block_on(adapter.publish(context(), output.clone())).unwrap();
        assert_eq!(execution.tool_output(), &output);
        let archived = reference(&execution);
        assert_eq!(archived.source_total_bytes, source.len());
        let persisted = execution.persisted_output().unwrap().clone();
        assert!(persisted.is_error);
        assert!(serde_json::to_vec(&persisted).unwrap().len() < REFERENCE_BYTES);
        drop(adapter);
        let adapter = directory.adapter();
        let tool = reader(adapter, persisted, context().call_id);
        let mut current = context();
        current.call_id = ToolCallId::new("page-call").unwrap();
        current.turn_id = TurnId::new("later-turn").unwrap();
        let arguments = json!({"handle":archived.handle.as_str(), "start_byte":70 * 1024 + 1,"byte_count":16 * 1024});
        let prepared = tool
            .prepare(ToolCall {
                id: current.call_id.clone(),
                name: ToolName::new("read_tool_result").unwrap(),
                arguments,
            })
            .unwrap();
        let page = block_on(tool.execute(
            current,
            prepared.arguments().clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(
            page.content["serialized_tool_output"],
            &source[70 * 1024..86 * 1024]
        );
        assert_eq!(page.content["total_bytes"], source.len());
    }

    #[test]
    fn archive_reader_requires_enabled_capability_and_matching_durable_call_and_owner() {
        let directory = Directory::new();
        let adapter = directory.adapter();
        let execution =
            block_on(adapter.publish(context(), ToolOutput::success("x".repeat(90 * 1024))))
                .unwrap();
        let archived = reference(&execution);
        let args = json!({"handle":archived.handle.as_str(),"start_byte":70 * 1024});
        let disabled =
            ReadToolResultTool::shared_session_store(Arc::new(InMemorySessionStore::new()));
        assert!(
            disabled
                .prepare(ToolCall {
                    id: ToolCallId::new("read").unwrap(),
                    name: ToolName::new("read_tool_result").unwrap(),
                    arguments: args.clone()
                })
                .is_err()
        );
        let tool = reader(
            Arc::clone(&adapter),
            execution.persisted_output().unwrap().clone(),
            ToolCallId::new("wrong-call").unwrap(),
        );
        assert!(block_on(tool.execute(context(), args.clone(), CancellationToken::new())).is_err());
        let tool = reader(
            Arc::clone(&adapter),
            execution.persisted_output().unwrap().clone(),
            context().call_id,
        );
        let mut foreign = context();
        foreign.session_incarnation_id = SessionIncarnationId::new("other-incarnation").unwrap();
        assert!(block_on(tool.execute(foreign, args, CancellationToken::new())).is_err());
        let mut wrong_length = archived;
        wrong_length.source_total_bytes += 1;
        assert!(
            block_on(adapter.read(context(), wrong_length, 1, 4096, CancellationToken::new()))
                .is_err()
        );
    }

    #[test]
    fn refused_publication_never_returns_a_durable_reference_and_precancelled_read_is_inert() {
        let directory = Directory::new();
        let adapter = directory.adapter();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let source = ArchivedToolResult {
            handle: crate::ToolResultArchiveHandle::parse(format!(
                "tool-archive-v1-{}-{}",
                "0".repeat(64),
                "0".repeat(64)
            ))
            .unwrap(),
            source_total_bytes: 1,
            source_context: context(),
        };
        assert_eq!(
            block_on(adapter.read(context(), source, 1, 4, cancellation))
                .unwrap_err()
                .kind,
            ToolErrorKind::Cancelled
        );
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
        std::fs::write(directory.0.join("archive-lock-v1"), b"unexpected").unwrap();
        let error =
            block_on(adapter.publish(context(), ToolOutput::success("x".repeat(90 * 1024))))
                .unwrap_err();
        assert!(
            !error.retryable,
            "committed action publication cannot authorize resubmission"
        );
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}
