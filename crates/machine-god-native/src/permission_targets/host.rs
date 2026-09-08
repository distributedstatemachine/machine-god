use super::{NativePermissionTerminalResolution, NativePermissionTerminalResolver, invalid};
use crate::terminal_host_authority::CapturedTerminalHostAuthority;
use crate::terminal_native_launch::ResolvedTerminalNativeLaunch;
use crate::{NativeOwnedWorkerScope, TerminalActionInvocation};
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken, PermissionError, TerminalActionRequest};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// Non-owning host lifecycle: the real resource's stop token and closed worker
/// scope prevent this captured selection from authorizing work after shutdown.
pub(crate) struct HostPermissionResolver {
    host: Arc<CapturedTerminalHostAuthority>,
    workers: NativeOwnedWorkerScope,
    stop: CancellationToken,
    active: Arc<AtomicUsize>,
}
impl HostPermissionResolver {
    pub(crate) fn new(
        host: Arc<CapturedTerminalHostAuthority>,
        workers: NativeOwnedWorkerScope,
        stop: CancellationToken,
    ) -> Self {
        Self {
            host,
            workers,
            stop,
            active: Arc::new(AtomicUsize::new(0)),
        }
    }
}
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl NativePermissionTerminalResolver for HostPermissionResolver {
    fn resolve(
        &self,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionTerminalResolution, PermissionError>> {
        Box::pin(async move {
            let deadline = Instant::now() + Duration::from_secs(2);
            if cancellation.is_cancelled() || self.stop.is_cancelled() {
                return Err(invalid());
            }
            self.active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < 4).then_some(n + 1)
                })
                .map_err(|_| invalid())?;
            let permit = Permit(Arc::clone(&self.active));
            let effective = CancellationToken::new();
            let cancel_on_drop = CancelOnDrop(effective.clone());
            let host = Arc::clone(&self.host);
            let operation = self.workers.run(move || {
                let _permit = permit;
                let resolved = host
                    .resolve_on_worker(invocation, deadline, &effective)
                    .map_err(|_| invalid())?;
                let shell = match &resolved.request {
                    TerminalActionRequest::Exec { request } => {
                        Some(host.exec_shell(request).map_err(|_| invalid())?)
                    }
                    TerminalActionRequest::Start { request } => Some(
                        ResolvedTerminalNativeLaunch::resolve(&host.launch_config(), request)
                            .map_err(|_| invalid())?
                            .shell()
                            .clone(),
                    ),
                    _ => None,
                };
                if effective.is_cancelled() || Instant::now() >= deadline {
                    return Err(invalid());
                }
                let identity = host.identity();
                NativePermissionTerminalResolution::new(
                    resolved.request,
                    resolved.cwd.map(std::fs::File::from),
                    shell,
                    identity.environment_sha256.clone(),
                    identity.shell_selection_sha256.clone(),
                )
            });
            let stopped = select(cancellation.cancelled(), self.stop.cancelled());
            let result = match select(operation, stopped).await {
                Either::Left((result, _)) => result.map_err(|_| invalid())?,
                Either::Right(_) => return Err(invalid()),
            };
            drop(cancel_on_drop);
            if cancellation.is_cancelled() || self.stop.is_cancelled() || Instant::now() >= deadline
            {
                return Err(invalid());
            }
            result
        })
    }
}
