//! Bounded worker-owned attention waits. No process, monitor, input, filesystem,
//! clock or thread authority is acquired here.
//!
//! Observation and publication are deliberately separate. An owner observes
//! committed session state, obtains completion tokens, persists each token's
//! attention transition, drops its profile transaction, and only then publishes
//! the reply. Dropping a token makes the same frozen receipt available again.

use std::future::Future;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use machine_god_core::{
    BackgroundOutputOwner, CancellationToken, Cancelled, TerminalActorRole, TerminalCursor,
    TerminalReturnCondition, TerminalSessionId, TerminalWaitRequest,
};

use crate::terminal_input::TerminalWriterId;
#[cfg(test)]
use crate::terminal_monitor::MAX_MONITOR_FEED_BYTES;
use crate::terminal_monitor::{
    TerminalMonitorContext, TerminalMonitorError, TerminalProcessOutcome, TerminalWaitOutcome,
    TerminalWaitState,
};

pub(crate) const MAX_TERMINAL_WAITS: usize = 32;
/// A 4096-byte pattern needs at most its last 4095 historical bytes to match
/// across the retained-history/live-output boundary.
pub(crate) const MAX_WAIT_HISTORY_SUFFIX: usize = 4095;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalWaitId(NonZeroU64);

/// Trusted host identity, never decoded from tool arguments. Session lookup
/// and attention admission must authorize these identities before registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalWaitIdentity {
    pub(crate) owner: BackgroundOutputOwner,
    pub(crate) session: TerminalSessionId,
    pub(crate) actor: TerminalActorRole,
    pub(crate) writer: TerminalWriterId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalWaitError {
    Capacity,
    Closed,
    Cancelled,
    NotFound,
    Invalid,
    Observation(TerminalMonitorError),
}
type Result<T> = std::result::Result<T, TerminalWaitError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalWaitAttentionError {
    Unavailable,
}

/// The observed condition and durable attention cleanup are separate facts.
/// In particular, shutdown cannot erase a condition already met, nor turn a
/// failed durable transition into a successful attention receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalWaitReceipt {
    pub(crate) outcome: TerminalWaitOutcome,
    pub(crate) attention_error: Option<TerminalWaitAttentionError>,
}

pub(crate) struct TerminalWaitObservation {
    pub(crate) id: TerminalWaitId,
    pub(crate) identity: TerminalWaitIdentity,
    pub(crate) cursor: TerminalCursor,
}

/// History scanning belongs to the owner before admission, not this bounded
/// registration operation. The suffix must end at the supplied committed
/// cursor; `matched` records an earlier match in validated retained history.
#[derive(Clone, Copy, Default)]
pub(crate) struct TerminalWaitHistory<'a> {
    pub(crate) matched: bool,
    pub(crate) suffix: &'a [u8],
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Reply {
    result: Option<TerminalWaitReceipt>,
    waker: Option<Waker>,
}
struct SharedReply {
    residency: OnceLock<crate::terminal_registry::TerminalResidentLease>,
    reply: Mutex<Reply>,
    abandoned: AtomicBool,
    published: AtomicBool,
    token_outstanding: AtomicBool,
    _permit: Permit,
}
struct Registration {
    id: TerminalWaitId,
    identity: TerminalWaitIdentity,
    state: TerminalWaitState,
    cancellation: CancellationToken,
    reply: Arc<SharedReply>,
    ready: Option<TerminalWaitOutcome>,
    now_ms: i64,
    last_output_ms: i64,
    deadline_ms: i64,
    cursor: TerminalCursor,
}

