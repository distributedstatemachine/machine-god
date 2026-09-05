//! Bounded, process-local authority for writing managed command input.

use machine_god_core::BackgroundOutputOwner;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, TryLockError};

pub(crate) const MAX_BACKGROUND_INPUT_BYTES: usize = 8192;
const MAX_INPUT_ENTRIES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundInputStatus {
    Written,
    Backpressure,
    Closed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BackgroundInputReceipt {
    bytes_written: usize,
    stdin_closed: bool,
    status: BackgroundInputStatus,
}

impl BackgroundInputReceipt {
    pub(crate) const fn new(
        bytes_written: usize,
        stdin_closed: bool,
        status: BackgroundInputStatus,
    ) -> Self {
        Self {
            bytes_written,
            stdin_closed,
            status,
        }
    }
    pub(crate) const fn bytes_written(self) -> usize {
        self.bytes_written
    }
    pub(crate) const fn stdin_closed(self) -> bool {
        self.stdin_closed
    }
    pub(crate) const fn status(self) -> BackgroundInputStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundInputErrorKind {
    NotFound,
    Busy,
    Process,
    Capacity,
    Conflict,
    InvalidRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BackgroundInputError {
    kind: BackgroundInputErrorKind,
}

impl BackgroundInputError {
    pub(crate) const fn new(kind: BackgroundInputErrorKind) -> Self {
        Self { kind }
    }
    pub(crate) const fn kind(self) -> BackgroundInputErrorKind {
        self.kind
    }
}
impl fmt::Display for BackgroundInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("background input operation failed")
    }
}
impl std::error::Error for BackgroundInputError {}

pub(crate) trait BackgroundInputTarget: Send + Sync + 'static {
    fn write(&self, data: &[u8], eof: bool)
    -> Result<BackgroundInputReceipt, BackgroundInputError>;
}

#[derive(Clone, Default)]
pub(crate) struct BackgroundInputRegistry {
    state: Arc<Mutex<BTreeMap<NonZeroU64, Arc<InputEntry>>>>,
}

struct InputEntry {
    owner: BackgroundOutputOwner,
    target: Arc<dyn BackgroundInputTarget>,
    operation: Mutex<()>,
}

impl BackgroundInputRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn register(
        &self,
        id: NonZeroU64,
        owner: &BackgroundOutputOwner,
        target: Arc<dyn BackgroundInputTarget>,
    ) -> Result<BackgroundInputLease, BackgroundInputError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| BackgroundInputError::new(BackgroundInputErrorKind::Process))?;
        if state.contains_key(&id) {
            return Err(BackgroundInputError::new(
                BackgroundInputErrorKind::Conflict,
            ));
        }
        if state.len() == MAX_INPUT_ENTRIES {
            return Err(BackgroundInputError::new(
                BackgroundInputErrorKind::Capacity,
            ));
        }
        let entry = Arc::new(InputEntry {
            owner: owner.clone(),
            target,
            operation: Mutex::new(()),
        });
        state.insert(id, Arc::clone(&entry));
        Ok(BackgroundInputLease {
            id,
            entry,
            registry: self.clone(),
        })
    }

    pub(crate) fn write(
        &self,
        id: NonZeroU64,
        owner: &BackgroundOutputOwner,
        data: &[u8],
        eof: bool,
    ) -> Result<BackgroundInputReceipt, BackgroundInputError> {
        validate_input(data, eof)?;
        let entry = {
            let state = self.state.try_lock().map_err(|error| lock_error(&error))?;
            Arc::clone(
                state
                    .get(&id)
                    .filter(|entry| entry.owner == *owner)
                    .ok_or_else(|| BackgroundInputError::new(BackgroundInputErrorKind::NotFound))?,
            )
        };
        let _operation = entry
            .operation
            .try_lock()
            .map_err(|error| lock_error(&error))?;
        entry.target.write(data, eof)
    }
}

