//! Fixed, single-refill Darwin directory binding (ADR 0005).

use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd},
};

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("directory enumeration requires a separately verified Darwin ABI");

/// Fixed initialized scratch size for one metered directory refill.
pub const DIRECTORY_READ_BUFFER_BYTES: usize = 8192;

// SAFETY: ADR 0005 verifies this fixed exported libSystem declaration against
// Apple Libc's telldir.h and both supported 64-bit SDK/link ABIs. There is no
// generic syscall number, dynamic symbol lookup or pointer-bearing public API.
#[allow(unsafe_code)]
unsafe extern "C" {
    fn __getdirentries64(
        fd: libc::c_int,
        buffer: *mut libc::c_void,
        length: libc::size_t,
        base: *mut libc::off_t,
    ) -> libc::size_t;
}

/// Performs exactly one directory-refill call on the supplied descriptor.
///
/// No allocation, reopen, close or retry occurs. The caller owns and must
/// serialize the shared directory cursor. Zero means EOF; only the returned
/// initialized prefix contains records, not any trailing kernel status flags.
/// A kernel/filesystem call can block; this is not a wall-clock deadline API.
///
/// # Errors
/// Preserves the original OS error, including `EINTR`, or returns `InvalidData`
/// if the returned byte count exceeds the fixed buffer. Ignore buffer contents
/// on every error; a later retry is a separate native call.
#[allow(unsafe_code)] // ADR 0005: one fixed synchronous read-only export.
pub fn read_directory_chunk(
    fd: BorrowedFd<'_>,
    buffer: &mut [u8; DIRECTORY_READ_BUFFER_BYTES],
) -> io::Result<usize> {
    checked_read(buffer, |buffer| {
        let mut base: libc::off_t = 0;
        // SAFETY: BorrowedFd keeps the descriptor valid for this call. The
        // entire fixed buffer is initialized and exclusively writable, and
        // base is initialized/aligned writable off_t storage. The synchronous
        // kernel export retains neither pointer and receives the exact buffer
        // size. No Rust structure is fabricated from its returned bytes.
        let result = unsafe {
            __getdirentries64(
                fd.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut base,
            )
        };
        if result == libc::size_t::MAX {
            Err(io::Error::last_os_error())
        } else {
            Ok(result)
        }
    })
}

fn checked_read(
    buffer: &mut [u8; DIRECTORY_READ_BUFFER_BYTES],
    read: impl FnOnce(&mut [u8; DIRECTORY_READ_BUFFER_BYTES]) -> io::Result<usize>,
) -> io::Result<usize> {
    let count = read(buffer)?;
    if count > buffer.len() {
        Err(io::Error::from(io::ErrorKind::InvalidData))
    } else {
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, os::fd::AsFd};

    #[test]
    fn exact_buffer_bounds_and_interrupted_errors_make_one_attempt() {
        let mut buffer = [0; DIRECTORY_READ_BUFFER_BYTES];
        for count in [0, 1, DIRECTORY_READ_BUFFER_BYTES] {
            let mut calls = 0;
            assert_eq!(
                checked_read(&mut buffer, |storage| {
                    calls += 1;
                    assert_eq!(storage.len(), 8192);
                    Ok(count)
                })
                .unwrap(),
                count
            );
            assert_eq!(calls, 1);
        }
        assert_eq!(
            checked_read(&mut buffer, |_| Ok(8193)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        for errno in [libc::EINTR, libc::EBADF, libc::EIO] {
            let mut calls = 0;
            let error = checked_read(&mut buffer, |_| {
                calls += 1;
                Err(io::Error::from_raw_os_error(errno))
            })
            .unwrap_err();
            assert_eq!(calls, 1);
            assert_eq!(error.raw_os_error(), Some(errno));
        }
    }

    #[test]
    fn real_export_rejects_non_directory_without_consuming_descriptor() {
        let file = File::open("/dev/null").unwrap();
        let mut buffer = [0; DIRECTORY_READ_BUFFER_BYTES];
        let error = read_directory_chunk(file.as_fd(), &mut buffer).unwrap_err();
        assert!(matches!(
            error.raw_os_error(),
            Some(libc::EINVAL | libc::ENOTDIR)
        ));
        assert!(file.metadata().is_ok());
    }

    #[test]
    fn libc_layout_matches_the_verified_sdk_fields() {
        assert_eq!(size_of::<libc::off_t>(), 8);
        assert_eq!(size_of::<libc::size_t>(), 8);
        assert_eq!(size_of::<libc::dirent>(), 1048);
        assert_eq!(std::mem::offset_of!(libc::dirent, d_ino), 0);
        assert_eq!(std::mem::offset_of!(libc::dirent, d_reclen), 16);
        assert_eq!(std::mem::offset_of!(libc::dirent, d_namlen), 18);
        assert_eq!(std::mem::offset_of!(libc::dirent, d_type), 20);
        assert_eq!(std::mem::offset_of!(libc::dirent, d_name), 21);
    }
}