/// Only the owner worker mutates registrations. Reply mutexes contain no
/// session state and are never held across caller waker operations.
pub(crate) struct TerminalWaitCoordinator {
    registrations: Vec<Registration>,
    count: Arc<AtomicUsize>,
    next_id: Option<NonZeroU64>,
    closed: bool,
}
impl TerminalWaitCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            registrations: Vec::with_capacity(MAX_TERMINAL_WAITS),
            count: Arc::new(AtomicUsize::new(0)),
            next_id: NonZeroU64::new(1),
            closed: false,
        }
    }

    /// This is a synchronous, effect-free owner request, not a wait for the
    /// condition. Only its returned future waits. Callers persist attention
    /// admission before making that future visible outside the owner job.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit trusted registration inputs"
    )]
    pub(crate) fn register(
        &mut self,
        identity: TerminalWaitIdentity,
        request: TerminalWaitRequest,
        context: &TerminalMonitorContext,
        last_output_ms: i64,
        history: TerminalWaitHistory<'_>,
        cancellation: CancellationToken,
    ) -> Result<(TerminalWaitId, TerminalWaitFuture)> {
        self.prune();
        if self.closed {
            return Err(TerminalWaitError::Closed);
        }
        if cancellation.is_cancelled() {
            return Err(TerminalWaitError::Cancelled);
        }
        if self.count.load(Ordering::Acquire) >= MAX_TERMINAL_WAITS {
            return Err(TerminalWaitError::Capacity);
        }
        if history.suffix.len() > MAX_WAIT_HISTORY_SUFFIX {
            return Err(TerminalWaitError::Invalid);
        }
        let id = TerminalWaitId(self.next_id.ok_or(TerminalWaitError::Capacity)?);
        let deadline_ms = i64::try_from(request.safety_ceiling_ms)
            .ok()
            .and_then(|duration| context.now_ms.checked_add(duration))
            .ok_or(TerminalWaitError::Invalid)?;
        let matches_output = matches!(request.condition, TerminalReturnCondition::Match { .. });
        let mut state = TerminalWaitState::new(request, context, last_output_ms, history.matched)
            .map_err(TerminalWaitError::Observation)?;
        if matches_output {
            // Only match waits consume history. Feeding historical bytes must
            // not reset the quiet condition's real last-output timestamp.
            state
                .output(history.suffix, context.now_ms)
                .map_err(TerminalWaitError::Observation)?;
        }
        self.next_id = id.0.get().checked_add(1).and_then(NonZeroU64::new);
        self.count.fetch_add(1, Ordering::AcqRel);
        let reply = Arc::new(SharedReply {
            residency: OnceLock::new(),
            reply: Mutex::new(Reply {
                result: None,
                waker: None,
            }),
            abandoned: AtomicBool::new(false),
            published: AtomicBool::new(false),
            token_outstanding: AtomicBool::new(false),
            _permit: Permit(Arc::clone(&self.count)),
        });
        let future = TerminalWaitFuture {
            reply: Some(Arc::clone(&reply)),
            cancellation_wait: Some(cancellation.cancelled()),
            cancellation: cancellation.clone(),
        };
        self.registrations.push(Registration {
            id,
            identity,
            state,
            cancellation,
            reply,
            ready: None,
            now_ms: context.now_ms,
            last_output_ms,
            deadline_ms,
            cursor: context.cursor.clone(),
        });
        Ok((id, future))
    }

    /// Roll back a reservation whose attention admission never succeeded.
    /// The future must not have escaped the owner request. This deliberately
    /// emits no attention completion and cannot cancel a pre-existing lease.
    pub(crate) fn withdraw_unadmitted(&mut self, id: TerminalWaitId) {
        self.registrations.retain(|entry| entry.id != id);
    }

    /// Feed only durably committed bytes, once, in cursor order. Empty output
    /// advances time/lifecycle without inventing a new output observation.
    /// Validation precedes mutation of *every* matching registration.
    #[cfg(test)]
    pub(crate) fn advance(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        bytes: &[u8],
        context: &TerminalMonitorContext,
        process: Option<TerminalProcessOutcome>,
    ) -> Result<()> {
        if bytes.len() > MAX_MONITOR_FEED_BYTES
            || context.now_ms < 0
            || context.cursor.validate().is_err()
            || !valid_process(process)
        {
            return Err(TerminalWaitError::Invalid);
        }
        let matches = |registration: &&mut Registration| {
            registration.identity.owner == *owner && registration.identity.session == *session
        };
        for registration in &self.registrations {
            if registration.identity.owner == *owner
                && registration.identity.session == *session
                && registration.ready.is_none()
                && (context.now_ms < registration.now_ms
                    || context.cursor < registration.cursor
                    || (!bytes.is_empty() && context.cursor == registration.cursor))
            {
                return Err(TerminalWaitError::Observation(TerminalMonitorError::Clock));
            }
        }
        for registration in self.registrations.iter_mut().filter(matches) {
            if registration.ready.is_some() {
                continue;
            }
            registration
                .state
                .output(bytes, context.now_ms)
                .map_err(TerminalWaitError::Observation)?;
            registration.ready = registration
                .state
                .poll(
                    context,
                    process,
                    registration.cancellation.is_cancelled()
                        || registration.reply.abandoned.load(Ordering::Acquire),
                )
                .map_err(TerminalWaitError::Observation)?;
            registration.now_ms = context.now_ms;
            if !bytes.is_empty() {
                registration.last_output_ms = context.now_ms;
            }
            registration.cursor = context.cursor.clone();
        }
        self.prune();
        Ok(())
    }

    /// Explicit host cancellation requires the exact wait, incarnation,
    /// terminal and actor. No monitor, process or input receipt is modified.
    #[cfg(test)]
    pub(crate) fn cancel(
        &mut self,
        id: TerminalWaitId,
        identity: &TerminalWaitIdentity,
    ) -> Result<()> {
        self.prune();
        let registration = self
            .registrations
            .iter_mut()
            .find(|registration| registration.id == id && registration.identity == *identity)
            .ok_or(TerminalWaitError::NotFound)?;
        if registration.ready.is_none() {
            registration.ready = Some(TerminalWaitOutcome::Cancelled);
        }
        Ok(())
    }

    /// A gap cannot be used as evidence of a literal match. The owner calls
    /// this when its committed observation stream loses continuity, rather
    /// than feeding unrelated retained chunks into the same matcher.
    pub(crate) fn lose(&mut self, owner: &BackgroundOutputOwner, session: &TerminalSessionId) {
        for registration in &mut self.registrations {
            if registration.identity.owner == *owner && registration.identity.session == *session {
                registration.ready.get_or_insert(TerminalWaitOutcome::Lost);
            }
        }
    }

    /// Revoke only pending waits for this exact terminal; completed conditions
    /// and accepted input receipts remain truthful and independently owned.
    pub(crate) fn cancel_session(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
    ) {
        for registration in &mut self.registrations {
            if registration.identity.owner == *owner && registration.identity.session == *session {
                registration
                    .ready
                    .get_or_insert(TerminalWaitOutcome::Cancelled);
            }
        }
    }

    /// Finishing one wait must not clear a newer pending wait's attention.
    /// The owner checks this immediately before its attention transaction.
    pub(crate) fn has_pending_attention(
        &self,
        identity: &TerminalWaitIdentity,
        excluding: TerminalWaitId,
    ) -> bool {
        self.registrations.iter().any(|registration| {
            registration.id != excluding
                && registration.identity == *identity
                && registration.ready.is_none()
                && !registration.cancellation.is_cancelled()
                && !registration.reply.abandoned.load(Ordering::Acquire)
        })
    }

    /// Returns each frozen outcome at most once until its token is published
    /// or dropped. The latter is the retry path after attention persistence
    /// failure: the observed condition stays frozen, but the future stays
    /// pending and must not claim that cleanup committed.
    pub(crate) fn take_ready(&mut self) -> Vec<TerminalWaitCompletion> {
        self.prune();
        for registration in &mut self.registrations {
            if registration.ready.is_none()
                && (registration.cancellation.is_cancelled()
                    || registration.reply.abandoned.load(Ordering::Acquire))
            {
                registration.ready = Some(TerminalWaitOutcome::Cancelled);
            }
        }
        self.registrations
            .iter()
            .filter_map(|registration| {
                let outcome = registration.ready?;
                if registration.reply.published.load(Ordering::Acquire)
                    || registration
                        .reply
                        .token_outstanding
                        .swap(true, Ordering::AcqRel)
                {
                    return None;
                }
                Some(TerminalWaitCompletion {
                    id: registration.id,
                    identity: registration.identity.clone(),
                    outcome,
                    reply: Arc::clone(&registration.reply),
                })
            })
            .collect()
    }

    pub(crate) fn next_deadline(&self) -> Option<i64> {
        self.registrations
            .iter()
            .filter(|registration| registration.ready.is_none())
            .filter_map(|registration| registration.state.next_deadline())
            .min()
    }

    /// One bounded durable page per pending registration lets the owner make
    /// progress without cloning history, skipping drain tails or replaying a
    /// pump step twice. Each returned cursor is updated only after its feed.
    pub(crate) fn observations(&self) -> Vec<TerminalWaitObservation> {
        self.registrations
            .iter()
            .filter(|registration| registration.ready.is_none())
            .map(|registration| TerminalWaitObservation {
                id: registration.id,
                identity: registration.identity.clone(),
                cursor: registration.cursor.clone(),
            })
            .collect()
    }

    /// Feed a validated durable page. While catching up, defer condition
    /// polling until the committed observation cursor is reached or the
    /// absolute safety deadline expires. A moving live tail cannot extend the
    /// ceiling indefinitely; only matches actually observed can win that poll.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit committed observation facts"
    )]
    pub(crate) fn advance_one(
        &mut self,
        observation: &TerminalWaitObservation,
        bytes: &[u8],
        context: &TerminalMonitorContext,
        last_output_ms: i64,
        process: Option<TerminalProcessOutcome>,
        caught_up: bool,
    ) -> Result<()> {
        let registration = self
            .registrations
            .iter_mut()
            .find(|registration| {
                registration.id == observation.id && registration.identity == observation.identity
            })
            .ok_or(TerminalWaitError::NotFound)?;
        if registration.ready.is_some() {
            return Ok(());
        }
        if observation.cursor != registration.cursor
            || context.cursor < registration.cursor
            || context.cursor.validate().is_err()
            || context.now_ms < registration.now_ms
            || last_output_ms < registration.last_output_ms
            || !valid_process(process)
        {
            return Err(TerminalWaitError::Invalid);
        }
        registration
            .state
            .committed_output(bytes, context.now_ms, last_output_ms)
            .map_err(TerminalWaitError::Observation)?;
        registration.cursor = context.cursor.clone();
        registration.now_ms = context.now_ms;
        registration.last_output_ms = last_output_ms;
        if caught_up || context.now_ms >= registration.deadline_ms {
            registration.ready = registration
                .state
                .poll(
                    context,
                    process,
                    registration.cancellation.is_cancelled()
                        || registration.reply.abandoned.load(Ordering::Acquire),
                )
                .map_err(TerminalWaitError::Observation)?;
        }
        Ok(())
    }

    /// Shutdown is explicit so callers can persist attention transitions and
    /// publish outside profile guards. Already frozen outcomes always win.
    pub(crate) fn close(&mut self) {
        self.closed = true;
        for registration in &mut self.registrations {
            registration.ready.get_or_insert(TerminalWaitOutcome::Lost);
        }
    }

    fn prune(&mut self) {
        self.registrations
            .retain(|registration| !registration.reply.published.load(Ordering::Acquire));
    }
}

