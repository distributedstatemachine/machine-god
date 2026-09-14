use super::{Arc, BoxFuture, CancellationToken, NativeAcpSession, NativeReferenceHost, Poll};
use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerSpawner};
use std::time::Duration;

pub(super) struct Receipt {
    pub complete: bool,
    pub workers: Vec<NativeOwnedWorkerCompletion>,
}

pub(super) fn retire(
    host: Arc<NativeReferenceHost>,
    session: Option<NativeAcpSession>,
    already_retired: bool,
    now_ms: i64,
) -> BoxFuture<'static, Receipt> {
    Box::pin(async move {
        let mut failed = false;
        if let Some(mut session) = session {
            if !already_retired {
                failed = session.request_close(&session.id()).is_err();
                if !failed {
                    failed = futures_util::future::poll_fn(|cx| {
                        let _ = session.poll_progress(cx, now_ms);
                        let _ = session.take_presentation();
                        let _ = session.take_outcome();
                        if session.shutdown_error().is_some() {
                            Poll::Ready(true)
                        } else if session.is_closed() {
                            Poll::Ready(false)
                        } else {
                            Poll::Pending
                        }
                    })
                    .await;
                }
            }
            drop(session);
        }
        host.close_mcp();
        match host.mcp_deadline_after(Duration::from_secs(30)) {
            Ok(deadline) => {
                failed |= host
                    .settle_mcp_ephemeral(deadline, CancellationToken::new())
                    .await
                    .is_err();
            }
            Err(_) => failed = true,
        }
        let completion = host.terminal_shutdown_completion();
        drop(host);
        let Some(completion) = completion else {
            return Receipt {
                complete: false,
                workers: Vec::new(),
            };
        };
        let observer = completion.clone();
        let joined = NativeOwnedWorkerSpawner::new()
            .run(move || observer.wait_on_worker())
            .await;
        Receipt {
            complete: !failed && matches!(joined, Ok(Ok(()))) && completion.is_complete(),
            workers: vec![completion],
        }
    })
}
