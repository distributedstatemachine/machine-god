//! Owner-side completion of accepted terminal input, without resubmission.
//! A cancelled or dropped caller never owns the queued suffix or its receipt.

use crate::terminal_input::{TerminalInputProgress, TerminalInputReceipt, TerminalWriterId};
use crate::terminal_registry::{TerminalRegistry, TerminalRegistryError};
use crate::terminal_session::TerminalSessionBackend;
use machine_god_core::{
    BackgroundOutputOwner, CancellationToken, TerminalActorRole, TerminalSessionId,
};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

pub(crate) const MAX_TERMINAL_WRITE_COMPLETIONS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalWriteIdentity {
    pub(crate) owner: BackgroundOutputOwner,
    pub(crate) session: TerminalSessionId,
    pub(crate) actor: TerminalActorRole,
    pub(crate) writer: TerminalWriterId,
}

/// Input commitment and subsequent state publication are independent facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalWriteReceipt {
    pub(crate) input: TerminalInputReceipt,
    pub(crate) publication_error: Option<TerminalRegistryError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalWriteError {
    Capacity,
    Closed,
    Cancelled,
    Effect(TerminalRegistryError),
    /// Last observed accepted bytes are retained; this is not final success.
    Unavailable {
        last: TerminalWriteReceipt,
    },
}
type Result<T> = std::result::Result<T, TerminalWriteError>;

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Reply {
    result: Option<Result<TerminalWriteReceipt>>,
    waker: Option<Waker>,
}
struct SharedReply {
    reply: Mutex<Reply>,
    abandoned: AtomicBool,
    _permit: Permit,
}
impl SharedReply {
    fn publish(&self, result: Result<TerminalWriteReceipt>) -> bool {
        let wake = {
            let mut reply = self
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            reply.result = Some(result);
            reply.waker.take()
        };
        wake.is_none_or(
            |wake| match catch_unwind(AssertUnwindSafe(|| wake.wake())) {
                Ok(()) => true,
                Err(payload) => {
                    // Opaque panic destructors must not interrupt other completions.
                    std::mem::forget(payload);
                    false
                }
            },
        )
    }
}
struct Registration {
    identity: TerminalWriteIdentity,
    last: TerminalWriteReceipt,
    reply: Arc<SharedReply>,
}

