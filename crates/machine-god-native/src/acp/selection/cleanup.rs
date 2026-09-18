use super::{Arc, BoxFuture, CancellationToken, NativeAcpSession, NativeReferenceHost, Poll};
use crate::NativeOwnedWorkerCompletion;
use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    #[test]
    fn managed_retirement_recovers_clear_failure_without_a_client_prompt() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (fixture, mut owner) =
                    crate::interactive_session::tests::managed_recovery::prepared().await;
                let blocked = fixture.fence_notice_clear(&mut owner).await;
                let providers = fixture.transport.requests().len();
                let session = NativeAcpSession::from_interactive(owner, false);
                let operation = retire(fixture.host.clone(), Some(session), false, 102);
                drop(fixture.host);
                let mut operation = std::pin::pin!(operation);
                let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                assert!(operation.as_mut().poll(&mut cx).is_pending());
                drop(blocked);
                let receipt = tokio::time::timeout(Duration::from_secs(10), operation)
                    .await
                    .unwrap();
                assert!(receipt.complete);
                assert_eq!(fixture.transport.requests().len(), providers);
            });
    }
}

pub(super) struct Receipt {
    pub complete: bool,
    pub workers: Vec<NativeOwnedWorkerCompletion>,
    // A failed pre-selection cleanup must retain the original manager and stage.
    pub managed: Option<Box<super::managed::Preparation>>,
    pub staged: Option<Box<super::reuse::Stage>>,
}

pub(super) fn reject_stage(stage: Box<super::reuse::Stage>) -> BoxFuture<'static, Receipt> {
    Box::pin(async move {
        match stage.settle().await {
            Ok(()) => Receipt {
                complete: true,
                workers: Vec::new(),
                managed: None,
                staged: None,
            },
            Err(stage) => Receipt {
                complete: false,
                workers: Vec::new(),
                managed: None,
                staged: Some(stage),
            },
        }
    })
}

pub(super) fn reject(
    mut host: super::NativeAcpPreparedHost,
    session: Option<NativeAcpSession>,
    now_ms: i64,
) -> BoxFuture<'static, Receipt> {
    Box::pin(async move {
        if let Some(managed) = host.managed.take()
            && let Err(managed) = managed.settle(now_ms).await
        {
            return Receipt {
                complete: false,
                workers: Vec::new(),
                managed: Some(managed),
                staged: None,
            };
        }
        retire(host.host, session, false, now_ms).await
    })
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
            failed = if already_retired {
                session.close_retired().is_err()
            } else {
                session.request_close(&session.id()).is_err()
            };
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
            drop(session);
        }
        host.close_mcp();
        if !host.managed_agents_selected() {
            match host.mcp_deadline_after(Duration::from_secs(30)) {
                Ok(deadline) => {
                    failed |= host
                        .settle_mcp_ephemeral(deadline, CancellationToken::new())
                        .await
                        .is_err();
                }
                Err(_) => failed = true,
            }
        }
        let completion = host.terminal_shutdown_completion();
        drop(host);
        let Some(completion) = completion else {
            return Receipt {
                complete: false,
                workers: Vec::new(),
                managed: None,
                staged: None,
            };
        };
        // Retirement must not need a fresh worker admission: the collector can
        // be full while the exact host's final worker or reap is still running.
        completion.wait().await;
        Receipt {
            complete: !failed && completion.is_complete(),
            workers: vec![completion],
            managed: None,
            staged: None,
        }
    })
}
