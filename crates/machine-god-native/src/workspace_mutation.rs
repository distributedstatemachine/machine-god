//! Exact-turn routing for the five descriptor-owned file mutations.

use crate::{
    FileUndoTracker, NativeFileApprovalKind as Kind, NativeFileApprovalRegistry,
    NativeWorkspaceContexts, NativeWorkspaceTurnScope,
};
use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolOutput, ToolSpec,
};
use serde_json::Value;
use std::sync::Arc;

mod projection;
pub(crate) use projection::{WorkspaceMutationEndpoint, WorkspaceMutationProjection, project};

pub(crate) struct WorkspaceMutationTool {
    kind: Kind,
    primary: Arc<dyn Tool>,
    contexts: Arc<NativeWorkspaceContexts>,
    registry: Option<Arc<NativeFileApprovalRegistry>>,
    undo: Option<Arc<FileUndoTracker>>,
}

impl WorkspaceMutationTool {
    pub(crate) fn new(
        kind: Kind,
        primary: Arc<dyn Tool>,
        contexts: Arc<NativeWorkspaceContexts>,
        registry: Option<Arc<NativeFileApprovalRegistry>>,
        undo: Option<Arc<FileUndoTracker>>,
    ) -> Self {
        Self {
            kind,
            primary,
            contexts,
            registry,
            undo,
        }
    }
}

pub(crate) fn check_scope(scope: Option<&NativeWorkspaceTurnScope>) -> Result<(), ToolError> {
    if scope.is_some_and(|scope| !scope.is_live()) {
        Err(unavailable())
    } else {
        Ok(())
    }
}

fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "workspace_context_unavailable",
        "workspace context is unavailable",
        false,
    )
}

fn invalid() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "workspace_mutation_invalid_arguments",
        "workspace mutation arguments are invalid",
        false,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "workspace_mutation_cancelled",
        "workspace mutation was cancelled",
        false,
    )
}

fn name(kind: Kind) -> &'static str {
    match kind {
        Kind::Write => crate::WRITE_FILE_TOOL_NAME,
        Kind::Edit => crate::EDIT_FILE_TOOL_NAME,
        Kind::Delete => crate::DELETE_FILE_TOOL_NAME,
        Kind::Copy => crate::COPY_FILE_TOOL_NAME,
        Kind::Rename => crate::RENAME_FILE_TOOL_NAME,
    }
}

impl Tool for WorkspaceMutationTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = self.primary.spec();
        for key in projection::path_keys(self.kind)
            .0
            .into_iter()
            .chain(std::iter::once(projection::path_keys(self.kind).1))
        {
            spec.input_schema["properties"][key]["description"] =
                "Primary-relative path or absolute path within an active workspace root".into();
        }
        spec
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.primary.prepare(call)
    }

    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        call: ToolCall,
    ) -> Result<PreparedToolCall, ToolError> {
        if call.name.as_str() != name(self.kind) {
            return Err(invalid());
        }
        let scope = self
            .contexts
            .snapshot_for_tool(context)
            .map_err(|_| unavailable())?;
        let projection: WorkspaceMutationProjection = project(
            &scope.snapshot().map_err(|_| unavailable())?,
            self.kind,
            &call.arguments,
        )?;
        check_scope(Some(&scope))?;
        Ok(PreparedToolCall::new(
            projection.capability(),
            projection.logical_arguments().clone(),
        ))
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        // These are bounded, read-only stamps. An unpolled old future may not
        // acquire a later registration or a later approval with reused call IDs.
        let scope = self.contexts.snapshot_for_tool(&context);
        let ticket = self
            .registry
            .as_ref()
            .map(|registry| registry.execution_ticket(&context, name(self.kind)));
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let scope = Arc::new(scope.map_err(|_| unavailable())?);
            let projection = project(
                &scope.snapshot().map_err(|_| unavailable())?,
                self.kind,
                &arguments,
            )?;
            if projection.logical_arguments() != &arguments {
                return Err(invalid());
            }
            let ticket = ticket
                .transpose()
                .map_err(crate::NativeFileApprovalError::tool)?;
            check_scope(Some(&scope))?;
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let source = projection
                .source()
                .map(WorkspaceMutationEndpoint::retain)
                .transpose();
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            check_scope(Some(&scope))?;
            let source = source?;
            let target = projection.target().retain();
            check_scope(Some(&scope))?;
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let target = target?;
            let approval = if let Some(registry) = &self.registry {
                Some(
                    registry
                        .claim_endpoints(
                            ticket.ok_or_else(unavailable)?,
                            &context,
                            name(self.kind),
                            &arguments,
                            source.as_ref(),
                            &target,
                            &cancellation,
                        )
                        .map_err(crate::NativeFileApprovalError::tool)?,
                )
            } else {
                None
            };
            // Every inner future receives an already consumed claim and never
            // constructs a new registry ticket after the outer future's stamp.
            macro_rules! execute {
                ($tool:expr) => {{
                    let mut tool = $tool.with_workspace_scope(scope);
                    if let Some(undo) = &self.undo {
                        tool = tool.with_undo_tracker(undo.clone());
                    }
                    if let Some(approval) = approval {
                        tool = tool.with_claimed_approval(approval);
                    }
                    tool.execute(context, arguments, cancellation).await
                }};
            }
            match self.kind {
                Kind::Write => execute!(crate::WriteFileTool::from_endpoint(target)),
                Kind::Edit => execute!(crate::EditFileTool::from_endpoint(target)),
                Kind::Delete => execute!(crate::DeleteFileTool::from_endpoint(target)),
                Kind::Copy => execute!(crate::CopyFileTool::from_endpoints(
                    source.ok_or_else(invalid)?,
                    target
                )),
                Kind::Rename => execute!(crate::RenameFileTool::from_endpoints(
                    source.ok_or_else(invalid)?,
                    target
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests;
