use super::{NativeMcpRuntime, NativeMcpRuntimeError, candidate::Publication, route::ServerRoute};
use crate::{
    McpFeatureRequest,
    mcp::{
        control::{McpFeatureControlAuthority, McpFeatureReply},
        feature::McpFeatureCodecError,
    },
};
use machine_god_core::{CancellationToken, ToolContext};
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

mod exchange;

/// Fixed failures; peer names, credentials, arguments and content stay redacted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpFeatureError {
    Runtime(NativeMcpRuntimeError),
    Codec(McpFeatureCodecError),
}
impl fmt::Display for NativeMcpFeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native MCP feature operation unavailable")
    }
}
impl std::error::Error for NativeMcpFeatureError {}
impl From<NativeMcpRuntimeError> for NativeMcpFeatureError {
    fn from(error: NativeMcpRuntimeError) -> Self {
        Self::Runtime(error)
    }
}
impl From<McpFeatureCodecError> for NativeMcpFeatureError {
    fn from(error: McpFeatureCodecError) -> Self {
        Self::Codec(error)
    }
}
pub(super) type Result<T> = std::result::Result<T, NativeMcpFeatureError>;

/// Complete data with the original native selection, not a new current route.
/// Keeping a result retains its bounded operation slot through projection/archive.
pub struct NativeMcpFeatureResult {
    reply: McpFeatureReply,
    authority: McpFeatureControlAuthority,
    _operation: FeatureOperation,
}
impl fmt::Debug for NativeMcpFeatureResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpFeatureResult { <redacted> }")
    }
}
impl NativeMcpFeatureResult {
    /// Data only. This accessor makes no freshness or authority assertion.
    #[must_use]
    pub fn reply(&self) -> &McpFeatureReply {
        &self.reply
    }

    /// # Errors
    /// Rejects retirement of the exact original command/turn or native generation.
    pub fn revalidate(&self) -> Result<()> {
        self.authority
            .is_live()
            .then_some(())
            .ok_or_else(|| NativeMcpRuntimeError::Cancelled.into())
    }

    #[must_use]
    pub fn cancelled(&self) -> machine_god_core::BoxFuture<'static, ()> {
        self.authority.cancelled()
    }
}

/// An explicitly owned human command lifetime, never a fabricated model turn.
/// It weakly retains its runtime and cancels pending work when closed or dropped.
pub struct NativeMcpHumanCommand {
    runtime: Weak<NativeMcpRuntime>,
    cancellation: CancellationToken,
}
impl fmt::Debug for NativeMcpHumanCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpHumanCommand { <redacted> }")
    }
}
impl NativeMcpHumanCommand {
    pub fn close(&self) {
        self.cancellation.cancel();
    }

    /// Selects an exact currently published server only when polled. The request
    /// is data; retained native runtime ownership supplies the effect authority.
    /// # Errors
    /// Rejects closed commands/runtimes, unknown servers and bounded protocol failures.
    pub async fn feature(
        &self,
        request: &McpFeatureRequest,
        cancellation: CancellationToken,
    ) -> Result<NativeMcpFeatureResult> {
        let runtime = self
            .runtime
            .upgrade()
            .ok_or(NativeMcpRuntimeError::Unavailable)?;
        let publication = runtime.feature_publication()?;
        let server = selected_server(&publication, request.server())?;
        let authority = McpFeatureControlAuthority::for_human(
            self.cancellation.clone(),
            cancellation,
            server.cancellation.clone(),
            publication.retired.clone(),
            server.authority_cancellations.clone(),
        )?;
        runtime
            .exchange_feature(&publication, &server, request, authority)
            .await
    }
}
impl Drop for NativeMcpHumanCommand {
    fn drop(&mut self) {
        self.close();
    }
}

impl NativeMcpRuntime {
    /// Inert command ownership. Does not select a server, read a clock, or acquire I/O.
    #[must_use]
    pub fn human_command(self: &Arc<Self>) -> NativeMcpHumanCommand {
        NativeMcpHumanCommand {
            runtime: Arc::downgrade(self),
            cancellation: CancellationToken::new(),
        }
    }

    /// Runs only in the exact registered model turn and its pinned publication.
    /// Full native responses are caller-owned data, not a 64 KiB model projection.
    /// # Errors
    /// Rejects foreign/retired contexts, unknown servers, cancellation and bounds.
    pub async fn feature_for_turn(
        &self,
        context: ToolContext,
        request: &McpFeatureRequest,
        cancellation: CancellationToken,
    ) -> Result<NativeMcpFeatureResult> {
        if cancellation.is_cancelled() {
            return Err(NativeMcpRuntimeError::Cancelled.into());
        }
        let context = Arc::new(
            self.contexts
                .snapshot_for_tool(&context)
                .map_err(|_| NativeMcpRuntimeError::Unavailable)?,
        );
        let registry = context
            .registry()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        let publication = self
            .for_turn(&registry)?
            .ok_or(NativeMcpRuntimeError::Unavailable)?;
        let server = selected_server(&publication, request.server())?;
        let authority = McpFeatureControlAuthority::for_model(
            context,
            cancellation,
            server.cancellation.clone(),
            publication.retired.clone(),
            server.authority_cancellations.clone(),
        )?;
        self.exchange_feature(&publication, &server, request, authority)
            .await
    }

    fn feature_publication(&self) -> Result<Arc<Publication>> {
        let state = self
            .state
            .lock()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        if state.closed {
            return Err(NativeMcpRuntimeError::Unavailable.into());
        }
        state
            .active
            .clone()
            .ok_or_else(|| NativeMcpRuntimeError::Unavailable.into())
    }

    async fn exchange_feature(
        &self,
        publication: &Publication,
        server: &ServerRoute,
        request: &McpFeatureRequest,
        authority: McpFeatureControlAuthority,
    ) -> Result<NativeMcpFeatureResult> {
        // Native feature admission has a separate finite transient budget. No
        // result/cache queue is retained after returning caller-owned data.
        let maximum = self.limits.max_pending_operations.min(2);
        self.feature_operations
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < maximum).then_some(count + 1)
            })
            .map_err(|_| NativeMcpRuntimeError::Limit)?;
        let operation = FeatureOperation(self.feature_operations.clone());
        let mut lane = server.acquire_feature(&authority).await?;
        publication.check()?;
        let reply = exchange::run(&mut lane, server, request, &authority).await?;
        publication.check()?;
        server.check_authority()?;
        if !authority.is_live() {
            return Err(NativeMcpRuntimeError::Cancelled.into());
        }
        Ok(NativeMcpFeatureResult {
            reply,
            authority,
            _operation: operation,
        })
    }
}

fn selected_server(publication: &Publication, name: &str) -> Result<Arc<ServerRoute>> {
    publication.check()?;
    let server = publication
        .servers
        .iter()
        .find(|server| server.name.as_ref() == name)
        .ok_or(NativeMcpRuntimeError::Unavailable)?;
    server.check_authority()?;
    Ok(server.clone())
}

struct FeatureOperation(Arc<AtomicUsize>);
impl Drop for FeatureOperation {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