fn lock_error<T>(error: &TryLockError<T>) -> BackgroundInputError {
    BackgroundInputError::new(match error {
        TryLockError::WouldBlock => BackgroundInputErrorKind::Busy,
        TryLockError::Poisoned(_) => BackgroundInputErrorKind::Process,
    })
}

pub(crate) fn validate_input(data: &[u8], eof: bool) -> Result<(), BackgroundInputError> {
    if data.len() > MAX_BACKGROUND_INPUT_BYTES || (data.is_empty() && !eof) {
        return Err(BackgroundInputError::new(
            BackgroundInputErrorKind::InvalidRequest,
        ));
    }
    Ok(())
}

pub(crate) struct BackgroundInputLease {
    id: NonZeroU64,
    entry: Arc<InputEntry>,
    registry: BackgroundInputRegistry,
}
impl fmt::Debug for BackgroundInputLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundInputLease")
            .finish_non_exhaustive()
    }
}
impl Drop for BackgroundInputLease {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .get(&self.id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
        {
            state.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Target {
        calls: AtomicUsize,
    }
    impl BackgroundInputTarget for Target {
        fn write(
            &self,
            data: &[u8],
            eof: bool,
        ) -> Result<BackgroundInputReceipt, BackgroundInputError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(BackgroundInputReceipt::new(
                data.len(),
                eof,
                if eof {
                    BackgroundInputStatus::Closed
                } else {
                    BackgroundInputStatus::Written
                },
            ))
        }
    }
    fn owner(incarnation: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new(incarnation).unwrap(),
        )
    }
    fn id(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap()
    }

    #[test]
    fn exact_owner_bounded_admission_and_lease_cleanup() {
        let registry = BackgroundInputRegistry::new();
        let target = Arc::new(Target::default());
        let mut leases = Vec::new();
        for number in 1..=16 {
            leases.push(
                registry
                    .register(id(number), &owner("one"), target.clone())
                    .unwrap(),
            );
        }
        assert_eq!(
            registry
                .register(id(1), &owner("one"), target.clone())
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::Conflict
        );
        assert_eq!(
            registry
                .register(id(17), &owner("one"), target.clone())
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::Capacity
        );
        assert_eq!(
            registry
                .write(id(1), &owner("two"), b"x", false)
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::NotFound
        );
        assert_eq!(
            registry
                .write(id(17), &owner("one"), b"x", false)
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::NotFound
        );
        assert_eq!(target.calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            registry
                .write(id(1), &owner("one"), b"x", false)
                .unwrap()
                .bytes_written(),
            1
        );
        leases.clear();
        assert_eq!(
            registry
                .write(id(1), &owner("one"), b"x", false)
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::NotFound
        );
        registry.register(id(17), &owner("one"), target).unwrap();
    }

    #[test]
    fn invalid_requests_and_concurrent_operation_have_no_dispatch() {
        let registry = BackgroundInputRegistry::new();
        let target = Arc::new(Target::default());
        let lease = registry
            .register(id(1), &owner("one"), target.clone())
            .unwrap();
        assert_eq!(
            registry
                .write(
                    id(1),
                    &owner("one"),
                    &[0; MAX_BACKGROUND_INPUT_BYTES + 1],
                    true
                )
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::InvalidRequest
        );
        assert_eq!(
            registry
                .write(id(1), &owner("one"), b"", false)
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::InvalidRequest
        );
        let guard = lease.entry.operation.lock().unwrap();
        assert_eq!(
            registry
                .write(id(1), &owner("one"), b"x", false)
                .unwrap_err()
                .kind(),
            BackgroundInputErrorKind::Busy
        );
        assert_eq!(target.calls.load(Ordering::Relaxed), 0);
        drop(guard);
        assert!(
            registry
                .write(id(1), &owner("one"), b"", true)
                .unwrap()
                .stdin_closed()
        );
    }
}
