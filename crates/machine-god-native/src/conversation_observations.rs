//! Bounded explicitly routed observations, without persistence or ambient effects.
use crate::conversation_history::{
    NativeHistoryFileEvidence, NativeHistoryFileSource, NativeHistoryFileStatus,
};
use crate::session_store::{MAX_FILE_SESSION_BYTES, MAX_STORED_JSON_NODES};
use machine_god_core::{SessionId, SessionIncarnationId, ToolContext, TurnId};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

const MAX_ROUTES: usize = 64;
const ENTRY_NODES: usize = 12 * 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeObservationError {
    Duplicate,
    Capacity,
    Identity,
    Exhausted,
}
impl fmt::Display for NativeObservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native conversation observation unavailable")
    }
}
impl std::error::Error for NativeObservationError {}

/// Shared weak routes; sessions, not this table, retain their pending facts.
#[derive(Default)]
pub struct NativeConversationObservations {
    routes: Mutex<Vec<Weak<ObservationSession>>>,
}
impl fmt::Debug for NativeConversationObservations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeConversationObservations { .. }")
    }
}
impl NativeConversationObservations {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn register(
        self: &Arc<Self>,
        session: SessionId,
        incarnation: SessionIncarnationId,
    ) -> Result<Arc<ObservationSession>, NativeObservationError> {
        let mut routes = lock(&self.routes);
        routes.retain(|route| route.strong_count() != 0);
        if routes
            .iter()
            .filter_map(Weak::upgrade)
            .any(|route| route.session == session && route.incarnation == incarnation)
        {
            return Err(NativeObservationError::Duplicate);
        }
        if routes.len() == MAX_ROUTES {
            return Err(NativeObservationError::Capacity);
        }
        let owner = Arc::new(ObservationSession {
            session,
            incarnation,
            state: Mutex::new(State::default()),
            routes: Arc::downgrade(self),
        });
        routes.push(Arc::downgrade(&owner));
        Ok(owner)
    }
    pub(crate) fn reserve(
        &self,
        context: &ToolContext,
        path: &str,
        destination: Option<&str>,
        action: crate::NativeHistoryFileAction,
        name: &str,
    ) -> Result<ObservationReservation, NativeObservationError> {
        let owner = lock(&self.routes)
            .iter()
            .filter_map(Weak::upgrade)
            .find(|route| route.matches(context))
            .ok_or(NativeObservationError::Identity)?;
        owner.reserve(context, path, destination, action, name)
    }
}

