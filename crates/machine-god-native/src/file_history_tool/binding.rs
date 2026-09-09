//! Native-only invocation binding for the five trusted mutation backends.
//!
//! Capture is a read-only in-memory stamp, including denial/error. It creates
//! no inner execution future, claims no proof and observes no filesystem state.
//! Binding consumes that exact stamp only after deferred history admission.

use crate::file_approval::{NativeFileApprovalClaim, NativeFileApprovalError};
use machine_god_core::{CancellationToken, Tool, ToolContext, ToolError};
use serde_json::Value;
use std::sync::Arc;

pub(crate) type MutationStamp = Option<Result<NativeFileApprovalClaim, NativeFileApprovalError>>;

pub(crate) trait MutationBinding: Send + Sync {
    fn capture(&self, context: &ToolContext) -> MutationStamp;

    fn bind(
        &self,
        context: &ToolContext,
        arguments: &Value,
        cancellation: &CancellationToken,
        stamp: MutationStamp,
    ) -> Result<Option<Arc<dyn Tool>>, ToolError>;
}

macro_rules! mutation_binding {
    ($($tool:ty),+ $(,)?) => {$(
        impl MutationBinding for $tool {
            fn capture(&self, context: &ToolContext) -> MutationStamp {
                self.approval_ticket(context)
            }

            fn bind(
                &self,
                context: &ToolContext,
                arguments: &Value,
                cancellation: &CancellationToken,
                stamp: MutationStamp,
            ) -> Result<Option<Arc<dyn Tool>>, ToolError> {
                self.approval_bound(context, arguments, cancellation, stamp)
                    .map(|bound| bound.map(|tool| Arc::new(tool) as Arc<dyn Tool>))
            }
        }
    )+};
}

mutation_binding!(
    crate::WriteFileTool,
    crate::EditFileTool,
    crate::DeleteFileTool,
    crate::CopyFileTool,
    crate::RenameFileTool,
);
