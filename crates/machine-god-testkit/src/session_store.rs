use machine_god_core::{
    BoxFuture, SessionId, SessionRecord, SessionRevision, SessionStore, SessionStoreError,
    SessionStoreErrorKind,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::DEFAULT_RECORD_CAPACITY;

/// One ordered response for a scripted load or save operation.
#[derive(Clone, Debug)]
pub enum SessionStoreStep {
    /// Execute the normal in-memory operation.
    Pass,
    /// Return the supplied failure without reading or mutating records.
    Error(SessionStoreError),
    /// Never resolve and do not read or mutate records.
    Pending,
}

/// Independent strict scripts for load and save calls.
///
/// `None` selects normal pass-through behavior for that operation. `Some`
/// selects a strict finite script, including when the vector is empty.
#[derive(Clone, Debug, Default)]
pub struct SessionStoreScript {
    pub loads: Option<Vec<SessionStoreStep>>,
    pub saves: Option<Vec<SessionStoreStep>>,
}

/// One store call captured before its behavior starts.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordedSessionStoreCall {
    Load {
        id: SessionId,
    },
    Save {
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    },
}

#[derive(Debug)]
struct StoreState {
    records: BTreeMap<SessionId, SessionRecord>,
    loads: Option<VecDeque<SessionStoreStep>>,
    saves: Option<VecDeque<SessionStoreStep>>,
    calls: Vec<RecordedSessionStoreCall>,
}

#[derive(Debug)]
struct StoreInner {
    record_capacity: usize,
    state: Mutex<StoreState>,
}

/// An atomic optimistic-CAS store with deterministic failures and inspection.
#[derive(Clone, Debug)]
pub struct InMemorySessionStore {
    inner: Arc<StoreInner>,
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySessionStore {
    /// Creates an empty pass-through store with the default call-recording bound.
    #[must_use]
    pub fn new() -> Self {
        Self::configured(
            BTreeMap::new(),
            SessionStoreScript::default(),
            DEFAULT_RECORD_CAPACITY,
        )
    }

    /// Creates a pass-through store seeded with the supplied records.
    #[must_use]
    pub fn from_records(records: BTreeMap<SessionId, SessionRecord>) -> Self {
        Self::configured(
            records,
            SessionStoreScript::default(),
            DEFAULT_RECORD_CAPACITY,
        )
    }

    /// Creates a store with explicit initial state, operation scripts, and
    /// retained-call bound.
    #[must_use]
    pub fn configured(
        records: BTreeMap<SessionId, SessionRecord>,
        script: SessionStoreScript,
        record_capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                record_capacity,
                state: Mutex::new(StoreState {
                    records,
                    loads: script.loads.map(VecDeque::from),
                    saves: script.saves.map(VecDeque::from),
                    calls: Vec::new(),
                }),
            }),
        }
    }

    /// Returns one stored record snapshot.
    #[must_use]
    pub fn record(&self, id: &SessionId) -> Option<SessionRecord> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .get(id)
            .cloned()
    }

    /// Returns a consistent snapshot of every stored record.
    #[must_use]
    pub fn records(&self) -> BTreeMap<SessionId, SessionRecord> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .clone()
    }

    /// Returns a consistent snapshot in operation-call order.
    #[must_use]
    pub fn calls(&self) -> Vec<RecordedSessionStoreCall> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .calls
            .clone()
    }

    /// Returns remaining strict load and save step counts. `None` denotes
    /// pass-through behavior.
    #[must_use]
    pub fn remaining_steps(&self) -> (Option<usize>, Option<usize>) {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            state.loads.as_ref().map(VecDeque::len),
            state.saves.as_ref().map(VecDeque::len),
        )
    }

    fn record_call(
        state: &mut StoreState,
        capacity: usize,
        call: RecordedSessionStoreCall,
    ) -> Result<(), SessionStoreError> {
        if state.calls.len() >= capacity {
            return Err(store_fixture_error(
                "testkit_record_capacity_exhausted",
                "in-memory session store call recording capacity was exhausted",
            ));
        }
        state.calls.push(call);
        Ok(())
    }

    fn next_step(
        script: &mut Option<VecDeque<SessionStoreStep>>,
        operation: &'static str,
    ) -> Result<SessionStoreStep, SessionStoreError> {
        match script {
            None => Ok(SessionStoreStep::Pass),
            Some(steps) => steps.pop_front().ok_or_else(|| {
                store_fixture_error(
                    "testkit_script_exhausted",
                    match operation {
                        "load" => {
                            "in-memory session store received a load after its script was exhausted"
                        }
                        _ => {
                            "in-memory session store received a save after its script was exhausted"
                        }
                    },
                )
            }),
        }
    }

    fn load_now(
        &self,
        id: &SessionId,
    ) -> Result<(SessionStoreStep, Option<SessionRecord>), SessionStoreError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::record_call(
            &mut state,
            self.inner.record_capacity,
            RecordedSessionStoreCall::Load { id: id.clone() },
        )?;
        let step = Self::next_step(&mut state.loads, "load")?;
        let record = matches!(step, SessionStoreStep::Pass)
            .then(|| state.records.get(id).cloned())
            .flatten();
        Ok((step, record))
    }

    fn save_now(
        &self,
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> Result<(SessionStoreStep, Option<SessionRevision>), SessionStoreError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::record_call(
            &mut state,
            self.inner.record_capacity,
            RecordedSessionStoreCall::Save {
                record: record.clone(),
                expected_revision,
            },
        )?;
        let step = Self::next_step(&mut state.saves, "save")?;
        if !matches!(step, SessionStoreStep::Pass) {
            return Ok((step, None));
        }

        let current = state.records.get(&record.id);
        if current.is_some_and(|stored| stored.incarnation_id != record.incarnation_id) {
            return Err(SessionStoreError::new(
                SessionStoreErrorKind::Conflict,
                "incarnation_conflict",
                "stored session incarnation did not match the saved record",
                false,
            ));
        }
        let current_revision = current.map(|stored| stored.revision);
        if current_revision != expected_revision {
            return Err(SessionStoreError::new(
                SessionStoreErrorKind::Conflict,
                "revision_conflict",
                "stored session revision did not match the expected revision",
                true,
            ));
        }
        let revision_base = current_revision.unwrap_or_default().max(record.revision);
        let revision = SessionRevision(revision_base.0.checked_add(1).ok_or_else(|| {
            SessionStoreError::new(
                SessionStoreErrorKind::Other,
                "revision_exhausted",
                "session revision counter was exhausted",
                false,
            )
        })?);
        let mut stored = record;
        stored.revision = revision;
        state.records.insert(stored.id.clone(), stored);
        Ok((step, Some(revision)))
    }
}

