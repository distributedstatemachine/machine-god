//! macOS process-incarnation queries and audit-token signal dispatch (ADR 0003).

use std::{fmt, io, num::NonZeroU32};

use rustix::process::Signal;

// Apple proc_info_private.h marks this 56-byte structure as API. Keep the exact
// layout; never infer identity from timestamps, executable UUIDs or plain PIDs.
#[repr(C)]
#[derive(Default)]
struct UniqueIdentifierInfo {
    executable_uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    pid_version: i32,
    original_parent_pid_version: i32,
    reserved: [u64; 2],
}
const _: () = assert!(size_of::<UniqueIdentifierInfo>() == 56);
const _: () = assert!(std::mem::offset_of!(UniqueIdentifierInfo, unique_id) == 16);
const _: () = assert!(std::mem::offset_of!(UniqueIdentifierInfo, pid_version) == 32);

#[repr(C)]
struct AuditToken {
    values: [u32; 8],
}
const _: () = assert!(size_of::<AuditToken>() == 32);

// SAFETY: Matches Apple's longstanding __proc_info syscall-wrapper declaration
// (libproc.c). The only invocation below fixes the audit-token operation and
// passes a live, correctly sized token. Using this wrapper instead of importing
// the newer proc_signal_with_audittoken symbol keeps older systems loadable;
// an unsupported kernel operation returns an error, not numeric-PID fallback.
#[allow(unsafe_code)]
unsafe extern "C" {
    fn __proc_info(
        call: libc::c_int,
        pid: libc::c_int,
        flavor: libc::c_int,
        argument: u64,
        buffer: *mut libc::c_void,
        buffer_size: libc::c_int,
    ) -> libc::c_int;
}

/// An observed macOS process incarnation, not a permission or ownership grant.
///
/// Native must separately prove membership in its owned process/session before
/// retaining this identity for control. No serialized identity can grant that
/// authority. PID reuse never transfers this identity to the replacement.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ProcessIdentity {
    pid: NonZeroU32,
    unique_id: u64,
    pid_version: i32,
}

impl fmt::Debug for ProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessIdentity")
            .finish_non_exhaustive()
    }
}

