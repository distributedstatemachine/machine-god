use super::NativeMcpOwnedPeer;
use super::{NativeMcpRuntime, NativeMcpRuntimeClock, NativeMcpRuntimeError as Error, Result};
use crate::mcp::{
    catalog::McpToolDescriptor,
    context::{NativeMcpContexts, NativeMcpTurnContext},
    permission::NativeMcpPermissionAuthority,
    protocol::{NegotiatedProtocol, RpcId},
    submission::{
        McpSubmissionRuntime, McpSubmissionRuntimeOwner, McpToolCallOptions, McpToolRequest,
    },
};
use futures_util::{
    future::{Either, select},
    lock::{Mutex, MutexGuard},
};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionInvocation, PermissionRequest,
    ToolName, ToolSpec,
};
use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) struct ServerRoute {
    pub name: Arc<str>,
    pub catalog_epoch: Instant,
    pub protocol: NegotiatedProtocol,
    pub peer: Mutex<NativeMcpOwnedPeer>,
    pub cancellation: CancellationToken,
    pub pending: AtomicUsize,
    pub max_pending: usize,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    pub timeout: Duration,
    pub authority_cancellations: Arc<[CancellationToken]>,
}
pub(super) struct ToolRoute {
    pub name: ToolName,
    pub spec: ToolSpec,
    pub descriptor: McpToolDescriptor,
    pub server: Weak<ServerRoute>,
    pub binding: Arc<McpSubmissionRuntime>,
    pub owner: McpSubmissionRuntimeOwner,
    pub contexts: Arc<NativeMcpContexts>,
    pub executor: Arc<dyn super::NativeMcpToolExecutor>,
    pub policy: super::NativeMcpToolExecutionPolicy,
}
struct Pending<'a>(&'a AtomicUsize);
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) struct PeerGuard<'a> {
    pub peer: MutexGuard<'a, NativeMcpOwnedPeer>,
    pub deadline: Instant,
    _pending: Pending<'a>,
}
impl ServerRoute {
    pub(super) fn check_authority(&self) -> Result<()> {
        if self.authority_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    fn authority_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
            || self
                .authority_cancellations
                .iter()
                .any(CancellationToken::is_cancelled)
    }

    async fn cancelled(&self) {
        let observers: Vec<_> = std::iter::once(&self.cancellation)
            .chain(self.authority_cancellations.iter())
            .map(|token| Box::pin(token.cancelled()))
            .collect();
        futures_util::future::select_all(observers).await;
    }

    pub async fn acquire<'a>(
        &'a self,
        context: &NativeMcpTurnContext,
        cancellation: &CancellationToken,
    ) -> Result<PeerGuard<'a>> {
        context.revalidate().map_err(|_| Error::Unavailable)?;
        self.check_authority()?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < self.max_pending).then_some(count + 1)
            })
            .map_err(|_| Error::Limit)?;
        let pending = Pending(&self.pending);
        let deadline = self
            .clock
            .now()
            .checked_add(self.timeout)
            .ok_or(Error::Limit)?;
        let cancelled = async {
            select(
                Box::pin(async {
                    select(cancellation.cancelled(), context.cancelled()).await;
                }),
                Box::pin(async {
                    select(Box::pin(self.cancelled()), self.clock.sleep_until(deadline)).await;
                }),
            )
            .await;
        };
        let guard = match select(Box::pin(self.peer.lock()), Box::pin(cancelled)).await {
            Either::Left((guard, _)) => guard,
            Either::Right(_) => return Err(Error::Cancelled),
        };
        context.revalidate().map_err(|_| Error::Unavailable)?;
        self.check_authority()?;
        if cancellation.is_cancelled() || self.clock.now() >= deadline {
            return Err(Error::Cancelled);
        }
        Ok(PeerGuard {
            peer: guard,
            deadline,
            _pending: pending,
        })
    }
}
impl NativeMcpPermissionAuthority for NativeMcpRuntime {
    fn resolve<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        context: &'a NativeMcpTurnContext,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Option<McpToolRequest>, PermissionError>> {
        Box::pin(async move {
            // Re-select the actual turn through the router; caller-provided IDs
            // and a context from a different live registry are not authority.
            let actual = self
                .contexts
                .snapshot_for_permission(request)
                .map_err(|_| permission_error())?;
            let registry = actual.registry().map_err(|_| permission_error())?;
            if !Arc::ptr_eq(
                &registry,
                &context.registry().map_err(|_| permission_error())?,
            ) {
                return Err(permission_error());
            }
            let publication = self
                .for_turn(&registry)
                .map_err(|_| permission_error())?
                .ok_or_else(permission_error)?;
            let Some(tool) = publication.tools.get(invocation.tool_name) else {
                return Ok(None);
            };
            let server = tool.server.upgrade().ok_or_else(permission_error)?;
            let mut peer = server
                .acquire(context, &cancellation)
                .await
                .map_err(|_| permission_error())?;
            publication.check().map_err(|_| permission_error())?;
            let reservation = peer.peer.reserve().map_err(|_| permission_error())?;
            let RpcId::Integer(id) = reservation.rpc_id() else {
                return Err(permission_error());
            };
            let mut options =
                McpToolCallOptions::new(server.protocol, *id).map_err(|_| permission_error())?;
            options = options.with_elicitation(tool.policy.form, tool.policy.url);
            if tool.policy.progress {
                options = options
                    .with_progress_token(u64::try_from(*id).map_err(|_| permission_error())?);
            }
            let projection = McpToolRequest::new(
                tool.binding.clone(),
                tool.descriptor.input_schema(),
                invocation,
                options,
            )
            .map_err(|_| permission_error())?
            .with_reservation(reservation)
            .map_err(|_| permission_error())?;
            #[cfg(feature = "mcp-http")]
            let projection = if let NativeMcpOwnedPeer::Http(http) = &*peer.peer {
                let head = http.request_head().map_err(|_| permission_error())?;
                projection
                    .with_http_head(&head)
                    .map_err(|_| permission_error())?
            } else {
                projection
            };
            context.revalidate().map_err(|_| permission_error())?;
            publication.check().map_err(|_| permission_error())?;
            if cancellation.is_cancelled() {
                return Err(permission_error());
            }
            Ok(Some(projection))
        })
    }
}
fn permission_error() -> PermissionError {
    PermissionError::new(
        "mcp_runtime_unavailable",
        "The exact MCP runtime route is unavailable",
    )
}
