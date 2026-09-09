//! Exact-turn, single-entry edits through the core's canonical CAS boundary.

use std::collections::BTreeMap;
use std::fmt;
use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::task::Poll;

use serde::Serialize;
use serde_json::Value;

use super::{
    JsonOwnerGuard, SessionRecord, SessionRevision, SessionState, Turn, json_limit_failure,
    redact_store_error, validate_record_limits,
};
use crate::engine::{EngineInner, HostLease};
use crate::json_bounds::{JsonLimitViolation, serialized_json_size_bounded, validate_json_roots};
use crate::{
    BoxFuture, CancellationToken, ContentBlock, EngineError, EngineLimits, SessionId,
    SessionIncarnationId, SessionStoreError, SessionStoreErrorKind, TurnId,
};

const MAX_CONFLICT_RETRIES: usize = 32;

/// Owned access to one metadata entry during one exact live turn.
/// Clones share editor identity; independently minted editors do not. No editor
/// retains the turn's exclusive lease or the host resource's lifetime.
#[derive(Clone)]
pub struct TurnMetadataEditor {
    identity: Arc<EditorIdentity>,
}

struct EditorIdentity {
    scope: Weak<TurnMetadataScope>,
    key: String,
}

/// Opaque editor-bound expected entry. It retains a validated immutable record
/// snapshot without cloning its transcript or unrelated metadata.
pub struct TurnMetadataSnapshot {
    editor: Arc<EditorIdentity>,
    record: Arc<SessionRecord>,
}

impl TurnMetadataSnapshot {
    #[must_use]
    pub fn entry(&self) -> Option<&Value> {
        self.record.metadata.get(&self.editor.key)
    }
}

impl fmt::Debug for TurnMetadataEditor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnMetadataEditor").finish_non_exhaustive()
    }
}
impl fmt::Debug for TurnMetadataSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnMetadataSnapshot")
            .finish_non_exhaustive()
    }
}

pub(super) struct TurnMetadataScope {
    engine: Arc<EngineInner>,
    state: Arc<SessionState>,
    host: HostLease,
    session: SessionId,
    incarnation: SessionIncarnationId,
    _turn: TurnId,
    next_sequence: u64,
    cancellation: CancellationToken,
    closed: CancellationToken,
    admission_closed: AtomicBool,
    operation_active: AtomicBool,
}

impl TurnMetadataScope {
    pub(super) fn new(
        engine: Arc<EngineInner>,
        state: Arc<SessionState>,
        host: HostLease,
        record: &SessionRecord,
        turn: TurnId,
        cancellation: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            engine,
            state,
            host,
            session: record.id.clone(),
            incarnation: record.incarnation_id.clone(),
            _turn: turn,
            next_sequence: record.next_turn_sequence,
            cancellation,
            closed: CancellationToken::new(),
            admission_closed: AtomicBool::new(false),
            operation_active: AtomicBool::new(false),
        })
    }

    pub(super) fn close_admission(&self) {
        self.admission_closed.store(true, Ordering::Release);
    }

    pub(super) fn wake_closed(&self) {
        self.closed.cancel();
    }

    fn ensure_open(&self) -> Result<(), EngineError> {
        self.host.ensure_open()?;
        if self.admission_closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() {
            return Err(closed());
        }
        Ok(())
    }

    fn validate_identity(&self, record: &SessionRecord) -> Result<(), EngineError> {
        if record.id != self.session || record.incarnation_id != self.incarnation {
            return Err(EngineError::SessionIncarnationConflict);
        }
        if record.next_turn_sequence != self.next_sequence {
            return Err(closed());
        }
        Ok(())
    }

    fn acquire(self: &Arc<Self>) -> Result<OperationLease, EngineError> {
        self.ensure_open()?;
        self.operation_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EngineError::SessionBusy)?;
        let lease = OperationLease(Arc::clone(self));
        self.ensure_open()?;
        Ok(lease)
    }

    fn snapshot(&self) -> Result<Arc<SessionRecord>, EngineError> {
        self.ensure_open()?;
        let (record, persisted) = self.state.snapshot();
        self.validate_identity(&record)?;
        if !persisted || record.revision == SessionRevision(0) {
            return Err(EngineError::Protocol(
                "turn metadata requires a persisted turn".to_owned(),
            ));
        }
        validate_record_limits(&record, self.engine.limits)?;
        Ok(record)
    }

    /// Polls the operation itself, not an outer-turn mailbox. Separate cancelled
    /// futures register this caller's waker even when the turn is not polled.
    async fn wait<T>(
        &self,
        mut operation: BoxFuture<'_, Result<T, SessionStoreError>>,
        save_success_wins: bool,
    ) -> Result<T, EngineError> {
        let mut cancelled = std::pin::pin!(self.cancellation.cancelled());
        let mut closed_wait = std::pin::pin!(self.closed.cancelled());
        poll_fn(|cx| {
            self.ensure_open()?;
            if cancelled.as_mut().poll(cx).is_ready() || closed_wait.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(closed()));
            }
            self.ensure_open()?;
            let result = operation.as_mut().poll(cx);
            if save_success_wins && matches!(&result, Poll::Ready(Ok(_))) {
                return result
                    .map(|result| result.map_err(|error| redact_store_error(error).into()));
            }
            self.ensure_open()?;
            result.map(|result| result.map_err(|error| redact_store_error(error).into()))
        })
        .await
    }

    async fn reload(&self) -> Result<Arc<SessionRecord>, EngineError> {
        self.ensure_open()?;
        let load = self.engine.session_store.load(self.session.clone());
        let guarded_load =
            Box::pin(async move { load.await.map(|record| record.map(JsonOwnerGuard::new)) });
        let loaded = self.wait(guarded_load, false).await?.ok_or_else(|| {
            EngineError::Protocol("session disappeared during turn metadata editing".to_owned())
        })?;
        self.validate_identity(loaded.get())?;
        SessionState::validate_loaded(loaded.get())?;
        validate_record_limits(loaded.get(), self.engine.limits)?;
        let revision = loaded.get().revision;
        if let Err(error) = self.state.reconcile_loaded(loaded.into_inner()) {
            // A concurrently confirmed core transcript publication can overtake
            // this load. Its newer canonical snapshot is already authoritative.
            let current = self.snapshot()?;
            if current.revision <= revision {
                return Err(error);
            }
        }
        // Active editors never clear global reconciliation debt: a different
        // operation may already own a newer uncertain publication obligation.
        self.snapshot()
    }
}

