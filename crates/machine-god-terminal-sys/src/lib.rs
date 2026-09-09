//! Narrow OS binding unavailable through the pinned safe dependencies.
//!
//! See docs/decisions/0003-macos-terminal-foreground-signal.md in the repository.
//! Read-only process enumeration is separately scoped by ADR 0004.
//! Metered descriptor-backed directory refills are scoped by ADR 0005.
//! All orchestration, process ownership and permission policy stay in native.

#[cfg(target_os = "macos")]
mod macos_directory;
#[cfg(target_os = "macos")]
pub use macos_directory::{DIRECTORY_READ_BUFFER_BYTES, read_directory_chunk};

#[cfg(target_os = "macos")]
mod process_identity;
#[cfg(target_os = "macos")]
pub use process_identity::ProcessIdentity;

#[cfg(target_os = "macos")]
mod process_inventory;
#[cfg(target_os = "macos")]
pub use process_inventory::process_ids;

#[cfg(target_os = "macos")]
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd},
};

#[cfg(target_os = "macos")]
use rustix::process::Signal;

/// Reads the boot-local clock used by Rust's macOS `Instant` implementation.
///
/// Unlike `CLOCK_MONOTONIC`, this clock excludes system sleep and uses the same
/// raw timebase as `Instant`, permitting conservative cross-process deadlines.
///
/// # Errors
/// Returns the clock query error or rejects an invalid kernel timestamp.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // ADR 0003: fixed read-only clock, no arbitrary clock API.
pub fn uptime_raw() -> io::Result<std::time::Duration> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the fixed supported clock ID accepts a writable timespec pointer.
    // `time` is initialized, aligned and exclusively borrowed for this call;
    // clock_gettime writes synchronously and does not retain its address.
    if unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &raw mut time) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let seconds =
        u64::try_from(time.tv_sec).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let nanos =
        u32::try_from(time.tv_nsec).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    if nanos >= 1_000_000_000 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(std::time::Duration::new(seconds, nanos))
}

/// Signals the foreground process group of the supplied macOS PTY master.
///
/// The kernel selects and references the group under its tty lock. No numeric
/// process/group lookup is used. The caller must retain its owned PTY master
/// and authorize the effect; success is delivery, not proof of process exit or
/// cleanup of other job-control groups. TIOCSIG may flush pending tty data.
/// There is deliberately no retry: another call could target a new foreground
/// job after the first call was interrupted.
///
/// # Errors
/// Returns the OS error for an invalid/non-master descriptor or failed ioctl.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // ADR 0003: fixed PTY ioctl, not an arbitrary request API.
pub fn signal_terminal_foreground(master: BorrowedFd<'_>, signal: Signal) -> io::Result<()> {
    let argument = libc::uintptr_t::try_from(signal.as_raw())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: BorrowedFd keeps the descriptor valid for this call. The fixed
    // macOS TIOCSIG command is IOC_VOID: its third argument is the signal value,
    // NOT a pointer to an int. Use a pointer-width scalar for the varargs ABI.
    // XNU copies that scalar into its own buffer and never dereferences user
    // memory for this command. No Rust pointer, buffer or ownership is exposed.
    // The kernel rejects inappropriate descriptors/signals. The only effect
    // permitted by this fixed request is signaling the PTY foreground group
    // (and tty flushing); there is no arbitrary-ioctl interface.
    let result = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSIG.into(), argument) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::{fs::File, os::fd::AsFd};

    #[test]
    fn uptime_raw_matches_instant_elapsed_interval() {
        let before = std::time::Instant::now();
        let raw_before = uptime_raw().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let raw_after = uptime_raw().unwrap();
        let after = std::time::Instant::now();
        let elapsed = raw_after.checked_sub(raw_before).unwrap();
        assert!(!elapsed.is_zero());
        assert!(elapsed <= after.duration_since(before));
    }

    #[test]
    fn rejects_non_terminal_without_consuming_descriptor() {
        let file = File::open("/dev/null").unwrap();
        for signal in [
            Signal::HUP,
            Signal::INT,
            Signal::QUIT,
            Signal::TERM,
            Signal::KILL,
        ] {
            let error = signal_terminal_foreground(file.as_fd(), signal).unwrap_err();
            assert!(matches!(
                error.raw_os_error(),
                Some(libc::ENOTTY | libc::ENODEV)
            ));
            assert!(file.metadata().is_ok());
        }
    }

    #[test]
    fn scalar_signal_abi_accepts_a_master_but_rejects_its_slave() {
        use rustix::fs::{Mode, OFlags, open};
        let flags = OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let master = open("/dev/ptmx", flags, Mode::empty()).unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let slave = open(name.as_c_str(), flags | OFlags::NOFOLLOW, Mode::empty()).unwrap();
        // No child/session has attached to this PTY. This exercises the actual
        // kernel ABI without directing signals at the test runner or its group.
        for signal in [
            Signal::HUP,
            Signal::INT,
            Signal::QUIT,
            Signal::TERM,
            Signal::KILL,
        ] {
            signal_terminal_foreground(master.as_fd(), signal).unwrap();
            let error = signal_terminal_foreground(slave.as_fd(), signal).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::ENOTTY));
        }
    }
}