impl SessionStore for InMemorySessionStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let result = self.load_now(&id);
        Box::pin(async move {
            let (step, record) = result?;
            match step {
                SessionStoreStep::Pass => Ok(record),
                SessionStoreStep::Error(error) => Err(error),
                SessionStoreStep::Pending => core::future::pending().await,
            }
        })
    }

    fn save(
        &self,
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let result = self.save_now(record, expected_revision);
        Box::pin(async move {
            let (step, revision) = result?;
            match step {
                SessionStoreStep::Pass => revision.ok_or_else(|| {
                    store_fixture_error(
                        "testkit_internal_state",
                        "pass-through save did not produce a revision",
                    )
                }),
                SessionStoreStep::Error(error) => Err(error),
                SessionStoreStep::Pending => core::future::pending().await,
            }
        })
    }
}

fn store_fixture_error(code: &'static str, message: &'static str) -> SessionStoreError {
    SessionStoreError::new(SessionStoreErrorKind::Other, code, message, false)
}

#[cfg(test)]
mod tests {
    use super::InMemorySessionStore;
    use machine_god_core::{
        SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
        SessionStoreErrorKind,
    };

    fn empty_record(id: SessionId) -> SessionRecord {
        let incarnation = SessionIncarnationId::new(format!("test-incarnation-{id}"))
            .expect("test incarnation is valid");
        SessionRecord::empty(id, incarnation)
    }

    #[test]
    fn compare_and_swap_is_atomic_across_threads() {
        let store = InMemorySessionStore::new();
        let id = SessionId::new("atomic-cas").unwrap();
        let mut initial = empty_record(id.clone());
        initial.next_turn_sequence = 2;
        assert_eq!(
            futures_executor::block_on(store.save(initial, None)).unwrap(),
            SessionRevision(1)
        );

        let mut workers = Vec::new();
        for sequence in [3, 4] {
            let store = store.clone();
            let mut candidate = store.record(&id).unwrap();
            candidate.next_turn_sequence = sequence;
            workers.push(std::thread::spawn(move || {
                futures_executor::block_on(store.save(candidate, Some(SessionRevision(1))))
            }));
        }
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert_eq!(store.record(&id).unwrap().revision, SessionRevision(2));
    }

    #[test]
    fn returned_revision_is_greater_than_an_unpersisted_input_revision() {
        let store = InMemorySessionStore::new();
        let mut record = empty_record(SessionId::new("high-input-revision").unwrap());
        record.revision = SessionRevision(41);
        assert_eq!(
            futures_executor::block_on(store.save(record, None)).unwrap(),
            SessionRevision(42)
        );
    }

    #[test]
    fn stored_session_incarnation_cannot_change() {
        let store = InMemorySessionStore::new();
        let id = SessionId::new("incarnation-cas").unwrap();
        let original = empty_record(id.clone());
        assert_eq!(
            futures_executor::block_on(store.save(original.clone(), None)).unwrap(),
            SessionRevision(1)
        );

        let replacement = SessionRecord::empty(
            id.clone(),
            SessionIncarnationId::new("replacement-incarnation").unwrap(),
        );
        let error = futures_executor::block_on(store.save(replacement, Some(SessionRevision(1))))
            .unwrap_err();

        assert_eq!(error.kind, SessionStoreErrorKind::Conflict);
        assert_eq!(error.code, "incarnation_conflict");
        assert_eq!(
            store.record(&id).unwrap().incarnation_id,
            original.incarnation_id
        );
    }

    #[test]
    fn recovers_a_poisoned_state_lock() {
        let store = InMemorySessionStore::new();
        let inner = store.inner.clone();
        let _ = std::thread::spawn(move || {
            let _guard = inner.state.lock().unwrap();
            panic!("poison store state");
        })
        .join();
        assert!(store.records().is_empty());
    }
}