struct OperationLease(Arc<TurnMetadataScope>);
impl Drop for OperationLease {
    fn drop(&mut self) {
        self.0.operation_active.store(false, Ordering::Release);
    }
}

/// Declared before the owned store future, so that future is dropped first.
/// Re-arming after its destruction prevents an overlapping newer load from
/// accidentally clearing this operation's still-uncertain publication debt.
struct PublicationDebt {
    state: Arc<SessionState>,
    confirmed: bool,
}
impl PublicationDebt {
    fn new(state: &Arc<SessionState>) -> Self {
        state
            .metadata_reconciliation_required
            .store(true, Ordering::Release);
        Self {
            state: Arc::clone(state),
            confirmed: false,
        }
    }
}
impl Drop for PublicationDebt {
    fn drop(&mut self) {
        if !self.confirmed {
            self.state
                .metadata_reconciliation_required
                .store(true, Ordering::Release);
        }
    }
}

impl Turn {
    /// Grants an owned editor for one key during this exact live turn. There is
    /// no provider/permission/tool phase restriction, and construction does no I/O.
    ///
    /// # Errors
    /// Rejects a closed/cancelled host or turn and keys exceeding metadata bounds.
    pub fn metadata_editor(&self, key: &str) -> Result<TurnMetadataEditor, EngineError> {
        self.metadata_scope.ensure_open()?;
        // Use the shortest JSON value, so key admission does not reject an
        // otherwise valid entry exactly at the configured byte boundary.
        validate_entry_bytes(key, &Value::from(0), self.metadata_scope.engine.limits)?;
        Ok(TurnMetadataEditor {
            identity: Arc::new(EditorIdentity {
                scope: Arc::downgrade(&self.metadata_scope),
                key: key.to_owned(),
            }),
        })
    }
}

