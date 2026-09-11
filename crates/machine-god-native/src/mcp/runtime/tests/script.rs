use super::*;
use crate::mcp::submission::{
    McpPendingToolReservation, McpSubmission, McpSubmissionRuntime, McpSubmissionWriter,
    McpToolReservation,
};
use std::{
    io,
    sync::atomic::AtomicBool,
    task::{Context, Poll},
};

type Response = dyn Fn(i64) -> Box<[u8]> + Send + Sync;

/// Test-only concrete peer. Exercises real marker custody and native proof
/// writing, without exposing an arbitrary production peer/callback interface.
pub struct ScriptPeer {
    pub(in crate::mcp::runtime) tools: bool,
    pub(in crate::mcp::runtime) runtimes: Vec<Arc<McpSubmissionRuntime>>,
    pending: McpPendingToolReservation,
    next: i64,
    pub(in crate::mcp::runtime) closed: Arc<AtomicBool>,
    writes: Arc<Mutex<Vec<u8>>>,
    response: Option<Arc<Response>>,
}
impl std::fmt::Debug for ScriptPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScriptPeer { <redacted> }")
    }
}
impl ScriptPeer {
    pub(super) fn new(writes: Arc<Mutex<Vec<u8>>>) -> Self {
        Self {
            tools: true,
            runtimes: vec![],
            pending: McpPendingToolReservation::default(),
            next: 1,
            closed: Arc::new(AtomicBool::new(false)),
            writes,
            response: None,
        }
    }
    pub(super) fn with_response(
        mut self,
        response: impl Fn(i64) -> Box<[u8]> + Send + Sync + 'static,
    ) -> Self {
        self.response = Some(Arc::new(response));
        self
    }
    pub(in crate::mcp::runtime) fn reserve(&mut self) -> Result<McpToolReservation> {
        if self.closed.load(Ordering::Acquire) {
            return Err(NativeMcpRuntimeError::Unavailable);
        }
        let id = RpcId::Integer(self.next);
        self.next += 1;
        self.pending.reserve(id).ok_or(NativeMcpRuntimeError::Limit)
    }
    pub(in crate::mcp::runtime) async fn call(
        &mut self,
        submission: McpSubmission,
    ) -> Result<Box<[u8]>> {
        if !self
            .runtimes
            .iter()
            .any(|runtime| submission.belongs_to_runtime(runtime))
            || self.closed.load(Ordering::Acquire)
        {
            return Err(NativeMcpRuntimeError::Unavailable);
        }
        let id = self
            .pending
            .take(submission.rpc_id(), submission.tool_reservation())
            .ok_or(NativeMcpRuntimeError::Invalid)?;
        submission
            .into_writer(Writer(self.writes.clone()))
            .await
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        let RpcId::Integer(id) = id else { panic!() };
        if let Some(response) = &self.response {
            return Ok(response(id));
        }
        Ok(format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete","content":[{{"type":"text","text":"fixture"}}]}}}}"#).into_bytes().into())
    }
    pub(in crate::mcp::runtime) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}
impl Drop for ScriptPeer {
    fn drop(&mut self) {
        self.close();
    }
}
struct Writer(Arc<Mutex<Vec<u8>>>);
impl McpSubmissionWriter for Writer {
    fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
