//! The complete unsafe boundary to Darwin's stable libproc ABI.

use std::ffi::{c_int, c_void};
use std::mem::{MaybeUninit, size_of};

use crate::{Error, ProcessInfo};

const PROC_PIDTBSDINFO: c_int = 3;
// Stable XNU process-identity flavor. Apple's public SDK header omits the
// constant, while XNU and the pinned fx implementation define flavor 17 with
// this fixed record layout.
const PROC_PIDUNIQIDENTIFIERINFO: c_int = 17;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
#[allow(
    clippy::struct_field_names,
    reason = "field names intentionally match the audited XNU ABI record"
)]
struct ProcUniqueIdentifierInfo {
    p_uuid: [u8; 16],
    p_uniqueid: u64,
    p_puniqueid: u64,
    p_idversion: i32,
    p_orig_ppidversion: i32,
    p_reserve2: u64,
    p_reserve3: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: u32,
    pbi_gid: u32,
    pbi_ruid: u32,
    pbi_rgid: u32,
    pbi_svuid: u32,
    pbi_svgid: u32,
    rfu_1: u32,
    pbi_comm: [u8; 16],
    pbi_name: [u8; 32],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

/// Marker for fixed C records in which every bit pattern is valid.
///
/// # Safety
///
/// Implementors must be `repr(C)`, contain no references or invalid scalar
/// niches, and match the record layout for their declared libproc flavor.
unsafe trait DarwinRecord: Copy {
    const FLAVOR: c_int;
}

// SAFETY: the repr(C) flavor-17 record contains only integer byte sequences,
// and the layout is asserted below against the pinned XNU ABI.
unsafe impl DarwinRecord for ProcUniqueIdentifierInfo {
    const FLAVOR: c_int = PROC_PIDUNIQIDENTIFIERINFO;
}

// SAFETY: the repr(C) BSD record contains only integer byte sequences, and the
// layout is asserted below against the public XNU flavor-3 ABI.
unsafe impl DarwinRecord for ProcBsdInfo {
    const FLAVOR: c_int = PROC_PIDTBSDINFO;
}

pub(super) fn list_all_pids(buffer: &mut [i32]) -> Result<usize, Error> {
    let byte_capacity = pid_buffer_byte_capacity(buffer)?;

    clear_errno();
    // SAFETY: `buffer` is live and uniquely borrowed for `byte_capacity`
    // bytes, its element type is Darwin's `pid_t`, and libproc initializes at
    // most the supplied byte count.
    let returned =
        unsafe { libc::proc_listallpids(buffer.as_mut_ptr().cast::<c_void>(), byte_capacity) };
    pid_count(returned, current_errno(), buffer.len())
}

pub(super) fn list_child_pids(parent_pid: i32, buffer: &mut [i32]) -> Result<usize, Error> {
    let byte_capacity = pid_buffer_byte_capacity(buffer)?;
    clear_errno();
    // SAFETY: `buffer` is live and uniquely borrowed for `byte_capacity`
    // bytes, its element type is Darwin's `pid_t`, and libproc initializes at
    // most the supplied byte count. `parent_pid` was validated by the safe
    // entry point.
    let returned = unsafe {
        libc::proc_listchildpids(
            parent_pid,
            buffer.as_mut_ptr().cast::<c_void>(),
            byte_capacity,
        )
    };
    pid_count(returned, current_errno(), buffer.len())
}

pub(super) fn process_info(pid: i32) -> Result<ProcessInfo, Error> {
    let first_identity = query_pid_info::<ProcUniqueIdentifierInfo>(pid)?;
    let bsd = query_pid_info::<ProcBsdInfo>(pid)?;
    let second_identity = query_pid_info::<ProcUniqueIdentifierInfo>(pid)?;
    if first_identity.p_uniqueid != second_identity.p_uniqueid
        || first_identity.p_puniqueid != second_identity.p_puniqueid
    {
        return Err(Error::InconsistentSnapshot);
    }

    let observed_pid = i32::try_from(bsd.pbi_pid).map_err(|_| Error::UnexpectedResult)?;
    if observed_pid != pid {
        return Err(Error::InconsistentSnapshot);
    }
    Ok(ProcessInfo {
        pid: observed_pid,
        parent_pid: i32::try_from(bsd.pbi_ppid).map_err(|_| Error::UnexpectedResult)?,
        process_group_id: i32::try_from(bsd.pbi_pgid).map_err(|_| Error::UnexpectedResult)?,
        unique_id: first_identity.p_uniqueid,
        parent_unique_id: first_identity.p_puniqueid,
    })
}

fn pid_buffer_byte_capacity(buffer: &[i32]) -> Result<c_int, Error> {
    buffer
        .len()
        .checked_mul(size_of::<i32>())
        .and_then(|bytes| c_int::try_from(bytes).ok())
        .ok_or(Error::BufferTooSmall)
}

fn pid_count(returned: c_int, error_number: i32, capacity: usize) -> Result<usize, Error> {
    if returned < 0 {
        return if error_number == 0 {
            Err(Error::UnexpectedResult)
        } else {
            Err(classify_errno(error_number))
        };
    }
    if returned == 0 && error_number != 0 {
        return Err(classify_errno(error_number));
    }
    // Unlike `proc_listpids`, both specialized list calls return a PID count
    // even though their input capacity is expressed in bytes.
    let returned = usize::try_from(returned).map_err(|_| Error::UnexpectedResult)?;
    if returned > capacity {
        return Err(Error::UnexpectedResult);
    }
    if returned == capacity {
        return Err(Error::Truncated);
    }
    Ok(returned)
}

fn query_pid_info<T: DarwinRecord>(pid: i32) -> Result<T, Error> {
    let mut value = MaybeUninit::<T>::uninit();
    let expected = c_int::try_from(size_of::<T>()).map_err(|_| Error::UnexpectedResult)?;
    clear_errno();
    // SAFETY: `value` is properly aligned and writable for exactly `expected`
    // bytes. It is assumed initialized only when libproc reports that exact
    // byte count.
    let returned = unsafe {
        libc::proc_pidinfo(
            pid,
            T::FLAVOR,
            0,
            value.as_mut_ptr().cast::<c_void>(),
            expected,
        )
    };
    let error_number = current_errno();
    if returned != expected {
        if returned == 0 {
            if error_number == 0 {
                return Err(Error::NotFound);
            }
            return Err(classify_errno(error_number));
        }
        return Err(Error::UnexpectedResult);
    }
    // SAFETY: the exact successful byte count proves that libproc initialized
    // the complete `T` record at the pointer supplied above.
    Ok(unsafe { value.assume_init() })
}

fn classify_errno(error_number: i32) -> Error {
    match error_number {
        0 | libc::ESRCH => Error::NotFound,
        libc::EACCES | libc::EPERM => Error::PermissionDenied,
        _ => Error::System,
    }
}

fn clear_errno() {
    // SAFETY: `__error` returns the calling thread's valid errno pointer on
    // macOS. Clearing it is required because zero is both a valid list count
    // and libproc's failure sentinel.
    unsafe {
        *libc::__error() = 0;
    }
}

fn current_errno() -> i32 {
    // SAFETY: `__error` returns the calling thread's valid errno pointer.
    unsafe { *libc::__error() }
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, offset_of, size_of};