impl ProcessIdentity {
    /// Reads one live process incarnation without signaling it.
    ///
    /// # Errors
    /// Returns the OS error for missing/inaccessible processes and rejects
    /// malformed identities, unsupported ABI or out-of-range PIDs.
    #[allow(unsafe_code)] // ADR 0003: fixed-size, initialized identity query.
    pub fn capture(pid: NonZeroU32) -> io::Result<Self> {
        let raw =
            i32::try_from(pid.get()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let mut info = UniqueIdentifierInfo::default();
        // SAFETY: info is initialized, writable, correctly aligned repr(C)
        // storage of the exact asserted ABI size. Flavor 17 only copies this
        // record synchronously; it cannot retain the pointer or write beyond
        // the supplied 56-byte bound. All returned fields have integer types.
        let count =
            unsafe { libc::proc_pidinfo(raw, 17, 0, std::ptr::from_mut(&mut info).cast(), 56) };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        if count != 56 || info.unique_id == 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(Self {
            pid,
            unique_id: info.unique_id,
            pid_version: info.pid_version,
        })
    }

    /// Numeric display identity, never sufficient for later signaling.
    #[must_use]
    pub const fn pid(self) -> NonZeroU32 {
        self.pid
    }

    /// Boot-local process-incarnation identifier for native retention/comparison.
    #[must_use]
    pub const fn unique_id(self) -> u64 {
        self.unique_id
    }

    /// Compares process incarnations, allowing an exec within the same process.
    #[must_use]
    pub const fn same_process(self, other: Self) -> bool {
        self.pid.get() == other.pid.get() && self.unique_id == other.unique_id
    }

    /// Whether this exact process still exists (exec does not change ownership).
    ///
    /// # Errors
    /// Returns observation failures rather than treating them as process exit.
    pub fn exists(self) -> io::Result<bool> {
        match Self::capture(self.pid) {
            Ok(current) => Ok(self.same_process(current)),
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Signals this incarnation, refreshing its exec version but never its identity.
    ///
    /// The kernel validates PID/version and retains the exact process reference
    /// while delivering the signal. A concurrent exec may cause ESRCH; callers
    /// may re-observe within their bounded cleanup loop. Success is not exit.
    ///
    /// # Errors
    /// Returns ESRCH for a vanished/replaced incarnation and other OS failures
    /// without falling back to kill(PID). Authorization remains the caller's job.
    pub fn signal(self, signal: Signal) -> io::Result<()> {
        let current = Self::capture(self.pid)?;
        if !self.same_process(current) {
            return Err(io::Error::from_raw_os_error(libc::ESRCH));
        }
        current.signal_version(signal)
    }

    fn signal_version(self, signal: Signal) -> io::Result<()> {
        let mut token = AuditToken { values: [0; 8] };
        // XNU proc_find_audit_token uses PID (slot 5) and pidversion (slot 7).
        // Credential slots are NOT grants: the kernel checks the caller's real
        // credentials with cansignal independently of these target selectors.
        token.values[5] = self.pid.get();
        token.values[7] = u32::from_ne_bytes(self.pid_version.to_ne_bytes());
        Self::signal_token(token, signal)
    }

    /// Checks kernel support without targeting any process or delivering a signal.
    ///
    /// Native calls this before starting a session that requires exact cleanup.
    ///
    /// # Errors
    /// Rejects unsupported kernels or unexpected responses before process effects.
    pub fn verify_signal_support() -> io::Result<()> {
        // PID zero is an invalid audit-token target, NOT kill(0)'s process group.
        match Self::signal_token(AuditToken { values: [0; 8] }, Signal::KILL) {
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
            Err(error) => Err(error),
            Ok(()) => Err(io::Error::from(io::ErrorKind::InvalidData)),
        }
    }

    #[allow(unsafe_code)] // ADR 0003: only the fixed audit-token signal operation.
    fn signal_token(mut token: AuditToken, signal: Signal) -> io::Result<()> {
        // SAFETY: Fixed operation 0x11 reads exactly one initialized repr(C)
        // 32-byte audit token synchronously. The pointer is valid for the call,
        // never retained, and there are no aliased Rust references to it. The
        // signal comes from rustix's typed Signal. PID is positive/in-range or
        // deliberately zero for the non-signaling feature-support probe.
        let result = unsafe {
            __proc_info(
                0x11,
                0,
                signal.as_raw(),
                0,
                std::ptr::from_mut(&mut token).cast(),
                32,
            )
        };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Child, Command, Stdio},
    };

    struct OwnedTestChild(Child);
    impl Drop for OwnedTestChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn stale_version_cannot_signal_a_live_replacement_and_exact_version_can() {
        let mut child = OwnedTestChild(
            Command::new("/bin/sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let identity = ProcessIdentity::capture(NonZeroU32::new(child.0.id()).unwrap()).unwrap();
        let mut stale = identity;
        stale.pid_version = stale.pid_version.wrapping_add(1);
        assert_eq!(
            stale
                .signal_version(Signal::KILL)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::ESRCH)
        );
        assert!(child.0.try_wait().unwrap().is_none());
        let mut reused = identity;
        reused.unique_id = reused.unique_id.wrapping_add(1);
        assert!(!reused.exists().unwrap());
        assert_eq!(
            reused.signal(Signal::KILL).unwrap_err().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert!(identity.exists().unwrap());
        identity.signal(Signal::KILL).unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(!identity.exists().unwrap());
        assert_eq!(format!("{identity:?}"), "ProcessIdentity { .. }");
    }

    #[test]
    fn query_rejects_out_of_range_pid_before_native_effect() {
        ProcessIdentity::verify_signal_support().unwrap();
        assert_eq!(
            ProcessIdentity::capture(NonZeroU32::new(u32::MAX).unwrap())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
