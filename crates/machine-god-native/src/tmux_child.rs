//! Exact tmux child ownership using the existing bounded reap capacity.

use super::{
    BackgroundProcessError, CHILD_REAP_PROBE_TIMEOUT, ChildReapPermit, ChildTryWaitError,
    ObservationBackoff, cleanup_error, discharge_reaped_child, invariant_error,
    quarantine_owned_child, reserve_child_reap_authority, spawn_error, try_wait_child, wait_error,
};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub(crate) struct TmuxChild {
    child: Option<Child>,
    permit: Option<ChildReapPermit>,
    status: Option<ExitStatus>,
    cleanup_deadline: Option<Instant>,
    #[cfg(test)]
    id: u32,
}

impl TmuxChild {
    pub(crate) fn spawn(command: &mut Command) -> Result<Self, BackgroundProcessError> {
        // Admission precedes the only requested spawn. No auxiliary process or
        // per-child worker is needed to reserve eventual cleanup ownership.
        let mut permit = reserve_child_reap_authority()?;
        permit.kill_pending = true;
        let child = command.spawn().map_err(|_| spawn_error())?;
        Ok(Self {
            #[cfg(test)]
            id: child.id(),
            child: Some(child),
            permit: Some(permit),
            status: None,
            cleanup_deadline: None,
        })
    }

    pub(crate) fn take_pipes(
        &mut self,
    ) -> (Option<ChildStdin>, Option<ChildStdout>, Option<ChildStderr>) {
        self.child.as_mut().map_or((None, None, None), |child| {
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        })
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, BackgroundProcessError> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let child = self.child.as_mut().ok_or_else(wait_error)?;
        #[cfg(test)]
        if self
            .permit
            .as_ref()
            .and_then(|permit| permit.deferred_reap.as_ref())
            .is_some_and(|deferred| deferred.load(Ordering::Acquire))
        {
            return Ok(None);
        }
        match try_wait_child(child) {
            Ok(Some(status)) => {
                self.status = Some(status);
                discharge_reaped_child(&mut self.child, &mut self.permit);
                Ok(Some(status))
            }
            Ok(None) => Ok(None),
            Err(ChildTryWaitError::LostAuthority) => {
                // No later retry or Drop may turn this stale numeric PID into
                // signal authority. A missing success receipt stays an error.
                discharge_reaped_child(&mut self.child, &mut self.permit);
                Err(wait_error())
            }
            // An interrupted observation has not established that the child
            // is running. Preserve ownership without admitting a kill; the
            // same owner can retry under its original cleanup deadline.
            Err(ChildTryWaitError::Interrupted | ChildTryWaitError::Operation) => Err(wait_error()),
        }
    }

    pub(crate) fn abort(&mut self) -> Result<(), BackgroundProcessError> {
        // Nested owner Drops may retry the public cleanup path. One child has
        // one grace window; subsequent explicit retries can still observe its
        // exit or retry a pending kill, but cannot refresh the wait budget.
        let deadline = *self
            .cleanup_deadline
            .get_or_insert_with(|| Instant::now() + CHILD_REAP_PROBE_TIMEOUT);
        self.abort_until(deadline)
    }

    pub(crate) fn retain_until_reaped(&mut self, keepalive: Box<dyn Send>) {
        if let Some(permit) = self.permit.as_mut() {
            debug_assert!(permit.keepalive.is_none());
            permit.keepalive = Some(keepalive);
        }
    }

    fn abort_until(&mut self, deadline: Instant) -> Result<(), BackgroundProcessError> {
        // Callers close their taken descriptors first. This also covers failure
        // before the caller has acquired every requested pipe.
        drop(self.take_pipes());
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        let child = self.child.as_mut().ok_or_else(wait_error)?;
        // Kill failure is not proof of death; still observe the exact child so
        // a concurrent natural exit can discharge its retained authority.
        let permit = self.permit.as_mut().ok_or_else(invariant_error)?;
        if permit.kill_pending && child.kill().is_ok() {
            permit.kill_pending = false;
        }
        let mut observation = ObservationBackoff::retry();
        loop {
            if self.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(cleanup_error());
            }
            observation.sleep_until_and_advance(deadline);
        }
    }

    #[cfg(test)]
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    #[cfg(test)]
    pub(crate) fn defer_reap_for_test(&mut self, deferred: Arc<AtomicBool>) {
        self.permit.as_mut().unwrap().deferred_reap = Some(deferred);
    }

    #[cfg(test)]
    pub(crate) fn is_owned_for_test(&self) -> bool {
        self.child.is_some() && self.permit.is_some()
    }
}

