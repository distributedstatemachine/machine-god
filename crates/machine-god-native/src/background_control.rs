//! Process-local authority for signalling managed background commands.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, MutexGuard};

use machine_god_core::BackgroundOutputOwner;

/// Maximum managed background commands retained for process-local control.
pub(crate) const MAX_BACKGROUND_CONTROL_LIVE_ENTRIES: usize = 16;

/// One native signal accepted by a managed background control target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundSignal {
    Hangup,
    Interrupt,
    Quit,
    Terminate,
    Kill,
}

impl BackgroundSignal {
    /// Returns the stable lowercase signal name.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Hangup => "hangup",
            Self::Interrupt => "interrupt",
            Self::Quit => "quit",
            Self::Terminate => "terminate",
            Self::Kill => "kill",
        }
    }
}

/// Stable category for a process-local background-control failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundControlErrorKind {
    NotFound,
    Busy,
    Process,
    Capacity,
    Conflict,
}

/// Fixed, data-free process-local background-control failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct BackgroundControlError {
    kind: BackgroundControlErrorKind,
}

impl BackgroundControlError {
    /// Constructs one fixed failure for a native control implementation.
    pub(crate) const fn new(kind: BackgroundControlErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure category.
    pub(crate) const fn kind(self) -> BackgroundControlErrorKind {
        self.kind
    }
}

impl fmt::Debug for BackgroundControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundControlError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for BackgroundControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("background control operation failed")
    }
}

impl Error for BackgroundControlError {}

/// Explicit synchronous authority for signalling one managed process.
pub(crate) trait BackgroundControlTarget: Send + Sync + 'static {
    /// Delivers one signal without retaining the caller's registry lock.
    fn signal(&self, signal: BackgroundSignal) -> Result<(), BackgroundControlError>;
}

/// Cloneable, bounded registry of process-local background control targets.
#[derive(Clone, Default)]
pub(crate) struct BackgroundControlRegistry {
    state: Arc<Mutex<RegistryState>>,
}

impl BackgroundControlRegistry {
    /// Constructs one empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Registers one exact owner and target until the returned lease is dropped.
    pub(crate) fn register(
        &self,
        id: NonZeroU64,
        owner: &BackgroundOutputOwner,
        target: Arc<dyn BackgroundControlTarget>,
    ) -> Result<BackgroundControlLease, BackgroundControlError> {
        let mut state = self.lock()?;
        if state.entries.contains_key(&id) {
            return Err(error(BackgroundControlErrorKind::Conflict));
        }
        if state.entries.len() == MAX_BACKGROUND_CONTROL_LIVE_ENTRIES {
            return Err(error(BackgroundControlErrorKind::Capacity));
        }

        let entry = Arc::new(ControlEntry {
            owner: owner.clone(),
            target,
        });
        state.entries.insert(id, Arc::clone(&entry));
        Ok(BackgroundControlLease {
            id,
            entry,
            state: Arc::clone(&self.state),
        })
    }

    /// Signals the target registered for the exact ID and owner pair.
    pub(crate) fn signal(
        &self,
        id: NonZeroU64,
        owner: &BackgroundOutputOwner,
        signal: BackgroundSignal,
    ) -> Result<(), BackgroundControlError> {
        let target = {
            let state = self.lock()?;
            let entry = state
                .entries
                .get(&id)
                .filter(|entry| entry.owner == *owner)
                .ok_or_else(|| error(BackgroundControlErrorKind::NotFound))?;
            Arc::clone(&entry.target)
        };
        target.signal(signal)
    }

    fn lock(&self) -> Result<MutexGuard<'_, RegistryState>, BackgroundControlError> {
        self.state
            .lock()
            .map_err(|_| error(BackgroundControlErrorKind::Process))
    }
}

/// Exclusive registration lifetime for one exact control entry.
pub(crate) struct BackgroundControlLease {
    id: NonZeroU64,
    entry: Arc<ControlEntry>,
    state: Arc<Mutex<RegistryState>>,
}

