//! One publication barrier for a prepared conversation's shared service routes.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const STAGED: u8 = 0;
const ACTIVE: u8 = 1;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
const RETIRED: u8 = 2;

#[derive(Clone)]
pub(crate) struct RoutePublication(Arc<AtomicU8>);

impl Default for RoutePublication {
    fn default() -> Self {
        Self(Arc::new(AtomicU8::new(ACTIVE)))
    }
}

impl RoutePublication {
    pub(crate) fn is_staged(&self) -> bool {
        self.0.load(Ordering::Acquire) == STAGED
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn staged() -> Self {
        Self(Arc::new(AtomicU8::new(STAGED)))
    }

    pub(crate) fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire) == ACTIVE
    }

    /// Under the owning table's lock, allow just one unpublished successor to
    /// an active route. Ordinary duplicates and a second reservation conflict.
    pub(crate) fn conflicts_with(&self, existing: &Self) -> bool {
        self.0.load(Ordering::Acquire) != STAGED || !existing.is_active()
    }

    /// Call only after every reserved table confirms the predecessor is gone.
    /// Reservations already charge capacity and prevent competing publication.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn activate(&self) -> bool {
        self.0
            .compare_exchange(STAGED, ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            || self.is_active()
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn retire(&self) {
        self.0.store(RETIRED, Ordering::Release);
    }
}
