//! Transparent instrumentation of explicitly selected native file tools.
use crate::conversation_history::NativeHistoryFileAction;
use crate::conversation_observations::{NativeConversationObservations, ObservationReservation};
use crate::session_store::{
    JsonValueOwner, MAX_FILE_SESSION_BYTES, MAX_STORED_JSON_DEPTH, MAX_STORED_JSON_NODES,
};
use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolExecution, ToolInputLimits, ToolOutput, ToolOutputLimits, ToolSpec,
};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeFileHistoryKind {
    Read,
    List,
    Glob,
    Grep,
    Write,
    Edit,
    Delete,
    Rename,
    Copy,
}
impl NativeFileHistoryKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Read => "read_file",
            Self::List => "list_files",
            Self::Glob => "glob_files",
            Self::Grep => "grep_files",
            Self::Write => "write_file",
            Self::Edit => "edit_file",
            Self::Delete => "delete_file",
            Self::Rename => "rename_file",
            Self::Copy => "copy_file",
        }
    }
    const fn action(self) -> NativeHistoryFileAction {
        match self {
            Self::Read => NativeHistoryFileAction::Read,
            Self::List => NativeHistoryFileAction::List,
            Self::Glob | Self::Grep => NativeHistoryFileAction::Search,
            Self::Write => NativeHistoryFileAction::Write,
            Self::Edit => NativeHistoryFileAction::Edit,
            Self::Delete => NativeHistoryFileAction::Delete,
            Self::Rename => NativeHistoryFileAction::Rename,
            Self::Copy => NativeHistoryFileAction::Copy,
        }
    }
}
pub(crate) struct NativeFileHistoryTool<T> {
    tool: T,
    kind: NativeFileHistoryKind,
    registry: Arc<NativeConversationObservations>,
}
impl<T: Tool> NativeFileHistoryTool<T> {
    pub(crate) fn new(
        tool: T,
        kind: NativeFileHistoryKind,
        registry: Arc<NativeConversationObservations>,
    ) -> Self {
        Self {
            tool,
            kind,
            registry,
        }
    }
    fn reserve(
        &self,
        context: &ToolContext,
        args: &Value,
    ) -> Result<ObservationReservation, ToolError> {
        if self.tool.spec().name.as_str() != self.kind.name() {
            return Err(observation_error());
        }
        let (path, destination) = match self.kind {
            NativeFileHistoryKind::Rename => ("old_path", Some("new_path")),
            NativeFileHistoryKind::Copy => ("source", Some("destination")),
            _ => ("path", None),
        };
        let path = canonical_path(
            args,
            path,
            matches!(
                self.kind,
                NativeFileHistoryKind::List
                    | NativeFileHistoryKind::Glob
                    | NativeFileHistoryKind::Grep
            ),
        )?;
        let destination = destination
            .map(|key| canonical_path(args, key, false))
            .transpose()?;
        self.registry
            .reserve(
                context,
                path,
                destination,
                self.kind.action(),
                self.kind.name(),
            )
            .map_err(|_| observation_error())
    }
    fn full_file(&self, output: &ToolOutput, persisted: bool) -> bool {
        if self.kind != NativeFileHistoryKind::Read || output.is_error || persisted {
            return false;
        }
        let mut compact = Vec::new();
        let valid = crate::tool_output_serializer::serialize_tool_output_compact_with_scratch(
            output,
            &mut compact,
            &mut crate::tool_output_serializer::CompactJsonScratch::new(),
            crate::tool_output_serializer::CompactToolOutputLimits {
                output_bytes: MAX_FILE_SESSION_BYTES,
                json_depth: MAX_STORED_JSON_DEPTH,
                json_nodes: MAX_STORED_JSON_NODES,
            },
            &CancellationToken::new(),
        )
        .is_ok();
        valid
            && !crate::read_tool_result::should_project_tool_result(
                true,
                self.kind.name(),
                compact.len(),
            )
    }
}
impl<T: Tool> Tool for NativeFileHistoryTool<T> {
    fn spec(&self) -> ToolSpec {
        self.tool.spec()
    }
    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        self.tool.complete_input_limits()
    }
    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        self.tool.complete_output_limits()
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.tool.prepare(call)
    }
    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        args: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        self.tool.persist_arguments(context, args, cancellation)
    }
    fn execute(
        &self,
        context: ToolContext,
        args: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let mut args = Arguments(Some(args));
        Box::pin(async move {
            let reservation = self.reserve(&context, args.get())?;
            let result = self.tool.execute(context, args.take(), cancellation).await;
            let success = result.as_ref().is_ok_and(|output| !output.is_error);
            let full = result
                .as_ref()
                .is_ok_and(|output| self.full_file(output, false));
            reservation.settle(success, full);
            result
        })
    }
    fn execute_for_turn(
        &self,
        context: ToolContext,
        args: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let mut args = Arguments(Some(args));
        Box::pin(async move {
            let reservation = self.reserve(&context, args.get())?;
            let result = self
                .tool
                .execute_for_turn(context, args.take(), cancellation)
                .await;
            let success = result
                .as_ref()
                .is_ok_and(|execution| !execution.tool_output().is_error);
            let full = result.as_ref().is_ok_and(|execution| {
                self.full_file(
                    execution.tool_output(),
                    execution.persisted_output().is_some(),
                )
            });
            reservation.settle(success, full);
            result
        })
    }
}
struct Arguments(Option<Value>);
impl Arguments {
    fn get(&self) -> &Value {
        self.0
            .as_ref()
            .expect("arguments retained before admission")
    }
    fn take(&mut self) -> Value {
        self.0.take().expect("arguments consumed once")
    }
}
impl Drop for Arguments {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            drop(JsonValueOwner::new(value));
        }
    }
}
fn observation_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "native_history_observation_unavailable",
        "native history observation unavailable",
        false,
    )
}
fn canonical_path<'a>(args: &'a Value, key: &str, allow_root: bool) -> Result<&'a str, ToolError> {
    let path = args
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .ok_or_else(observation_error)?;
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.as_bytes().contains(&0)
        || (path != "." && path.split('/').any(|part| matches!(part, "" | "." | "..")))
        || (path == "." && !allow_root)
    {
        return Err(observation_error());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeHistoryFileStatus as Status;
    use crate::conversation_observations::tests::setup;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fake {
        name: &'static str,
        calls: Arc<AtomicUsize>,
        output: ToolOutput,
        pending: bool,
        fail: bool,
        persisted: bool,
    }
    impl Tool for Fake {
        fn complete_input_limits(&self) -> Option<ToolInputLimits> {
            let one = std::num::NonZeroUsize::new(1).unwrap();
            Some(ToolInputLimits {
                max_argument_bytes: one,
                max_argument_nodes: one,
                max_prepared_argument_bytes: one,
                max_prepared_argument_nodes: one,
            })
        }
        fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
            let one = std::num::NonZeroUsize::new(1).unwrap();
            Some(ToolOutputLimits {
                max_serialized_bytes: one,
                max_json_nodes: one,
            })
        }
        fn persist_arguments<'a>(
            &'a self,
            _context: ToolContext,
            _args: &'a Value,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
            Box::pin(async { Ok(Some(json!({"argument_receipt":"unchanged"}))) })
        }
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: machine_god_core::ToolName::new(self.name).unwrap(),
                description: "fake".into(),
                input_schema: json!({}),
            }
        }
        fn prepare(&self, mut call: ToolCall) -> Result<PreparedToolCall, ToolError> {
            call.arguments["path"] = json!("normalized");
            Ok(PreparedToolCall::without_authority(call.arguments))
        }
        fn execute(
            &self,
            _context: ToolContext,
            args: Value,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                drop(args);
                if self.pending {
                    std::future::pending::<()>().await;
                }
                if self.fail {
                    Err(observation_error())
                } else {
                    Ok(self.output.clone())
                }
            })
        }
        fn execute_for_turn(
            &self,
            context: ToolContext,
            args: Value,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
            Box::pin(async move {
                let output = self.execute(context, args, cancellation).await?;
                Ok(if self.persisted {
                    ToolExecution::with_persisted_output(
                        output,
                        ToolOutput::success(json!({"archive":"opaque"})),
                    )
                } else {
                    ToolExecution::output(output)
                })
            })
        }
    }
    fn fake(name: &'static str, output: ToolOutput) -> Fake {
        Fake {
            name,
            calls: Arc::new(AtomicUsize::new(0)),
            output,
            pending: false,
            fail: false,
            persisted: false,
        }
    }
    #[test]
    fn unpolled_and_failed_route_never_construct_inner_execution() {
        let (registry, session, mut context) = setup("read_file");
        let fake = fake("read_file", ToolOutput::success(json!({"content":"a"})));
        let calls = fake.calls.clone();
        let tool = NativeFileHistoryTool::new(fake, NativeFileHistoryKind::Read, registry);
        drop(tool.execute(
            context.clone(),
            json!({"path":"a"}),
            CancellationToken::new(),
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(session.snapshot().entries().is_empty());
        context.session_id = machine_god_core::SessionId::new("wrong").unwrap();
        assert!(
            futures_executor::block_on(tool.execute(
                context,
                json!({"path":"a"}),
                CancellationToken::new()
            ))
            .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn pending_drop_records_only_unknown() {
        let (registry, session, context) = setup("read_file");
        let mut fake = fake("read_file", ToolOutput::success(json!({})));
        fake.pending = true;
        let tool = NativeFileHistoryTool::new(fake, NativeFileHistoryKind::Read, registry);
        let mut future = tool.execute(context, json!({"path":"a"}), CancellationToken::new());
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        drop(future);
        assert_eq!(
            session.snapshot().entries()[0].file().status(),
            Status::Unknown
        );
    }
    #[test]
    fn outcomes_preserve_return_values_and_do_not_parse_content_fields() {
        for (failure, is_error) in [(false, false), (false, true), (true, false)] {
            let (registry, session, context) = setup("copy_file");
            let output = ToolOutput {
                content: json!({"status":"success","path":"invented"}),
                is_error,
            };
            let mut fake = fake("copy_file", output.clone());
            fake.fail = failure;
            let tool = NativeFileHistoryTool::new(fake, NativeFileHistoryKind::Copy, registry);
            let result = futures_executor::block_on(tool.execute(
                context,
                json!({"source":"source","destination":"dest"}),
                CancellationToken::new(),
            ));
            if failure {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), output);
            }
            let batch = session.snapshot();
            let file = batch.entries()[0].file();
            assert_eq!(file.path(), "source");
            assert_eq!(file.new_path(), Some("dest"));
            assert_eq!(
                file.status(),
                if failure || is_error {
                    Status::Failure
                } else {
                    Status::Success
                }
            );
            assert!(!file.model_view_covers_full_file());
        }
    }
    #[test]
    fn read_full_file_uses_exact_escaped_whole_output_projection_size() {
        for (content, expected) in [("ordinary".to_owned(), true), ("\u{1}".repeat(8192), false)] {
            let (registry, session, context) = setup("read_file");
            let tool = NativeFileHistoryTool::new(
                fake("read_file", ToolOutput::success(json!({"content":content}))),
                NativeFileHistoryKind::Read,
                registry,
            );
            futures_executor::block_on(tool.execute(
                context,
                json!({"path":"a"}),
                CancellationToken::new(),
            ))
            .unwrap();
            assert_eq!(
                session.snapshot().entries()[0]
                    .file()
                    .model_view_covers_full_file(),
                expected
            );
        }
        for (size, expected) in [(16384, false), (16385, true), (65536, true), (65537, false)] {
            assert_eq!(
                crate::read_tool_result::should_project_tool_result(true, "read_file", size),
                expected
            );
            assert!(!crate::read_tool_result::should_project_tool_result(
                false,
                "read_file",
                size
            ));
            assert!(!crate::read_tool_result::should_project_tool_result(
                true,
                "read_tool_result",
                size
            ));
        }
    }
    #[test]
    fn persisted_execution_representation_is_preserved_and_not_full_file() {
        let (registry, session, context) = setup("read_file");
        let mut fake = fake("read_file", ToolOutput::success(json!({"content":"whole"})));
        fake.persisted = true;
        let tool = NativeFileHistoryTool::new(fake, NativeFileHistoryKind::Read, registry);
        let result = futures_executor::block_on(tool.execute_for_turn(
            context,
            json!({"path":"a"}),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(
            result.persisted_output().unwrap().content,
            json!({"archive":"opaque"})
        );
        assert_eq!(result.tool_output().content, json!({"content":"whole"}));
        assert!(
            !session.snapshot().entries()[0]
                .file()
                .model_view_covers_full_file()
        );
    }
    #[test]
    fn canonical_paths_are_bounded_and_preparation_is_transparent() {
        let (registry, _session, context) = setup("read_file");
        let fake = fake("read_file", ToolOutput::success(json!({})));
        let calls = fake.calls.clone();
        let tool = NativeFileHistoryTool::new(fake, NativeFileHistoryKind::Read, registry);
        assert_eq!(
            tool.complete_input_limits(),
            tool.tool.complete_input_limits()
        );
        assert_eq!(
            tool.complete_output_limits(),
            tool.tool.complete_output_limits()
        );
        assert_eq!(
            futures_executor::block_on(tool.persist_arguments(
                context.clone(),
                &json!({"path":"a"}),
                CancellationToken::new()
            ))
            .unwrap(),
            Some(json!({"argument_receipt":"unchanged"}))
        );
        let prepared = tool
            .prepare(ToolCall {
                id: context.call_id.clone(),
                name: machine_god_core::ToolName::new("read_file").unwrap(),
                arguments: json!({"path":"./normalized"}),
            })
            .unwrap();
        assert_eq!(prepared.arguments(), &json!({"path":"normalized"}));
        for path in ["", "/a", "a/../b", "./a", "a//b", "a\0b", "."] {
            assert!(
                futures_executor::block_on(tool.execute(
                    context.clone(),
                    json!({"path":path}),
                    CancellationToken::new()
                ))
                .is_err()
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(canonical_path(&json!({"path":"."}), "path", true).is_ok());
    }
    #[test]
    fn all_native_kinds_use_explicit_canonical_keys_and_actions() {
        for kind in [
            NativeFileHistoryKind::Read,
            NativeFileHistoryKind::List,
            NativeFileHistoryKind::Glob,
            NativeFileHistoryKind::Grep,
            NativeFileHistoryKind::Write,
            NativeFileHistoryKind::Edit,
            NativeFileHistoryKind::Delete,
            NativeFileHistoryKind::Rename,
            NativeFileHistoryKind::Copy,
        ] {
            let (registry, session, context) = setup(kind.name());
            let tool = NativeFileHistoryTool::new(
                fake(kind.name(), ToolOutput::success(json!({}))),
                kind,
                registry,
            );
            let args = match kind {
                NativeFileHistoryKind::Rename => json!({"old_path":"a","new_path":"b"}),
                NativeFileHistoryKind::Copy => json!({"source":"a","destination":"b"}),
                _ => json!({"path":"a"}),
            };
            futures_executor::block_on(tool.execute(context, args, CancellationToken::new()))
                .unwrap();
            assert_eq!(
                session.snapshot().entries()[0].file().action(),
                kind.action()
            );
        }
    }
}
