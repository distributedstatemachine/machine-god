//! Bounded safe access to the small Darwin process-query surface used by
//! machine-god.
//!
//! The API never sizes an allocation from a kernel-reported process count.
//! Callers provide the complete PID buffer and receive an explicit truncation
//! error when Darwin fills it.

#![deny(unsafe_code)]

use std::fmt;

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "the audited Darwin ABI boundary is isolated to this one module"
)]
mod ffi;

/// A coherent Darwin process identity and lineage observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessInfo {
    /// Process identifier returned by `PROC_PIDTBSDINFO`.
    pub pid: i32,
    /// Current parent process identifier.
    pub parent_pid: i32,
    /// Current process-group identifier.
    pub process_group_id: i32,
    /// Kernel process-incarnation identifier from flavor 17.
    pub unique_id: u64,
    /// Kernel identity of the process parent observed with `unique_id`.
    pub parent_unique_id: u64,
}

/// Stable failure categories for bounded Darwin process queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The current target is not macOS.
    Unsupported,
    /// A PID must be positive.
    InvalidPid,
    /// PID enumeration needs at least one caller-owned slot.
    BufferTooSmall,
    /// Darwin filled the complete caller-owned PID buffer.
    Truncated,
    /// The process did not exist when queried.
    NotFound,
    /// The process exists but the caller cannot inspect it.
    PermissionDenied,
    /// The two identity reads around the BSD metadata read disagreed.
    InconsistentSnapshot,
    /// Darwin returned a count or record inconsistent with its ABI.
    UnexpectedResult,
    /// Darwin reported another operating-system error.
    System,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unsupported => "Darwin process queries are unsupported on this target",
            Self::InvalidPid => "the process identifier is invalid",
            Self::BufferTooSmall => "the PID buffer has no slots",
            Self::Truncated => "the bounded PID snapshot was truncated",
            Self::NotFound => "the process was not found",
            Self::PermissionDenied => "permission to inspect the process was denied",
            Self::InconsistentSnapshot => "the process changed while it was inspected",
            Self::UnexpectedResult => "Darwin returned an unexpected process-query result",
            Self::System => "Darwin reported a process-query error",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for Error {}

/// Lists all process identifiers into one fixed caller-owned buffer.
///
/// A successful slice borrows `buffer` and contains exactly the initialized
/// prefix reported by Darwin. If Darwin fills every available byte, this
/// returns [`Error::Truncated`] conservatively: callers that admit at most `N`
/// processes should pass `N + 1` slots and reject a returned slice longer than
/// `N`.
///
/// # Errors
///
/// Returns a fixed input, truncation, ABI, or operating-system failure. On
/// non-macOS targets it returns [`Error::Unsupported`].
pub fn list_all_pids(buffer: &mut [i32]) -> Result<&[i32], Error> {
    if buffer.is_empty() {
        return Err(Error::BufferTooSmall);
    }
    #[cfg(target_os = "macos")]
    {
        let length = ffi::list_all_pids(buffer)?;
        Ok(&buffer[..length])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = buffer;
        Err(Error::Unsupported)
    }
}

/// Lists the current direct children of `parent_pid` into one fixed buffer.
///
/// The successful slice and conservative full-buffer truncation rule are the
/// same as [`list_all_pids`]. No kernel-reported count controls allocation.
///
/// # Errors
///
/// Returns a fixed input, truncation, ABI, or operating-system failure. On
/// non-macOS targets it returns [`Error::Unsupported`].
pub fn list_child_pids(parent_pid: i32, buffer: &mut [i32]) -> Result<&[i32], Error> {
    if parent_pid <= 0 {
        return Err(Error::InvalidPid);
    }
    if buffer.is_empty() {
        return Err(Error::BufferTooSmall);
    }
    #[cfg(target_os = "macos")]
    {
        let length = ffi::list_child_pids(parent_pid, buffer)?;
        Ok(&buffer[..length])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (parent_pid, buffer);
        Err(Error::Unsupported)
    }
}