impl Drop for TmuxChild {
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        // An expired explicit abort has already consumed its grace. Drop may
        // observe once more, but must not start a second 500 ms wait window.
        let deadline = self
            .cleanup_deadline
            .unwrap_or_else(|| Instant::now() + CHILD_REAP_PROBE_TIMEOUT);
        let _ = self.abort_until(deadline);
        if self.child.is_some() {
            let _ = quarantine_owned_child(&mut self.child, &mut self.permit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        GROUP_SNAPSHOT_TEST_LOCK, TRY_WAIT_ERRNO, TRY_WAIT_FAILURE_PID, TRY_WAIT_FAILURES,
        inject_failures, reap_quarantined_direct,
    };
    use super::*;
    use std::num::NonZeroU32;
    use std::os::unix::process::ExitStatusExt;
    use std::thread;
    use std::time::Duration;

    fn exited_child() -> TmuxChild {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 7"]).env_clear();
        TmuxChild::spawn(&mut command).unwrap()
    }

    #[test]
    fn positive_exit_receipt_survives_owned_handle_discharge() {
        let mut child = exited_child();
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(status.code(), Some(7));
        assert!(!child.is_owned_for_test());
        assert_eq!(child.try_wait().unwrap(), Some(status));
        child.abort().unwrap();
        assert_eq!(child.try_wait().unwrap(), Some(status));
    }

    #[test]
    fn known_external_reap_irrevocably_revokes_signal_authority() {
        let mut child = exited_child();
        let pid = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG)
                .unwrap()
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(child.try_wait().is_err());
        assert!(!child.is_owned_for_test());
        assert!(child.abort().is_err());
        assert!(child.try_wait().is_err());
        assert!(child.child.is_none());
        assert!(child.permit.is_none());
    }

    #[test]
    fn repeated_failed_aborts_preserve_the_exact_original_cleanup_deadline() {
        struct ReleaseOnDrop(Arc<AtomicBool>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let mut child = exited_child();
        let deferred = ReleaseOnDrop(Arc::new(AtomicBool::new(true)));
        child.defer_reap_for_test(Arc::clone(&deferred.0));
        assert!(child.abort().is_err());
        let original = child.cleanup_deadline.unwrap();
        assert!(child.abort().is_err());
        assert_eq!(child.cleanup_deadline, Some(original));
        assert!(child.is_owned_for_test());
        drop(deferred);
        child.abort().unwrap();
        assert_eq!(child.cleanup_deadline, Some(original));
        assert!(!child.is_owned_for_test());
    }

    #[test]
    fn interrupted_abort_observation_retains_child_without_attempting_kill() {
        let _guard = GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut command = Command::new("/bin/sleep");
        command.arg("30").env_clear();
        let mut child = TmuxChild::spawn(&mut command).unwrap();
        inject_failures(
            &TRY_WAIT_FAILURE_PID,
            &TRY_WAIT_FAILURES,
            NonZeroU32::new(child.id()).unwrap(),
            1,
        );
        TRY_WAIT_ERRNO.store(libc::EINTR, Ordering::Release);
        let interrupted = child.abort();
        TRY_WAIT_FAILURE_PID.store(0, Ordering::Release);
        TRY_WAIT_FAILURES.store(0, Ordering::Release);
        TRY_WAIT_ERRNO.store(0, Ordering::Release);
        assert!(interrupted.is_err());
        assert!(child.is_owned_for_test());
        assert!(child.permit.as_ref().unwrap().kill_pending);
        assert!(child.status.is_none());
        assert!(matches!(
            try_wait_child(child.child.as_mut().unwrap()),
            Ok(None)
        ));
        child.abort().unwrap();
        assert!(!child.is_owned_for_test());
        assert_eq!(child.status.unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn quarantine_retries_pending_tmux_kill_only_after_observation_recovers() {
        let _guard = GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut command = Command::new("/bin/sleep");
        command.arg("30").env_clear();
        let mut child = TmuxChild::spawn(&mut command).unwrap();
        inject_failures(
            &TRY_WAIT_FAILURE_PID,
            &TRY_WAIT_FAILURES,
            NonZeroU32::new(child.id()).unwrap(),
            1,
        );
        TRY_WAIT_ERRNO.store(libc::EIO, Ordering::Release);
        let failed_observation = reap_quarantined_direct(
            child.child.as_mut().unwrap(),
            child.permit.as_mut().unwrap(),
        );
        TRY_WAIT_FAILURE_PID.store(0, Ordering::Release);
        TRY_WAIT_FAILURES.store(0, Ordering::Release);
        TRY_WAIT_ERRNO.store(0, Ordering::Release);
        assert!(!failed_observation);
        assert!(child.permit.as_ref().unwrap().kill_pending);
        assert!(!reap_quarantined_direct(
            child.child.as_mut().unwrap(),
            child.permit.as_mut().unwrap(),
        ));
        assert!(!child.permit.as_ref().unwrap().kill_pending);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(status.signal(), Some(libc::SIGKILL));
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!child.is_owned_for_test());
    }
}
