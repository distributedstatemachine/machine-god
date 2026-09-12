use std::sync::MutexGuard;

use super::{
    Arc, MAX_MCP_STDIO_RUNTIMES, McpStdioConnection, McpStdioError, McpSubmissionRuntime, Result,
};

/// An inert staged replacement. The caller must not await while holding it.
pub(crate) struct PreparedStdioRuntimeSet<'a> {
    admitted: MutexGuard<'a, Box<[Arc<McpSubmissionRuntime>]>>,
    replacement: Box<[Arc<McpSubmissionRuntime>]>,
}

impl PreparedStdioRuntimeSet<'_> {
    pub(crate) fn commit(mut self) -> Box<[Arc<McpSubmissionRuntime>]> {
        std::mem::replace(&mut *self.admitted, self.replacement)
    }
}

pub(super) fn prepare(
    connection: &McpStdioConnection,
    runtimes: Vec<Arc<McpSubmissionRuntime>>,
) -> Result<PreparedStdioRuntimeSet<'_>> {
    if runtimes.len() > MAX_MCP_STDIO_RUNTIMES {
        return Err(McpStdioError::Capacity);
    }
    let replacement = runtimes.into_boxed_slice();
    connection.shared.check()?;
    let admitted = connection
        .shared
        .runtimes
        .lock()
        .map_err(|_| McpStdioError::Closed)?;
    connection.shared.check()?;
    Ok(PreparedStdioRuntimeSet {
        admitted,
        replacement,
    })
}