/// Reads one coherent identity, parent, and process-group observation.
///
/// The wrapper reads flavor 17 both before and after `PROC_PIDTBSDINFO`; it
/// returns [`Error::InconsistentSnapshot`] instead of combining metadata from
/// two different process incarnations or flavor-17 parent identities.
///
/// # Errors
///
/// Returns a fixed input, process, consistency, ABI, or operating-system
/// failure. On non-macOS targets it returns [`Error::Unsupported`].
pub fn process_info(pid: i32) -> Result<ProcessInfo, Error> {
    if pid <= 0 {
        return Err(Error::InvalidPid);
    }
    #[cfg(target_os = "macos")]
    {
        ffi::process_info(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, list_all_pids, list_child_pids, process_info};

    #[test]
    fn error_taxonomy_and_rendering_are_data_free() {
        let expected = [
            (Error::Unsupported, "Unsupported"),
            (Error::InvalidPid, "InvalidPid"),
            (Error::BufferTooSmall, "BufferTooSmall"),
            (Error::Truncated, "Truncated"),
            (Error::NotFound, "NotFound"),
            (Error::PermissionDenied, "PermissionDenied"),
            (Error::InconsistentSnapshot, "InconsistentSnapshot"),
            (Error::UnexpectedResult, "UnexpectedResult"),
            (Error::System, "System"),
        ];

        for (error, expected_debug) in expected {
            assert_eq!(format!("{error:?}"), expected_debug);
            assert!(
                !error
                    .to_string()
                    .chars()
                    .any(|character| character.is_ascii_digit())
            );
        }
    }

    #[test]
    fn rejects_empty_pid_buffer_without_calling_the_platform() {
        assert_eq!(list_all_pids(&mut []), Err(Error::BufferTooSmall));
    }

    #[test]
    fn rejects_non_positive_process_identifiers() {
        assert_eq!(process_info(0), Err(Error::InvalidPid));
        assert_eq!(process_info(-1), Err(Error::InvalidPid));
        let mut buffer = [0_i32; 1];
        assert_eq!(list_child_pids(0, &mut buffer), Err(Error::InvalidPid));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_targets_are_inert() {
        let mut buffer = [0_i32; 2];
        assert_eq!(list_all_pids(&mut buffer), Err(Error::Unsupported));
        assert_eq!(list_child_pids(1, &mut buffer), Err(Error::Unsupported));
        assert_eq!(process_info(1), Err(Error::Unsupported));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lists_current_process_in_a_fixed_buffer() {
        let mut buffer = [0_i32; 4_096];
        let own_pid = i32::try_from(std::process::id()).expect("own PID fits i32");
        match list_all_pids(&mut buffer) {
            Ok(pids) => assert!(pids.contains(&own_pid)),
            Err(Error::Truncated) => {}
            other => panic!("unexpected bounded PID result: {other:?}"),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reports_a_full_tiny_buffer_as_truncated() {
        let mut buffer = [0_i32; 1];
        assert_eq!(list_all_pids(&mut buffer), Err(Error::Truncated));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_current_process_identity_and_lineage() {
        let own_pid = i32::try_from(std::process::id()).expect("own PID fits i32");
        let info = process_info(own_pid).expect("current process info");
        assert_eq!(info.pid, own_pid);
        assert!(info.parent_pid >= 0);
        assert!(info.process_group_id > 0);
        assert_ne!(info.unique_id, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lists_a_live_direct_child_without_kernel_sized_allocation() {
        use std::process::{Child, Command, Stdio};

        struct ReapedChild(Child);

        impl Drop for ReapedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let child = Command::new("/bin/sleep")
            .arg("10")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        let child = ReapedChild(child);
        let own_pid = i32::try_from(std::process::id()).expect("own PID fits i32");
        let child_pid = i32::try_from(child.0.id()).expect("child PID fits i32");
        let mut buffer = [0_i32; 32];
        let children = list_child_pids(own_pid, &mut buffer).expect("bounded direct-child list");
        assert!(children.contains(&child_pid));
    }
}
