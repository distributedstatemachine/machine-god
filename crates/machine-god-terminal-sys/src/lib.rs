//! Narrow OS binding unavailable through the pinned safe dependencies.
//!
//! See docs/adr/0001-macos-terminal-foreground-signal.md in the repository.
//! All orchestration, process ownership and permission policy stay in native.

#[cfg(target_os = "macos")]
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd},
};

#[cfg(target_os = "macos")]
use rustix::process::Signal;

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
#[allow(unsafe_code)] // ADR 0001: this function is the entire unsafe exception.
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
}