impl TurnMetadataEditor {
    /// Observes whether the exact owning turn still admits metadata operations.
    /// This synchronous observation performs no I/O, keeps no turn lease, and
    /// is not a store receipt or a guarantee against subsequent cancellation.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.identity.scope.upgrade().is_some_and(|scope| {
            scope.ensure_open().is_ok()
                && scope.validate_identity(&scope.state.snapshot().0).is_ok()
        })
    }
    /// Authoritatively loads this entry through the core store and reconciles
    /// the canonical record. The owned future is inert until polled.
    ///
    /// # Errors
    /// Rejects closure, another editor operation, invalid/missing stored records,
    /// identity or turn-allocator changes, and store failures.
    #[must_use]
    pub fn read_entry(&self) -> BoxFuture<'static, Result<TurnMetadataSnapshot, EngineError>> {
        let identity = Arc::clone(&self.identity);
        Box::pin(async move {
            let scope = identity.scope.upgrade().ok_or_else(closed)?;
            let _lease = scope.acquire()?;
            let record = scope.reload().await?;
            Ok(TurnMetadataSnapshot {
                editor: identity,
                record,
            })
        })
    }

    /// Replaces/removes only this entry if it still equals the expected entry.
    /// Transcript progress and unrelated metadata changes are preserved. A save
    /// conflict is retried at most 32 times while the exact turn remains open;
    /// a changed target entry is never silently rebased. No work starts before poll.
    ///
    /// # Errors
    /// Rejects foreign snapshots, closure, concurrent editor work, bound overflow,
    /// entry/turn conflicts and persistence failures. Once save is invoked,
    /// failure or drop may follow publication; neither means rollback. An observed
    /// successful save wins same-poll cancellation and returns its exact revision.
    #[must_use]
    pub fn compare_exchange(
        &self,
        expected: TurnMetadataSnapshot,
        replacement: Option<Value>,
    ) -> BoxFuture<'static, Result<SessionRevision, EngineError>> {
        let identity = Arc::clone(&self.identity);
        let replacement = replacement.map(JsonOwnerGuard::new);
        Box::pin(async move {
            if !Arc::ptr_eq(&expected.editor, &identity) {
                return Err(EngineError::Protocol(
                    "turn metadata snapshot belongs to another editor".to_owned(),
                ));
            }
            let scope = identity.scope.upgrade().ok_or_else(closed)?;
            let _lease = scope.acquire()?;
            let replacement = replacement.as_ref().map(JsonOwnerGuard::get);
            if let Some(value) = replacement {
                validate_json_roots([value], scope.engine.limits).map_err(limit_error)?;
                validate_entry_bytes(&identity.key, value, scope.engine.limits)?;
            }
            if scope
                .state
                .metadata_reconciliation_required
                .load(Ordering::Acquire)
            {
                scope.reload().await?;
            }
            for _ in 0..MAX_CONFLICT_RETRIES {
                let current = scope.snapshot()?;
                if current.metadata.get(&identity.key) != expected.entry() {
                    return Err(entry_conflict());
                }
                validate_candidate(&current, &identity.key, replacement, scope.engine.limits)?;
                let mut candidate = (*current).clone();
                if let Some(value) = replacement {
                    candidate
                        .metadata
                        .insert(identity.key.clone(), value.clone());
                } else {
                    candidate.metadata.remove(&identity.key);
                }
                let candidate = JsonOwnerGuard::new(candidate);
                if !scope.state.snapshot_is_current(&current, true) {
                    continue;
                }
                scope.ensure_open()?;
                let mut debt = PublicationDebt::new(&scope.state);
                let save = scope
                    .engine
                    .session_store
                    .save(candidate.get().clone(), Some(current.revision));
                match scope.wait(save, true).await {
                    Ok(revision) if revision > current.revision => {
                        let mut candidate = candidate.into_inner();
                        candidate.revision = revision;
                        scope.state.reconcile_saved(Arc::new(candidate))?;
                        debt.confirmed = true;
                        return Ok(revision);
                    }
                    Ok(_) => {
                        return Err(EngineError::Protocol(
                            "turn metadata save returned a non-increasing revision".to_owned(),
                        ));
                    }
                    Err(EngineError::Store(error))
                        if error.kind == SessionStoreErrorKind::Conflict =>
                    {
                        scope.reload().await?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(SessionStoreError::new(
                SessionStoreErrorKind::Conflict,
                "turn_metadata_contended",
                "turn metadata edit exceeded its conflict retry bound",
                true,
            )
            .into())
        })
    }
}

fn closed() -> EngineError {
    EngineError::Protocol("turn metadata editor is closed".to_owned())
}
fn entry_conflict() -> EngineError {
    SessionStoreError::new(
        SessionStoreErrorKind::Conflict,
        "turn_metadata_entry_changed",
        "turn metadata entry changed",
        true,
    )
    .into()
}
fn limit_error(error: JsonLimitViolation) -> EngineError {
    EngineError::Protocol(json_limit_failure(error).message)
}
fn validate_bytes(value: &impl Serialize, limits: EngineLimits) -> Result<(), EngineError> {
    if serialized_json_size_bounded(value, limits.max_session_metadata_bytes.get())
        .map_err(|_| EngineError::Protocol("turn metadata serialization failed".to_owned()))?
        .is_none()
    {
        return Err(EngineError::Protocol(
            "turn metadata exceeds the configured byte limit".to_owned(),
        ));
    }
    Ok(())
}
fn validate_entry_bytes(key: &str, value: &Value, limits: EngineLimits) -> Result<(), EngineError> {
    validate_bytes(&BTreeMap::from([(key, value)]), limits)
}
fn validate_candidate(
    record: &SessionRecord,
    key: &str,
    replacement: Option<&Value>,
    limits: EngineLimits,
) -> Result<(), EngineError> {
    let mut metadata: BTreeMap<_, _> = record
        .metadata
        .iter()
        .filter(|(name, _)| name.as_str() != key)
        .map(|(name, value)| (name.as_str(), value))
        .collect();
    if let Some(value) = replacement {
        metadata.insert(key, value);
    }
    let messages = record.messages.iter().flat_map(|message| {
        message.content.iter().filter_map(|block| match block {
            ContentBlock::Json { value } => Some(value),
            ContentBlock::ToolCall { call } => Some(&call.arguments),
            ContentBlock::ToolResult { output, .. } => Some(&output.content),
            ContentBlock::Text { .. } => None,
        })
    });
    validate_json_roots(metadata.values().copied().chain(messages), limits).map_err(limit_error)?;
    validate_bytes(&metadata, limits)
}