pub(crate) struct TerminalWriteCoordinator {
    registrations: Vec<Registration>,
    count: Arc<AtomicUsize>,
    closed: bool,
}
impl TerminalWriteCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            registrations: Vec::with_capacity(MAX_TERMINAL_WRITE_COMPLETIONS),
            count: Arc::new(AtomicUsize::new(0)),
            closed: false,
        }
    }

    /// Reserve before any effect. The trusted callback must authorize this
    /// exact identity and return a receipt even if post-write publication fails.
    /// The callback's profile guards must end before returning here.
    pub(crate) fn submit(
        &mut self,
        identity: TerminalWriteIdentity,
        cancellation: &CancellationToken,
        effect: impl FnOnce() -> std::result::Result<TerminalWriteReceipt, TerminalRegistryError>,
    ) -> Result<TerminalWriteFuture> {
        self.registrations
            .retain(|entry| !entry.reply.abandoned.load(Ordering::Acquire));
        if self.closed {
            return Err(TerminalWriteError::Closed);
        }
        if cancellation.is_cancelled() {
            return Err(TerminalWriteError::Cancelled);
        }
        if self.count.load(Ordering::Acquire) >= MAX_TERMINAL_WRITE_COMPLETIONS {
            return Err(TerminalWriteError::Capacity);
        }
        self.count.fetch_add(1, Ordering::AcqRel);
        let permit = Permit(Arc::clone(&self.count));
        // The permit is already held if allocation or the effect unwinds.
        let last = effect().map_err(TerminalWriteError::Effect)?;
        let reply = Arc::new(SharedReply {
            reply: Mutex::new(Reply {
                result: None,
                waker: None,
            }),
            abandoned: AtomicBool::new(false),
            _permit: permit,
        });
        if !valid_receipt(last.input) {
            reply.publish(Err(TerminalWriteError::Unavailable { last }));
        } else if last.input.progress == TerminalInputProgress::Pending {
            self.registrations.push(Registration {
                identity,
                last,
                reply: Arc::clone(&reply),
            });
        } else {
            reply.publish(Ok(last));
        }
        Ok(TerminalWriteFuture { reply: Some(reply) })
    }

    /// Must run after pumping and after the final shutdown attempt, outside all
    /// profile guards. No terminal input is submitted or cancelled by observing.
    pub(crate) fn observe<B: TerminalSessionBackend>(
        &mut self,
        registry: &TerminalRegistry<B>,
    ) -> bool {
        self.observe_with(|identity, operation| {
            registry.write_receipt(
                &identity.owner,
                &identity.session,
                identity.actor,
                identity.writer,
                operation,
            )
        })
    }

    fn observe_with(
        &mut self,
        mut observe: impl FnMut(
            &TerminalWriteIdentity,
            std::num::NonZeroU64,
        )
            -> std::result::Result<TerminalInputReceipt, TerminalRegistryError>,
    ) -> bool {
        let mut woke = true;
        self.registrations.retain_mut(|entry| {
            if entry.reply.abandoned.load(Ordering::Acquire) {
                return false;
            }
            let previous = entry.last.input;
            let observed = observe(
                &entry.identity,
                previous.operation_id.expect("pending operation"),
            );
            let result = match observed {
                Ok(receipt)
                    if valid_receipt(receipt)
                        && receipt.operation_id == previous.operation_id
                        && receipt.encoded_bytes == previous.encoded_bytes
                        && receipt.accepted_bytes >= previous.accepted_bytes =>
                {
                    entry.last.input = receipt;
                    if receipt.progress == TerminalInputProgress::Pending {
                        return true;
                    }
                    Ok(entry.last)
                }
                _ => Err(TerminalWriteError::Unavailable { last: entry.last }),
            };
            woke &= entry.reply.publish(result);
            false
        });
        woke
    }

    /// Call only after observing shutdown's quiesced input. Unresolved sessions
    /// produce an explicit failure with their last known accepted counts.
    pub(crate) fn close(&mut self) -> bool {
        self.closed = true;
        let mut woke = true;
        for entry in self.registrations.drain(..) {
            woke &= entry
                .reply
                .publish(Err(TerminalWriteError::Unavailable { last: entry.last }));
        }
        woke
    }
}
impl Drop for TerminalWriteCoordinator {
    fn drop(&mut self) {
        self.close();
    }
}

fn valid_receipt(receipt: TerminalInputReceipt) -> bool {
    receipt.accepted_bytes <= receipt.encoded_bytes
        && (receipt.operation_id.is_some()
            || (receipt.encoded_bytes == 0 && receipt.progress == TerminalInputProgress::Complete))
        && (receipt.progress != TerminalInputProgress::Complete
            || receipt.accepted_bytes == receipt.encoded_bytes)
}