fn valid_process(process: Option<TerminalProcessOutcome>) -> bool {
    match process {
        None => true,
        Some(TerminalProcessOutcome::Exited(code)) => (0..=255).contains(&code),
        Some(TerminalProcessOutcome::Signaled(signal)) => (1..=255).contains(&signal),
    }
}

/// A prepared receipt, not evidence of durable attention cleanup. `publish`
/// must be called only after that cleanup commits and its guard is dropped.
/// Drop does not invoke user callbacks and does not discard the receipt.
pub(crate) struct TerminalWaitCompletion {
    pub(crate) id: TerminalWaitId,
    pub(crate) identity: TerminalWaitIdentity,
    pub(crate) outcome: TerminalWaitOutcome,
    reply: Arc<SharedReply>,
}
impl TerminalWaitCompletion {
    /// Returns false if a caller waker panicked. The reply remains committed
    /// and readable; other completions can still be published independently.
    pub(crate) fn publish(self) -> bool {
        self.publish_with(None)
    }

    /// Shutdown's final bounded attempt failed to persist attention. Publish
    /// an explicit failure alongside the already frozen condition, not an
    /// invented successful cleanup and not a permanently pending future.
    pub(crate) fn publish_failed(self) -> bool {
        self.publish_with(Some(TerminalWaitAttentionError::Unavailable))
    }