impl fmt::Debug for BackgroundControlLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundControlLease")
            .finish_non_exhaustive()
    }
}

impl Drop for BackgroundControlLease {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .entries
            .get(&self.id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
        {
            state.entries.remove(&self.id);
        }
    }
}

#[derive(Default)]
struct RegistryState {
    entries: BTreeMap<NonZeroU64, Arc<ControlEntry>>,
}

struct ControlEntry {
    owner: BackgroundOutputOwner,
    target: Arc<dyn BackgroundControlTarget>,
}

const fn error(kind: BackgroundControlErrorKind) -> BackgroundControlError {
    BackgroundControlError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, TryLockError, Weak};
    use std::thread;

    use machine_god_core::{BackgroundOutputOwner, SessionId, SessionIncarnationId};

    use super::{
        BackgroundControlError, BackgroundControlErrorKind, BackgroundControlRegistry,
        BackgroundControlTarget, BackgroundSignal, ControlEntry,
        MAX_BACKGROUND_CONTROL_LIVE_ENTRIES, RegistryState, error,
    };

    struct CountingTarget {
        calls: AtomicUsize,
    }

    impl CountingTarget {
        const fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl BackgroundControlTarget for CountingTarget {
        fn signal(&self, _signal: BackgroundSignal) -> Result<(), BackgroundControlError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    struct LockProbeTarget {
        state: Weak<std::sync::Mutex<RegistryState>>,
    }

    impl BackgroundControlTarget for LockProbeTarget {
        fn signal(&self, _signal: BackgroundSignal) -> Result<(), BackgroundControlError> {
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| error(BackgroundControlErrorKind::Process))?;
            match state.try_lock() {
                Ok(_) => Ok(()),
                Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {
                    Err(error(BackgroundControlErrorKind::Busy))
                }
            }
        }
    }

    fn id(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap()
    }

    fn owner(name: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new(format!("session-{name}")).unwrap(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
    }

    #[test]
    fn registration_is_bounded_and_lease_drop_releases_capacity() {
        let registry = BackgroundControlRegistry::new();
        let owner = owner("capacity");
        let mut leases = Vec::new();
        for value in 1..=MAX_BACKGROUND_CONTROL_LIVE_ENTRIES as u64 {
            leases.push(
                registry
                    .register(id(value), &owner, Arc::new(CountingTarget::new()))
                    .unwrap(),
            );
        }
        assert_eq!(
            registry
                .register(id(17), &owner, Arc::new(CountingTarget::new()))
                .unwrap_err()
                .kind(),
            BackgroundControlErrorKind::Capacity
        );

        leases.pop();
        registry
            .register(id(17), &owner, Arc::new(CountingTarget::new()))
            .unwrap();
    }

    #[test]
    fn duplicate_registration_is_a_conflict_without_replacing_the_target() {
        let registry = BackgroundControlRegistry::new();
        let owner = owner("duplicate");
        let first = Arc::new(CountingTarget::new());
        let _lease = registry.register(id(1), &owner, first.clone()).unwrap();

        assert_eq!(
            registry
                .register(id(1), &owner, Arc::new(CountingTarget::new()))
                .unwrap_err()
                .kind(),
            BackgroundControlErrorKind::Conflict
        );
        registry
            .signal(id(1), &owner, BackgroundSignal::Terminate)
            .unwrap();
        assert_eq!(first.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn unknown_and_wrong_owner_are_indistinguishable() {
        let registry = BackgroundControlRegistry::new();
        let exact_owner = owner("exact");
        let wrong_owner = owner("wrong");
        let _lease = registry
            .register(id(1), &exact_owner, Arc::new(CountingTarget::new()))
            .unwrap();

        let unknown = registry
            .signal(id(2), &exact_owner, BackgroundSignal::Interrupt)
            .unwrap_err();
        let unauthorized = registry
            .signal(id(1), &wrong_owner, BackgroundSignal::Interrupt)
            .unwrap_err();
        assert_eq!(unknown, unauthorized);
        assert_eq!(unknown.kind(), BackgroundControlErrorKind::NotFound);
        assert_eq!(format!("{unknown}"), "background control operation failed");
        assert_eq!(
            format!("{unknown:?}"),
            "BackgroundControlError { kind: NotFound }"
        );
    }

    #[test]
    fn error_rendering_and_signal_names_are_fixed_and_data_free() {
        for kind in [
            BackgroundControlErrorKind::NotFound,
            BackgroundControlErrorKind::Busy,
            BackgroundControlErrorKind::Process,
            BackgroundControlErrorKind::Capacity,
            BackgroundControlErrorKind::Conflict,
        ] {
            let error = BackgroundControlError::new(kind);
            assert_eq!(error.to_string(), "background control operation failed");
            assert!(error.source().is_none());
        }
        assert_eq!(BackgroundSignal::Hangup.as_str(), "hangup");
        assert_eq!(BackgroundSignal::Interrupt.as_str(), "interrupt");
        assert_eq!(BackgroundSignal::Quit.as_str(), "quit");
        assert_eq!(BackgroundSignal::Terminate.as_str(), "terminate");
        assert_eq!(BackgroundSignal::Kill.as_str(), "kill");
    }

    #[test]
    fn registry_lock_is_released_before_the_target_is_called() {
        let registry = BackgroundControlRegistry::new();
        let owner = owner("lock-probe");
        let target = Arc::new(LockProbeTarget {
            state: Arc::downgrade(&registry.state),
        });
        let _lease = registry.register(id(1), &owner, target).unwrap();

        registry
            .signal(id(1), &owner, BackgroundSignal::Quit)
            .unwrap();
    }

    #[test]
    fn stale_lease_does_not_remove_a_replacement_entry() {
        let registry = BackgroundControlRegistry::new();
        let owner = owner("replacement");
        let stale_lease = registry
            .register(id(1), &owner, Arc::new(CountingTarget::new()))
            .unwrap();
        let replacement_target = Arc::new(CountingTarget::new());
        let replacement = Arc::new(ControlEntry {
            owner: owner.clone(),
            target: replacement_target.clone(),
        });
        registry
            .state
            .lock()
            .unwrap()
            .entries
            .insert(id(1), replacement);

        drop(stale_lease);
        registry
            .signal(id(1), &owner, BackgroundSignal::Kill)
            .unwrap();
        assert_eq!(replacement_target.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_signals_share_one_registered_target_safely() {
        const THREADS: usize = 8;
        const CALLS_PER_THREAD: usize = 64;

        let registry = BackgroundControlRegistry::new();
        let owner = owner("concurrent");
        let target = Arc::new(CountingTarget::new());
        let _lease = registry.register(id(1), &owner, target.clone()).unwrap();
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut threads = Vec::new();
        for _ in 0..THREADS {
            let registry = registry.clone();
            let owner = owner.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..CALLS_PER_THREAD {
                    registry
                        .signal(id(1), &owner, BackgroundSignal::Terminate)
                        .unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(
            target.calls.load(Ordering::Relaxed),
            THREADS * CALLS_PER_THREAD
        );
    }

    #[test]
    fn poisoned_registry_operations_return_the_fixed_process_failure() {
        let registry = BackgroundControlRegistry::new();
        let state = Arc::clone(&registry.state);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = state.lock().unwrap();
            panic!("poison registry for test");
        }));
        let owner = owner("poison");

        assert_eq!(
            registry
                .register(id(1), &owner, Arc::new(CountingTarget::new()))
                .unwrap_err()
                .kind(),
            BackgroundControlErrorKind::Process
        );
        assert_eq!(
            registry
                .signal(id(1), &owner, BackgroundSignal::Hangup)
                .unwrap_err()
                .kind(),
            BackgroundControlErrorKind::Process
        );
    }
}