pub(crate) struct TerminalWriteFuture {
    reply: Option<Arc<SharedReply>>,
}
impl Future for TerminalWriteFuture {
    type Output = Result<TerminalWriteReceipt>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let shared = self.reply.as_ref().expect("completed write future");
        let incoming = context.waker().clone();
        let (result, old) = {
            let mut reply = shared
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = reply.result.take();
            let old = if result.is_some() {
                Some(incoming)
            } else {
                reply.waker.replace(incoming)
            };
            (result, old)
        };
        drop(old);
        match result {
            None => Poll::Pending,
            Some(result) => {
                self.reply.take();
                Poll::Ready(result)
            }
        }
    }
}
impl Drop for TerminalWriteFuture {
    fn drop(&mut self) {
        if let Some(shared) = self.reply.take() {
            shared.abandoned.store(true, Ordering::Release);
            let old = shared
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .waker
                .take();
            drop(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId};
    use std::num::NonZeroU64;

    fn identity() -> TerminalWriteIdentity {
        TerminalWriteIdentity {
            owner: BackgroundOutputOwner::new(
                SessionId::new("owner").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            ),
            session: TerminalSessionId::new("terminal").unwrap(),
            actor: TerminalActorRole::Agent,
            writer: TerminalWriterId::new(NonZeroU64::new(1).unwrap()),
        }
    }
    fn receipt(accepted: usize, progress: TerminalInputProgress) -> TerminalWriteReceipt {
        TerminalWriteReceipt {
            input: TerminalInputReceipt {
                operation_id: NonZeroU64::new(1),
                accepted_bytes: accepted,
                encoded_bytes: 8,
                progress,
            },
            publication_error: None,
        }
    }
    fn poll(future: &mut TerminalWriteFuture) -> Poll<Result<TerminalWriteReceipt>> {
        Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn delayed_write_submits_once_and_cancel_preserves_final_accepted_count() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let cancellation = CancellationToken::new();
        let mut effects = 0;
        let mut future = coordinator
            .submit(identity(), &cancellation, || {
                effects += 1;
                Ok(receipt(2, TerminalInputProgress::Pending))
            })
            .unwrap();
        cancellation.cancel();
        for accepted in [2, 4, 6] {
            assert!(poll(&mut future).is_pending());
            assert!(coordinator.observe_with(|who, operation| {
                assert_eq!(*who, identity());
                assert_eq!(operation.get(), 1);
                Ok(receipt(accepted, TerminalInputProgress::Pending).input)
            }));
        }
        assert!(
            coordinator.observe_with(|_, _| Ok(receipt(8, TerminalInputProgress::Complete).input))
        );
        assert_eq!(
            poll(&mut future),
            Poll::Ready(Ok(receipt(8, TerminalInputProgress::Complete)))
        );
        assert_eq!(effects, 1);
        assert_eq!(coordinator.count.load(Ordering::Acquire), 0);
    }

    #[test]
    fn reservation_covers_effect_and_unconsumed_results() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let count = Arc::clone(&coordinator.count);
        let mut futures = Vec::new();
        for i in 1..=MAX_TERMINAL_WRITE_COMPLETIONS {
            futures.push(
                coordinator
                    .submit(identity(), &CancellationToken::new(), || {
                        assert_eq!(count.load(Ordering::Acquire), i);
                        Ok(receipt(8, TerminalInputProgress::Complete))
                    })
                    .unwrap(),
            );
        }
        assert!(matches!(
            coordinator.submit(identity(), &CancellationToken::new(), || panic!(
                "full must precede effect"
            )),
            Err(TerminalWriteError::Capacity)
        ));
        assert!(poll(&mut futures[0]).is_ready());
        assert!(
            coordinator
                .submit(identity(), &CancellationToken::new(), || Ok(receipt(
                    8,
                    TerminalInputProgress::Complete
                )))
                .is_ok()
        );
        drop(futures);
        assert_eq!(count.load(Ordering::Acquire), 0);
    }

    #[test]
    fn drop_abandons_observation_without_touching_input_and_frees_capacity() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let future = coordinator
            .submit(identity(), &CancellationToken::new(), || {
                Ok(receipt(2, TerminalInputProgress::Pending))
            })
            .unwrap();
        drop(future);
        assert!(coordinator.observe_with(|_, _| panic!("abandoned observer")));
        assert_eq!(coordinator.count.load(Ordering::Acquire), 0);
    }