    use crate::Error;

    use super::{ProcBsdInfo, ProcUniqueIdentifierInfo, pid_count};

    #[test]
    fn unique_identifier_layout_matches_xnu_flavor_17() {
        assert_eq!(size_of::<ProcUniqueIdentifierInfo>(), 56);
        assert_eq!(align_of::<ProcUniqueIdentifierInfo>(), 8);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_uuid), 0);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_uniqueid), 16);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_puniqueid), 24);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_idversion), 32);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_orig_ppidversion), 36);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_reserve2), 40);
        assert_eq!(offset_of!(ProcUniqueIdentifierInfo, p_reserve3), 48);
    }

    #[test]
    fn bsd_info_layout_matches_xnu_flavor_3() {
        assert_eq!(size_of::<ProcBsdInfo>(), 136);
        assert_eq!(align_of::<ProcBsdInfo>(), 8);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_pid), 12);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_ppid), 16);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_comm), 48);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_name), 64);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_nfiles), 96);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_pgid), 100);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_nice), 116);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_start_tvsec), 120);
        assert_eq!(offset_of!(ProcBsdInfo, pbi_start_tvusec), 128);
    }

    #[test]
    fn pid_count_classification_is_bounded_and_deterministic() {
        assert_eq!(pid_count(0, 0, 4), Ok(0));
        assert_eq!(pid_count(3, 0, 4), Ok(3));
        assert_eq!(pid_count(4, 0, 4), Err(Error::Truncated));
        assert_eq!(pid_count(5, 0, 4), Err(Error::UnexpectedResult));
        assert_eq!(pid_count(0, libc::ESRCH, 4), Err(Error::NotFound));
        assert_eq!(pid_count(-1, libc::EPERM, 4), Err(Error::PermissionDenied));
        assert_eq!(pid_count(-1, libc::EIO, 4), Err(Error::System));
    }
}