    fn publish_with(self, attention_error: Option<TerminalWaitAttentionError>) -> bool {
        let waker = {
            let mut reply = self
                .reply
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            reply.result = Some(TerminalWaitReceipt {
                outcome: self.outcome,
                attention_error,
            });
            self.reply.published.store(true, Ordering::Release);
            reply.waker.take()
        };
        waker.is_none_or(|waker| catch_callback(|| waker.wake()))
    }
}
impl Drop for TerminalWaitCompletion {
    fn drop(&mut self) {
        self.reply.token_outstanding.store(false, Ordering::Release);
    }
}

pub(crate) struct TerminalWaitFuture {
    reply: Option<Arc<SharedReply>>,
    cancellation: CancellationToken,
    cancellation_wait: Option<Cancelled>,
}
impl TerminalWaitFuture {
    /// Both an abandoned registration awaiting durable attention cleanup and an
    /// unconsumed reply keep residency, without keeping the host alive.
    pub(crate) fn retain_residency(&self, lease: crate::terminal_registry::TerminalResidentLease) {
        if let Some(reply) = &self.reply {
            let _ = reply.residency.set(lease);
        }
    }
}
impl Future for TerminalWaitFuture {
    type Output = TerminalWaitReceipt;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let shared = Arc::clone(
            self.reply
                .as_ref()
                .expect("terminal wait polled after completion"),
        );
        // Clone and destroy arbitrary caller wakers outside the reply lock.
        let mut incoming = Some(context.waker().clone());
        let (result, previous) = {
            let mut reply = shared
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = reply.result;
            let previous = if result.is_some() {
                reply.waker.take()
            } else {
                std::mem::replace(&mut reply.waker, incoming.take())
            };
            (result, previous)
        };
        drop(previous);
        drop(incoming);
        if let Some(result) = result {
            self.cancellation_wait = None;
            self.reply = None;
            return Poll::Ready(result);
        }
        if self.cancellation.is_cancelled() {
            self.cancellation_wait = None;
        } else if let Some(cancellation) = &mut self.cancellation_wait {
            let _ = Pin::new(cancellation).poll(context);
        }
        Poll::Pending
    }
}
impl Drop for TerminalWaitFuture {
    fn drop(&mut self) {
        let cancellation_wait = self.cancellation_wait.take();
        catch_callback(|| drop(cancellation_wait));
        if let Some(shared) = self.reply.take() {
            shared.abandoned.store(true, Ordering::Release);
            let waker = {
                shared
                    .reply
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .waker
                    .take()
            };
            // A destructor must not unwind a second time during owner failure.
            // More importantly, no callback runs under the reply mutex.
            catch_callback(|| drop(waker));
        }
    }
}