    #[test]
    fn failed_effect_and_pre_cancel_release_reserved_slot() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let cancellation = CancellationToken::new();
        assert!(matches!(
            coordinator.submit(identity(), &cancellation, || Err(
                TerminalRegistryError::NotFound
            )),
            Err(TerminalWriteError::Effect(TerminalRegistryError::NotFound))
        ));
        cancellation.cancel();
        assert!(matches!(
            coordinator.submit(identity(), &cancellation, || panic!("cancelled")),
            Err(TerminalWriteError::Cancelled)
        ));
        assert_eq!(coordinator.count.load(Ordering::Acquire), 0);
    }

    #[test]
    fn observation_error_or_rewind_keeps_last_committed_receipt() {
        for invalid in [
            None,
            Some(receipt(1, TerminalInputProgress::Pending).input),
            Some(TerminalInputReceipt {
                operation_id: NonZeroU64::new(2),
                ..receipt(8, TerminalInputProgress::Complete).input
            }),
        ] {
            let mut coordinator = TerminalWriteCoordinator::new();
            let last = receipt(4, TerminalInputProgress::Pending);
            let mut future = coordinator
                .submit(identity(), &CancellationToken::new(), || Ok(last))
                .unwrap();
            coordinator.observe_with(|_, _| invalid.ok_or(TerminalRegistryError::NotFound));
            assert_eq!(
                poll(&mut future),
                Poll::Ready(Err(TerminalWriteError::Unavailable { last }))
            );
        }
    }

    #[test]
    fn shutdown_observes_final_closed_bytes_and_preserves_publication_failure() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let mut last = receipt(2, TerminalInputProgress::Pending);
        last.publication_error = Some(TerminalRegistryError::Invalid);
        let mut future = coordinator
            .submit(identity(), &CancellationToken::new(), || Ok(last))
            .unwrap();
        coordinator.observe_with(|_, _| Ok(receipt(5, TerminalInputProgress::Closed).input));
        coordinator.close();
        last.input = receipt(5, TerminalInputProgress::Closed).input;
        assert_eq!(poll(&mut future), Poll::Ready(Ok(last)));
        assert!(matches!(
            coordinator.submit(identity(), &CancellationToken::new(), || panic!(
                "closed effect"
            )),
            Err(TerminalWriteError::Closed)
        ));
    }

    struct WakeCheck {
        reply: std::sync::Weak<SharedReply>,
        calls: AtomicUsize,
        panic: bool,
    }
    impl std::task::Wake for WakeCheck {
        fn wake(self: Arc<Self>) {
            assert!(
                self.reply
                    .upgrade()
                    .unwrap()
                    .reply
                    .try_lock()
                    .unwrap()
                    .result
                    .is_some()
            );
            self.calls.fetch_add(1, Ordering::AcqRel);
            assert!(!self.panic, "test wake panic");
        }
    }
    #[test]
    fn publication_releases_reply_lock_and_contains_each_waker_panic() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let mut futures = Vec::new();
        let mut callbacks = Vec::new();
        for panic in [true, false] {
            let mut future = coordinator
                .submit(identity(), &CancellationToken::new(), || {
                    Ok(receipt(2, TerminalInputProgress::Pending))
                })
                .unwrap();
            let wake = Arc::new(WakeCheck {
                reply: Arc::downgrade(future.reply.as_ref().unwrap()),
                calls: AtomicUsize::new(0),
                panic,
            });
            let waker = Waker::from(Arc::clone(&wake));
            assert!(
                Pin::new(&mut future)
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            futures.push(future);
            callbacks.push(wake);
        }
        assert!(
            !coordinator.observe_with(|_, _| Ok(receipt(8, TerminalInputProgress::Complete).input))
        );
        for (future, wake) in futures.iter_mut().zip(callbacks) {
            assert_eq!(wake.calls.load(Ordering::Acquire), 1);
            assert_eq!(
                poll(future),
                Poll::Ready(Ok(receipt(8, TerminalInputProgress::Complete)))
            );
        }
    }

    #[test]
    fn dropping_owner_settles_unknown_completion_explicitly() {
        let mut coordinator = TerminalWriteCoordinator::new();
        let last = receipt(3, TerminalInputProgress::Pending);
        let mut future = coordinator
            .submit(identity(), &CancellationToken::new(), || Ok(last))
            .unwrap();
        drop(coordinator);
        assert_eq!(
            poll(&mut future),
            Poll::Ready(Err(TerminalWriteError::Unavailable { last }))
        );
    }
}