#[derive(Default)]
struct State {
    retired: bool,
    active: Option<Attempt>,
    last_sequence: u64,
    next_version: u64,
    bytes: usize,
    entries: Vec<Entry>,
}
struct Attempt {
    turn: TurnId,
    first: usize,
    sequence: u64,
    binding: Option<NativeHistoryFileSource>,
    last_source: Option<(usize, usize)>,
}
struct Entry {
    id: u64,
    version: u64,
    finalized: bool,
    bytes: usize,
    fact: FileObservation,
}
pub(crate) struct ObservationSession {
    session: SessionId,
    incarnation: SessionIncarnationId,
    state: Mutex<State>,
    routes: Weak<NativeConversationObservations>,
}
#[derive(Clone)]
pub(crate) struct FileObservation {
    first: usize,
    sequence: u64,
    file: NativeHistoryFileEvidence,
}
impl FileObservation {
    pub(crate) const fn first_user_message(&self) -> usize {
        self.first
    }
    pub(crate) const fn turn_sequence(&self) -> u64 {
        self.sequence
    }
    pub(crate) const fn file(&self) -> &NativeHistoryFileEvidence {
        &self.file
    }
}
pub(crate) struct ObservationBatch {
    owner: Weak<ObservationSession>,
    versions: Vec<(u64, u64)>,
    entries: Vec<FileObservation>,
}
impl ObservationBatch {
    pub(crate) fn entries(&self) -> &[FileObservation] {
        &self.entries
    }
}
impl ObservationSession {
    pub(crate) fn retire(self: &Arc<Self>) {
        {
            let mut state = lock(&self.state);
            state.retired = true;
            state.active = None;
        }
        // Pending facts remain owned, including late settlements of already
        // admitted effects. Detachment is not a persistence acknowledgment.
        if let Some(routes) = self.routes.upgrade() {
            lock(&routes.routes).retain(|route| !Weak::ptr_eq(route, &Arc::downgrade(self)));
        }
    }
    fn matches(&self, context: &ToolContext) -> bool {
        self.session == context.session_id && self.incarnation == context.session_incarnation_id
    }
    pub(crate) fn begin_attempt(
        &self,
        turn: TurnId,
        first_user: usize,
        sequence: u64,
    ) -> Result<(), NativeObservationError> {
        let mut state = lock(&self.state);
        if state.retired
            || state.active.is_some()
            || sequence == 0
            || sequence <= state.last_sequence
            || first_user > MAX_FILE_SESSION_BYTES
        {
            return Err(NativeObservationError::Identity);
        }
        state.last_sequence = sequence;
        state.active = Some(Attempt {
            turn,
            first: first_user,
            sequence,
            binding: None,
            last_source: None,
        });
        Ok(())
    }
    pub(crate) fn bind_call(
        &self,
        context: &ToolContext,
        source: NativeHistoryFileSource,
    ) -> Result<(), NativeObservationError> {
        if !self.matches(context) || source.call_id() != &context.call_id {
            return Err(NativeObservationError::Identity);
        }
        let mut state = lock(&self.state);
        if state.retired {
            return Err(NativeObservationError::Identity);
        }
        let attempt = state
            .active
            .as_mut()
            .filter(|attempt| attempt.turn == context.turn_id)
            .ok_or(NativeObservationError::Identity)?;
        let key = (source.assistant_message(), source.content_block());
        if source.assistant_message() <= attempt.first
            || attempt.last_source.is_some_and(|previous| previous >= key)
        {
            return Err(NativeObservationError::Identity);
        }
        attempt.last_source = Some(key);
        attempt.binding = Some(source);
        Ok(())
    }
    pub(crate) fn finish_attempt(&self, turn: &TurnId) {
        let mut state = lock(&self.state);
        if state
            .active
            .as_ref()
            .is_some_and(|attempt| &attempt.turn == turn)
        {
            state.active = None;
        }
    }
    pub(crate) fn snapshot(self: &Arc<Self>) -> ObservationBatch {
        let state = lock(&self.state);
        ObservationBatch {
            owner: Arc::downgrade(self),
            versions: state
                .entries
                .iter()
                .map(|entry| (entry.id, entry.version))
                .collect(),
            entries: state
                .entries
                .iter()
                .map(|entry| entry.fact.clone())
                .collect(),
        }
    }
    pub(crate) fn acknowledge(self: &Arc<Self>, batch: &ObservationBatch) {
        if !Weak::ptr_eq(&batch.owner, &Arc::downgrade(self)) {
            return;
        }
        let mut state = lock(&self.state);
        state.entries.retain(|entry| {
            !entry.finalized
                || batch
                    .versions
                    .binary_search(&(entry.id, entry.version))
                    .is_err()
        });
        state.bytes = state.entries.iter().map(|entry| entry.bytes).sum();
    }
    fn reserve(
        self: &Arc<Self>,
        context: &ToolContext,
        path: &str,
        destination: Option<&str>,
        action: crate::NativeHistoryFileAction,
        name: &str,
    ) -> Result<ObservationReservation, NativeObservationError> {
        let mut state = lock(&self.state);
        if state.retired {
            return Err(NativeObservationError::Identity);
        }
        let attempt = state
            .active
            .as_ref()
            .filter(|attempt| attempt.turn == context.turn_id)
            .ok_or(NativeObservationError::Identity)?;
        let source = attempt
            .binding
            .as_ref()
            .filter(|source| {
                source.call_id() == &context.call_id && source.tool_name().as_str() == name
            })
            .ok_or(NativeObservationError::Identity)?;
        let first = attempt.first;
        let sequence = attempt.sequence;
        let make = |status, full| {
            crate::NativeHistoryFileEvidence::new(path, action, false)
                .and_then(|file| file.with_execution(source.clone(), status, destination, full))
                .map_err(|_| NativeObservationError::Identity)
        };
        let unknown = make(NativeHistoryFileStatus::Unknown, false)?;
        let success = make(NativeHistoryFileStatus::Success, false)?;
        let failure = make(NativeHistoryFileStatus::Failure, false)?;
        let full = if action == crate::NativeHistoryFileAction::Read {
            Some(make(NativeHistoryFileStatus::Success, true)?)
        } else {
            None
        };
        // Four owned shallow variants are the worst-case reservation footprint;
        // count the same overhead for batches so concurrent settlement never grows it.
        let bytes = serde_json::to_vec(&unknown)
            .map_err(|_| NativeObservationError::Capacity)?
            .len()
            .checked_add(128)
            .and_then(|value| value.checked_mul(4))
            .ok_or(NativeObservationError::Capacity)?;
        if state.entries.len() >= MAX_STORED_JSON_NODES / ENTRY_NODES
            || bytes > MAX_FILE_SESSION_BYTES.saturating_sub(state.bytes)
        {
            return Err(NativeObservationError::Capacity);
        }
        let id = state
            .next_version
            .checked_add(1)
            .ok_or(NativeObservationError::Exhausted)?;
        let settled = id.checked_add(1).ok_or(NativeObservationError::Exhausted)?;
        state.next_version = settled;
        state.bytes += bytes;
        state
            .active
            .as_mut()
            .expect("validated active attempt")
            .binding = None;
        state.entries.push(Entry {
            id,
            version: id,
            finalized: false,
            bytes,
            fact: FileObservation {
                first,
                sequence,
                file: unknown,
            },
        });
        Ok(ObservationReservation {
            owner: Arc::clone(self),
            id,
            settled,
            success: Some(success),
            failure: Some(failure),
            full,
            done: false,
        })
    }
}
pub(crate) struct ObservationReservation {
    owner: Arc<ObservationSession>,
    id: u64,
    settled: u64,
    success: Option<NativeHistoryFileEvidence>,
    failure: Option<NativeHistoryFileEvidence>,
    full: Option<NativeHistoryFileEvidence>,
    done: bool,
}
impl ObservationReservation {
    pub(crate) fn settle(mut self, success: bool, full: bool) {
        let file = if !success {
            self.failure.take()
        } else if full {
            self.full.take().or_else(|| self.success.take())
        } else {
            self.success.take()
        }
        .expect("reserved settlement variant");
        let mut state = lock(&self.owner.state);
        if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == self.id) {
            entry.fact.file = file;
            entry.version = self.settled;
            entry.finalized = true;
        }
        self.done = true;
    }
}
impl Drop for ObservationReservation {
    fn drop(&mut self) {
        if !self.done {
            let mut state = lock(&self.owner.state);
            if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == self.id) {
                entry.version = self.settled;
                entry.finalized = true;
            }
        }
    }
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use machine_god_core::{ToolCallId, ToolName};
    pub(crate) fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("opaque-turn").unwrap(),
            call_id: ToolCallId::new("reused").unwrap(),
        }
    }
    pub(crate) fn source(message: usize, name: &str) -> NativeHistoryFileSource {
        NativeHistoryFileSource::new(
            message,
            0,
            ToolCallId::new("reused").unwrap(),
            ToolName::new(name).unwrap(),
        )
        .unwrap()
    }
    pub(crate) fn setup(
        name: &str,
    ) -> (
        Arc<NativeConversationObservations>,
        Arc<ObservationSession>,
        ToolContext,
    ) {
        let registry = Arc::new(NativeConversationObservations::new());
        let context = context();
        let session = registry
            .register(
                context.session_id.clone(),
                context.session_incarnation_id.clone(),
            )
            .unwrap();
        session
            .begin_attempt(context.turn_id.clone(), 0, 1)
            .unwrap();
        session.bind_call(&context, source(1, name)).unwrap();
        (registry, session, context)
    }
    #[test]
    fn retirement_detaches_exact_route_without_acknowledging_pending_facts() {
        let (registry, old, context) = setup("write_file");
        let pending = registry
            .reserve(
                &context,
                "/file",
                None,
                crate::NativeHistoryFileAction::Write,
                "write_file",
            )
            .unwrap();
        old.retire();
        assert!(
            registry
                .reserve(
                    &context,
                    "/other",
                    None,
                    crate::NativeHistoryFileAction::Write,
                    "write_file"
                )
                .is_err()
        );
        assert_eq!(old.snapshot().entries().len(), 1);
        let replacement = registry
            .register(
                context.session_id.clone(),
                context.session_incarnation_id.clone(),
            )
            .unwrap();
        old.retire();
        pending.settle(true, false);
        assert_eq!(
            old.snapshot().entries()[0].file().status(),
            NativeHistoryFileStatus::Success
        );
        drop(old);
        replacement
            .begin_attempt(context.turn_id.clone(), 0, 1)
            .unwrap();
        replacement
            .bind_call(&context, source(1, "write_file"))
            .unwrap();
        registry
            .reserve(
                &context,
                "/new",
                None,
                crate::NativeHistoryFileAction::Write,
                "write_file",
            )
            .unwrap()
            .settle(true, false);
        assert_eq!(replacement.snapshot().entries().len(), 1);
    }

    #[test]
    fn routes_are_exact_weak_bounded_and_reusable() {
        let registry = Arc::new(NativeConversationObservations::new());
        let mut owners = Vec::new();
        for index in 0..64 {
            owners.push(
                registry
                    .register(
                        SessionId::new(format!("s-{index}")).unwrap(),
                        SessionIncarnationId::new("i").unwrap(),
                    )
                    .unwrap(),
            );
        }
        assert!(matches!(
            registry.register(
                SessionId::new("s-0").unwrap(),
                SessionIncarnationId::new("i").unwrap()
            ),
            Err(NativeObservationError::Duplicate)
        ));
        assert!(matches!(
            registry.register(
                SessionId::new("extra").unwrap(),
                SessionIncarnationId::new("i").unwrap()
            ),
            Err(NativeObservationError::Capacity)
        ));
        owners.pop();
        assert!(
            registry
                .register(
                    SessionId::new("extra").unwrap(),
                    SessionIncarnationId::new("i").unwrap()
                )
                .is_ok()
        );
    }
    #[test]
    fn snapshots_cannot_erase_live_or_newer_settlements() {
        let (registry, session, context) = setup("write_file");
        let reservation = registry
            .reserve(
                &context,
                "a",
                None,
                crate::NativeHistoryFileAction::Write,
                "write_file",
            )
            .unwrap();
        let unknown = session.snapshot();
        session.acknowledge(&unknown);
        assert_eq!(session.snapshot().entries().len(), 1);
        reservation.settle(true, false);
        session.acknowledge(&unknown);
        let complete = session.snapshot();
        assert_eq!(
            complete.entries()[0].file().status(),
            NativeHistoryFileStatus::Success
        );
        assert_eq!(complete.entries()[0].first_user_message(), 0);
        assert_eq!(complete.entries()[0].turn_sequence(), 1);
        session.acknowledge(&complete);
        assert!(session.snapshot().entries().is_empty());
    }
    #[test]
    fn dropped_execution_retains_unknown_after_attempt_retirement() {
        let (registry, session, context) = setup("read_file");
        let reservation = registry
            .reserve(
                &context,
                "a",
                None,
                crate::NativeHistoryFileAction::Read,
                "read_file",
            )
            .unwrap();
        let before = session.snapshot();
        session.finish_attempt(&context.turn_id);
        drop(reservation);
        session.acknowledge(&before);
        let current = session.snapshot();
        assert_eq!(
            current.entries()[0].file().status(),
            NativeHistoryFileStatus::Unknown
        );
        session.acknowledge(&current);
        assert!(session.snapshot().entries().is_empty());
    }
    #[test]
    fn mismatched_context_source_and_duplicate_execution_fail_before_reservation() {
        let (registry, session, context) = setup("read_file");
        let mut wrong = context.clone();
        wrong.session_incarnation_id = SessionIncarnationId::new("wrong").unwrap();
        assert!(
            registry
                .reserve(
                    &wrong,
                    "a",
                    None,
                    crate::NativeHistoryFileAction::Read,
                    "read_file"
                )
                .is_err()
        );
        assert!(session.bind_call(&context, source(1, "read_file")).is_err());
        assert!(
            registry
                .reserve(
                    &context,
                    "a",
                    None,
                    crate::NativeHistoryFileAction::Write,
                    "write_file"
                )
                .is_err()
        );
        let lease = registry
            .reserve(
                &context,
                "a",
                None,
                crate::NativeHistoryFileAction::Read,
                "read_file",
            )
            .unwrap();
        assert!(
            registry
                .reserve(
                    &context,
                    "a",
                    None,
                    crate::NativeHistoryFileAction::Read,
                    "read_file"
                )
                .is_err()
        );
        drop(lease);
        session.bind_call(&context, source(2, "read_file")).unwrap();
        assert!(
            registry
                .reserve(
                    &context,
                    "a",
                    None,
                    crate::NativeHistoryFileAction::Read,
                    "read_file"
                )
                .is_ok()
        );
    }
    #[test]
    fn capacity_and_version_exhaustion_are_pre_effect_and_settlement_is_reserved() {
        let (registry, session, context) = setup("write_file");
        lock(&session.state).next_version = u64::MAX - 1;
        assert!(matches!(
            registry.reserve(
                &context,
                "a",
                None,
                crate::NativeHistoryFileAction::Write,
                "write_file"
            ),
            Err(NativeObservationError::Exhausted)
        ));
        lock(&session.state).next_version = u64::MAX - 2;
        let lease = registry
            .reserve(
                &context,
                "a",
                None,
                crate::NativeHistoryFileAction::Write,
                "write_file",
            )
            .unwrap();
        lease.settle(true, false);
        assert_eq!(
            session.snapshot().entries()[0].file().status(),
            NativeHistoryFileStatus::Success
        );
        let (registry, session, context) = setup("read_file");
        let mut admitted = 0;
        for index in 1..=MAX_STORED_JSON_NODES {
            if index > 1 {
                session
                    .bind_call(&context, source(index, "read_file"))
                    .unwrap();
            }
            match registry.reserve(
                &context,
                "a",
                None,
                crate::NativeHistoryFileAction::Read,
                "read_file",
            ) {
                Ok(lease) => {
                    drop(lease);
                    admitted += 1;
                }
                Err(NativeObservationError::Capacity) => break,
                Err(error) => panic!("unexpected {error:?}"),
            }
        }
        assert_eq!(admitted, MAX_STORED_JSON_NODES / ENTRY_NODES);
        let batch = session.snapshot();
        session.acknowledge(&batch);
        assert!(
            registry
                .reserve(
                    &context,
                    "a",
                    None,
                    crate::NativeHistoryFileAction::Read,
                    "read_file"
                )
                .is_ok()
        );
    }
    #[test]
    fn a_foreign_batch_never_erases_this_sessions_pending_facts() {
        let (registry, session, context) = setup("read_file");
        drop(
            registry
                .reserve(
                    &context,
                    "a",
                    None,
                    crate::NativeHistoryFileAction::Read,
                    "read_file",
                )
                .unwrap(),
        );
        let (_, other, _) = setup("read_file");
        other.acknowledge(&session.snapshot());
        assert_eq!(session.snapshot().entries().len(), 1);
    }
}