fn catch_callback(callback: impl FnOnce()) -> bool {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(()) => true,
        Err(payload) => {
            // An opaque panic payload may itself have a panicking destructor.
            std::mem::forget(payload);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId, TerminalLifecycle};
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    fn identity(session: &str, incarnation: &str, writer: u64) -> TerminalWaitIdentity {
        TerminalWaitIdentity {
            owner: BackgroundOutputOwner::new(
                SessionId::new("owner").unwrap(),
                SessionIncarnationId::new(incarnation).unwrap(),
            ),
            session: TerminalSessionId::new(session).unwrap(),
            actor: TerminalActorRole::Agent,
            writer: TerminalWriterId::new(NonZeroU64::new(writer).unwrap()),
        }
    }
    fn context(now_ms: i64, offset: u64) -> TerminalMonitorContext {
        TerminalMonitorContext {
            now_ms,
            cursor: TerminalCursor::new(1, offset).unwrap(),
            lifecycle: TerminalLifecycle::Running,
        }
    }
    fn register(
        coordinator: &mut TerminalWaitCoordinator,
        identity: TerminalWaitIdentity,
        condition: TerminalReturnCondition,
    ) -> (TerminalWaitId, TerminalWaitFuture) {
        coordinator
            .register(
                identity,
                TerminalWaitRequest {
                    condition,
                    safety_ceiling_ms: 100,
                },
                &context(0, 0),
                0,
                TerminalWaitHistory::default(),
                CancellationToken::new(),
            )
            .unwrap()
    }
    fn poll(future: &mut TerminalWaitFuture) -> Poll<TerminalWaitOutcome> {
        Pin::new(future)
            .poll(&mut Context::from_waker(Waker::noop()))
            .map(|receipt| {
                assert!(receipt.attention_error.is_none());
                receipt.outcome
            })
    }
    #[test]
    fn withdrawn_reservation_emits_no_attention_completion() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let (id, future) = register(
            &mut coordinator,
            identity("terminal", "incarnation", 1),
            TerminalReturnCondition::Exit {},
        );
        coordinator.withdraw_unadmitted(id);
        drop(future);
        assert!(coordinator.observations().is_empty());
        assert!(coordinator.take_ready().is_empty());
        assert_eq!(coordinator.count.load(Ordering::Acquire), 0);
    }
    fn advance(
        coordinator: &mut TerminalWaitCoordinator,
        identity: &TerminalWaitIdentity,
        bytes: &[u8],
        now_ms: i64,
        offset: u64,
        process: Option<TerminalProcessOutcome>,
    ) {
        coordinator
            .advance(
                &identity.owner,
                &identity.session,
                bytes,
                &context(now_ms, offset),
                process,
            )
            .unwrap();
    }
    fn publish(coordinator: &mut TerminalWaitCoordinator) {
        for completion in coordinator.take_ready() {
            assert!(completion.publish());
        }
    }

    #[test]
    fn four_conditions_are_nonblocking_and_independent() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut started) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        let (_, mut exit) = register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        let (_, mut quiet) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Quiet { duration_ms: 10 },
        );
        let (_, mut matching) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Match {
                pattern: "hello".into(),
            },
        );
        assert_eq!(poll(&mut started), Poll::Pending);
        advance(&mut coordinator, &who, b"he", 1, 2, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut started),
            Poll::Ready(TerminalWaitOutcome::Started)
        );
        assert_eq!(poll(&mut exit), Poll::Pending);
        assert_eq!(poll(&mut matching), Poll::Pending);
        assert_eq!(coordinator.next_deadline(), Some(11));
        advance(&mut coordinator, &who, b"llo", 2, 5, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut matching),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
        advance(&mut coordinator, &who, b"", 12, 5, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut quiet),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
        advance(
            &mut coordinator,
            &who,
            b"",
            13,
            5,
            Some(TerminalProcessOutcome::Exited(7)),
        );
        publish(&mut coordinator);
        assert_eq!(poll(&mut exit), Poll::Ready(TerminalWaitOutcome::Exited(7)));
        assert_eq!(coordinator.next_deadline(), None);
    }

    #[test]
    fn history_seed_preserves_boundary_matches_and_quiet_timestamp() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let request = TerminalWaitRequest {
            condition: TerminalReturnCondition::Match {
                pattern: "hello".into(),
            },
            safety_ceiling_ms: 100,
        };
        let (_, mut matching) = coordinator
            .register(
                who.clone(),
                request,
                &context(10, 20),
                2,
                TerminalWaitHistory {
                    matched: false,
                    suffix: b"he",
                },
                CancellationToken::new(),
            )
            .unwrap();
        let (_, mut quiet) = coordinator
            .register(
                who.clone(),
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Quiet { duration_ms: 10 },
                    safety_ceiling_ms: 100,
                },
                &context(10, 20),
                2,
                TerminalWaitHistory {
                    matched: false,
                    suffix: b"he",
                },
                CancellationToken::new(),
            )
            .unwrap();
        advance(&mut coordinator, &who, b"", 12, 20, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut quiet),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
        advance(&mut coordinator, &who, b"llo", 13, 23, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut matching),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
    }

    #[test]
    fn full_pattern_suffix_and_seeded_prior_match_work() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let request = TerminalWaitRequest {
            condition: TerminalReturnCondition::Match {
                pattern: "x".repeat(4096),
            },
            safety_ceiling_ms: 100,
        };
        let (_, mut future) = coordinator
            .register(
                who.clone(),
                request.clone(),
                &context(0, 4095),
                0,
                TerminalWaitHistory {
                    matched: false,
                    suffix: &vec![b'x'; 4095],
                },
                CancellationToken::new(),
            )
            .unwrap();
        advance(&mut coordinator, &who, b"x", 1, 4096, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut future),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
        let (_, mut future) = coordinator
            .register(
                who.clone(),
                request.clone(),
                &context(1, 4096),
                0,
                TerminalWaitHistory {
                    matched: true,
                    suffix: b"",
                },
                CancellationToken::new(),
            )
            .unwrap();
        advance(&mut coordinator, &who, b"", 1, 4096, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut future),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
        assert!(matches!(
            coordinator.register(
                who,
                request,
                &context(1, 4096),
                0,
                TerminalWaitHistory {
                    matched: false,
                    suffix: &vec![b'x'; 4096]
                },
                CancellationToken::new()
            ),
            Err(TerminalWaitError::Invalid)
        ));
    }

    #[test]
    fn completion_requires_explicit_publication_and_retries_frozen_receipt() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (id, mut future) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        advance(&mut coordinator, &who, b"", 0, 0, None);
        let completion = coordinator.take_ready().pop().unwrap();
        assert_eq!(completion.id, id);
        assert_eq!(completion.identity, who);
        assert_eq!(completion.outcome, TerminalWaitOutcome::Started);
        assert!(coordinator.take_ready().is_empty());
        assert_eq!(poll(&mut future), Poll::Pending);
        drop(completion); // attention publication failed: retry, don't wake.
        coordinator.cancel(id, &who).unwrap();
        coordinator.close();
        let retry = coordinator.take_ready().pop().unwrap();
        assert_eq!(retry.outcome, TerminalWaitOutcome::Started);
        assert!(retry.publish());
        assert_eq!(poll(&mut future), Poll::Ready(TerminalWaitOutcome::Started));
    }

    #[test]
    fn capacity_includes_completed_unconsumed_and_abandoned_unpublished() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let mut futures = Vec::new();
        for _ in 0..MAX_TERMINAL_WAITS {
            futures.push(
                register(
                    &mut coordinator,
                    who.clone(),
                    TerminalReturnCondition::Started,
                )
                .1,
            );
        }
        let attempt = |coordinator: &mut TerminalWaitCoordinator| {
            coordinator.register(
                who.clone(),
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit,
                    safety_ceiling_ms: 100,
                },
                &context(0, 0),
                0,
                TerminalWaitHistory::default(),
                CancellationToken::new(),
            )
        };
        assert!(matches!(
            attempt(&mut coordinator),
            Err(TerminalWaitError::Capacity)
        ));
        advance(&mut coordinator, &who, b"", 0, 0, None);
        publish(&mut coordinator);
        assert!(matches!(
            attempt(&mut coordinator),
            Err(TerminalWaitError::Capacity)
        ));
        assert_eq!(
            poll(&mut futures[0]),
            Poll::Ready(TerminalWaitOutcome::Started)
        );
        let (_, dropped) = attempt(&mut coordinator).unwrap();
        drop(dropped);
        assert!(matches!(
            attempt(&mut coordinator),
            Err(TerminalWaitError::Capacity)
        ));
        let completion = coordinator.take_ready().pop().unwrap();
        assert_eq!(completion.outcome, TerminalWaitOutcome::Cancelled);
        assert!(completion.publish());
        assert!(attempt(&mut coordinator).is_ok());
    }

    #[test]
    fn stale_identity_never_cancels_or_observes_another_wait() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (id, mut first) =
            register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        for wrong in [
            identity("two", "inc", 1),
            identity("one", "other", 1),
            identity("one", "inc", 2),
        ] {
            assert_eq!(
                coordinator.cancel(id, &wrong),
                Err(TerminalWaitError::NotFound)
            );
        }
        let mut wrong = who.clone();
        wrong.actor = TerminalActorRole::Human;
        assert_eq!(
            coordinator.cancel(id, &wrong),
            Err(TerminalWaitError::NotFound)
        );
        let other = identity("two", "inc", 1);
        let (_, mut second) = register(
            &mut coordinator,
            other.clone(),
            TerminalReturnCondition::Exit,
        );
        advance(
            &mut coordinator,
            &identity("one", "stale", 1),
            b"",
            1,
            0,
            Some(TerminalProcessOutcome::Exited(4)),
        );
        assert!(coordinator.take_ready().is_empty());
        advance(
            &mut coordinator,
            &other,
            b"",
            1,
            0,
            Some(TerminalProcessOutcome::Signaled(9)),
        );
        publish(&mut coordinator);
        assert_eq!(poll(&mut first), Poll::Pending);
        assert_eq!(
            poll(&mut second),
            Poll::Ready(TerminalWaitOutcome::Signaled(9))
        );
        coordinator.cancel(id, &who).unwrap();
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut first),
            Poll::Ready(TerminalWaitOutcome::Cancelled)
        );
        assert_eq!(
            coordinator.cancel(id, &who),
            Err(TerminalWaitError::NotFound)
        );
    }

    #[test]
    fn cancellation_and_drop_leave_independent_receipts_untouched() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let token = CancellationToken::new();
        let (_, mut cancelled) = coordinator
            .register(
                who.clone(),
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit,
                    safety_ceiling_ms: 100,
                },
                &context(0, 0),
                0,
                TerminalWaitHistory::default(),
                token.clone(),
            )
            .unwrap();
        let (_, dropped) = register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        let (_, mut independent) =
            register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        assert_eq!(poll(&mut cancelled), Poll::Pending);
        token.cancel();
        drop(dropped);
        assert_eq!(poll(&mut cancelled), Poll::Pending);
        let completions = coordinator.take_ready();
        assert_eq!(completions.len(), 2);
        for completion in completions {
            assert_eq!(completion.outcome, TerminalWaitOutcome::Cancelled);
            assert!(completion.publish());
        }
        assert_eq!(
            poll(&mut cancelled),
            Poll::Ready(TerminalWaitOutcome::Cancelled)
        );
        assert_eq!(poll(&mut independent), Poll::Pending);
        advance(
            &mut coordinator,
            &who,
            b"",
            2,
            0,
            Some(TerminalProcessOutcome::Exited(0)),
        );
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut independent),
            Poll::Ready(TerminalWaitOutcome::Exited(0))
        );
    }

    #[test]
    fn safety_ceiling_lost_shutdown_and_cancelled_admission_are_bounded() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut ceiling) =
            register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        advance(&mut coordinator, &who, b"", 100, 0, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut ceiling),
            Poll::Ready(TerminalWaitOutcome::SafetyCeiling)
        );
        let (_, mut lost) = register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        let mut lost_context = context(0, 0);
        lost_context.lifecycle = TerminalLifecycle::Lost;
        coordinator
            .advance(&who.owner, &who.session, b"", &lost_context, None)
            .unwrap();
        publish(&mut coordinator);
        assert_eq!(poll(&mut lost), Poll::Ready(TerminalWaitOutcome::Lost));
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            coordinator.register(
                who.clone(),
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit,
                    safety_ceiling_ms: 100
                },
                &context(0, 0),
                0,
                TerminalWaitHistory::default(),
                token
            ),
            Err(TerminalWaitError::Cancelled)
        ));
        let (_, mut shutdown) =
            register(&mut coordinator, who.clone(), TerminalReturnCondition::Exit);
        coordinator.close();
        publish(&mut coordinator);
        assert_eq!(poll(&mut shutdown), Poll::Ready(TerminalWaitOutcome::Lost));
        assert!(matches!(
            coordinator.register(
                who,
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit,
                    safety_ceiling_ms: 100
                },
                &context(0, 0),
                0,
                TerminalWaitHistory::default(),
                CancellationToken::new()
            ),
            Err(TerminalWaitError::Closed)
        ));
    }

    #[test]
    fn invalid_observation_does_not_partially_feed_matches() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut first) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Match {
                pattern: "xy".into(),
            },
        );
        let (_, _newer) = coordinator
            .register(
                who.clone(),
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit,
                    safety_ceiling_ms: 100,
                },
                &context(10, 10),
                0,
                TerminalWaitHistory::default(),
                CancellationToken::new(),
            )
            .unwrap();
        assert!(
            coordinator
                .advance(&who.owner, &who.session, b"x", &context(5, 5), None)
                .is_err()
        );
        advance(&mut coordinator, &who, b"y", 11, 11, None);
        assert_eq!(poll(&mut first), Poll::Pending);
        assert!(coordinator.take_ready().is_empty());
        assert!(
            coordinator
                .advance(
                    &who.owner,
                    &who.session,
                    b"x",
                    &context(12, 12),
                    Some(TerminalProcessOutcome::Exited(-1))
                )
                .is_err()
        );
        assert!(
            coordinator
                .advance(
                    &who.owner,
                    &who.session,
                    &vec![0; MAX_MONITOR_FEED_BYTES + 1],
                    &context(12, 12),
                    None
                )
                .is_err()
        );
        advance(&mut coordinator, &who, b"xy", 12, 13, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut first),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
    }

    #[derive(Default)]
    struct Counter(AtomicUsize);
    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn preparation_and_token_drop_never_wake_the_caller() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut future) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        let counter = Arc::new(Counter::default());
        let waker = Waker::from(Arc::clone(&counter));
        assert_eq!(
            Pin::new(&mut future).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        advance(&mut coordinator, &who, b"", 0, 0, None);
        drop(coordinator.take_ready());
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        publish(&mut coordinator);
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert_eq!(poll(&mut future), Poll::Ready(TerminalWaitOutcome::Started));
    }

    #[test]
    fn clone_drop_and_wake_reentrancy_never_holds_reply_mutex() {
        for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
            let mut coordinator = TerminalWaitCoordinator::new();
            let who = identity("one", "inc", 1);
            let (_, mut future) = register(
                &mut coordinator,
                who.clone(),
                TerminalReturnCondition::Started,
            );
            let shared = Arc::clone(future.reply.as_ref().unwrap());
            let (waker, state) = reentrant_waker(callback, move || {
                assert!(
                    shared.reply.try_lock().is_ok(),
                    "callback ran under reply mutex"
                );
            });
            assert_eq!(
                Pin::new(&mut future).poll(&mut Context::from_waker(&waker)),
                Poll::Pending
            );
            drop(waker);
            advance(&mut coordinator, &who, b"", 0, 0, None);
            publish(&mut coordinator);
            assert_eq!(poll(&mut future), Poll::Ready(TerminalWaitOutcome::Started));
            assert!(state.calls() > 0);
        }
    }

    #[test]
    fn panicking_wake_cannot_erase_completion_or_block_other_waits() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut future) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        let (_, mut other) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        let (waker, _) = reentrant_waker(Callback::Wake, || panic!("injected wake panic"));
        assert_eq!(
            Pin::new(&mut future).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        advance(&mut coordinator, &who, b"", 0, 0, None);
        let mut completions = coordinator.take_ready().into_iter();
        assert!(!completions.next().unwrap().publish());
        assert!(completions.next().unwrap().publish());
        assert_eq!(poll(&mut future), Poll::Ready(TerminalWaitOutcome::Started));
        assert_eq!(poll(&mut other), Poll::Ready(TerminalWaitOutcome::Started));
    }

    #[test]
    fn completion_does_not_clear_newer_attention_and_gaps_lose_pending_waits() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (old, mut completed) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        advance(&mut coordinator, &who, b"", 0, 0, None);
        let (_, mut pending) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Match {
                pattern: "xy".into(),
            },
        );
        assert!(coordinator.has_pending_attention(&who, old));
        advance(&mut coordinator, &who, b"x", 1, 1, None);
        coordinator.lose(&who.owner, &who.session);
        assert!(!coordinator.has_pending_attention(&who, old));
        advance(&mut coordinator, &who, b"y", 2, 20, None);
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut completed),
            Poll::Ready(TerminalWaitOutcome::Started)
        );
        assert_eq!(poll(&mut pending), Poll::Ready(TerminalWaitOutcome::Lost));
    }

    #[test]
    fn replaced_waker_drop_reenters_only_outside_reply_mutex() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut future) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        let shared = Arc::clone(future.reply.as_ref().unwrap());
        let (waker, state) = reentrant_waker(Callback::Drop, move || {
            assert!(shared.reply.try_lock().is_ok());
        });
        assert_eq!(
            Pin::new(&mut future).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        drop(waker);
        let before = state.calls();
        assert_eq!(poll(&mut future), Poll::Pending);
        assert!(state.calls() > before);
        advance(&mut coordinator, &who, b"", 0, 0, None);
        publish(&mut coordinator);
        assert_eq!(poll(&mut future), Poll::Ready(TerminalWaitOutcome::Started));
    }

    #[test]
    fn dropping_future_contains_panicking_waker_destructors() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut future) = register(&mut coordinator, who, TerminalReturnCondition::Exit);
        let (waker, _) = reentrant_waker(Callback::Drop, || panic!("injected drop panic"));
        // Poll can drop its spare clone through the cancellation registration;
        // contain any callback panic outside all coordinator/reply locks.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            Pin::new(&mut future).poll(&mut Context::from_waker(&waker))
        }));
        assert!(catch_unwind(AssertUnwindSafe(|| drop(future))).is_ok());
        catch_callback(|| drop(waker));
        let completion = coordinator.take_ready().pop().unwrap();
        assert_eq!(completion.outcome, TerminalWaitOutcome::Cancelled);
        assert!(completion.publish());
    }

    #[test]
    fn committed_catchup_defers_conditions_and_preserves_real_quiet_time() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut matching) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Match {
                pattern: "hello".into(),
            },
        );
        let (_, mut quiet) = register(
            &mut coordinator,
            who,
            TerminalReturnCondition::Quiet { duration_ms: 10 },
        );
        for observation in coordinator.observations() {
            coordinator
                .advance_one(&observation, b"he", &context(50, 2), 2, None, false)
                .unwrap();
        }
        assert!(coordinator.take_ready().is_empty());
        for observation in coordinator.observations() {
            coordinator
                .advance_one(&observation, b"llo", &context(101, 5), 2, None, true)
                .unwrap();
        }
        publish(&mut coordinator);
        // The retained match outranks the elapsed ceiling; replaying at 101
        // does not falsely reset the last real output time (2) for quiet.
        assert_eq!(
            poll(&mut matching),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
        assert_eq!(
            poll(&mut quiet),
            Poll::Ready(TerminalWaitOutcome::ConditionMet)
        );
    }

    #[test]
    fn final_publication_failure_preserves_condition_with_explicit_failure() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut future) = register(
            &mut coordinator,
            who.clone(),
            TerminalReturnCondition::Started,
        );
        advance(&mut coordinator, &who, b"", 0, 0, None);
        assert!(coordinator.take_ready().pop().unwrap().publish_failed());
        assert_eq!(
            Pin::new(&mut future).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(TerminalWaitReceipt {
                outcome: TerminalWaitOutcome::Started,
                attention_error: Some(TerminalWaitAttentionError::Unavailable),
            })
        );
    }

    #[test]
    fn an_unfinished_catchup_cannot_extend_the_absolute_safety_ceiling() {
        let mut coordinator = TerminalWaitCoordinator::new();
        let who = identity("one", "inc", 1);
        let (_, mut future) = register(
            &mut coordinator,
            who,
            TerminalReturnCondition::Match {
                pattern: "never".into(),
            },
        );
        let observation = coordinator.observations().pop().unwrap();
        coordinator
            .advance_one(&observation, b"x", &context(100, 1), 100, None, false)
            .unwrap();
        publish(&mut coordinator);
        assert_eq!(
            poll(&mut future),
            Poll::Ready(TerminalWaitOutcome::SafetyCeiling)
        );
    }
}
