//! Fixed read-only Darwin process enumeration (ADR 0004).

use std::{io, num::NonZeroU32};

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("process inventory requires a separately verified Darwin kinfo_proc ABI");

// Verified against both 64-bit Darwin SDK ABIs by tests/process_inventory_abi.c.
// Scratch storage is bytes: no pointer-bearing kernel record becomes a Rust
// value or escapes this module. Only the signed pid_t field is decoded.
const RECORD_BYTES: usize = 648;
const PID_OFFSET: usize = 40;
const MAX_RAW_BYTES: usize = 8 * 1024 * 1024;
const MAX_DATA_CALLS: usize = 3;
const MIN_HEADROOM_BYTES: usize = 16 * RECORD_BYTES;

/// Returns a bounded, sorted inventory of observed positive macOS process IDs.
///
/// This is read-only discovery, not a process-incarnation, membership, signaling,
/// or session-ownership grant. Callers must independently authenticate every
/// candidate. The kernel call can block: deadline-sensitive callers must invoke
/// this only in their independently cancellable, owned helper process.
///
/// Raw storage is at most 8 MiB; the result contains at most 12,945 IDs. There
/// are at most three data calls, each preceded by one size query. Size hints
/// include kernel headroom and are clamped, not treated as copied record bytes.
/// No partially copied inventory is returned after an OS or validation error.
///
/// # Errors
/// Returns kernel/allocation failures, the last `ENOMEM` after bounded churn,
/// or `InvalidData` for malformed lengths, negative IDs or duplicate records.
pub fn process_ids() -> io::Result<Vec<NonZeroU32>> {
    process_ids_with(read_inventory)
}

fn process_ids_with(
    mut read: impl FnMut(Option<&mut [u8]>) -> io::Result<usize>,
) -> io::Result<Vec<NonZeroU32>> {
    let mut previous_capacity = 0;
    for attempt in 0..MAX_DATA_CALLS {
        let hint = read(None)?;
        // The size-only query includes KERN_PROCSLOP and may exceed the cap
        // even when the real records fit. Always allow the capped data call.
        // Saturation prevents a malformed/huge hint from overflowing arithmetic.
        let capacity = hint
            .saturating_add((hint / 8).max(MIN_HEADROOM_BYTES))
            .max(previous_capacity)
            .min(MAX_RAW_BYTES);
        let mut scratch = Vec::new();
        scratch
            .try_reserve_exact(capacity)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        scratch.resize(capacity, 0);
        match read(Some(&mut scratch)) {
            Ok(copied) => return decode_inventory(&scratch, copied),
            Err(error)
                if error.raw_os_error() == Some(libc::ENOMEM) && attempt + 1 < MAX_DATA_CALLS =>
            {
                // Drop this entire scratch buffer before retrying. Never decode
                // the prefix returned with ENOMEM, even if it contains full rows.
                previous_capacity = capacity.saturating_mul(2).min(MAX_RAW_BYTES);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the final data call always returns its result")
}

fn decode_inventory(scratch: &[u8], copied: usize) -> io::Result<Vec<NonZeroU32>> {
    if copied == 0
        || copied > MAX_RAW_BYTES
        || copied > scratch.len()
        || !copied.is_multiple_of(RECORD_BYTES)
    {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let mut ids = Vec::new();
    ids.try_reserve_exact(copied / RECORD_BYTES)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    let mut kernel_row_seen = false;
    for row in scratch[..copied].chunks_exact(RECORD_BYTES) {
        let mut bytes = [0; size_of::<i32>()];
        bytes.copy_from_slice(&row[PID_OFFSET..PID_OFFSET + size_of::<i32>()]);
        let pid = u32::try_from(i32::from_ne_bytes(bytes))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        if let Some(pid) = NonZeroU32::new(pid) {
            ids.push(pid);
        } else if kernel_row_seen {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        } else {
            kernel_row_seen = true;
        }
    }
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(ids)
}

#[allow(unsafe_code)] // ADR 0004: fixed read-only process inventory only.
fn read_inventory(scratch: Option<&mut [u8]>) -> io::Result<usize> {
    let mut selector = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL, 0];
    let (buffer, mut bytes) = scratch.map_or((std::ptr::null_mut(), 0), |scratch| {
        (scratch.as_mut_ptr().cast::<libc::c_void>(), scratch.len())
    });
    // SAFETY: The selector is a fixed initialized four-int MIB. The new-value
    // pointer and length are always null/zero, forbidding writes. oldlenp points
    // to an initialized writable size_t. oldp is either null for the size query
    // or an exclusively borrowed initialized byte allocation of exactly `bytes`
    // writable bytes. sysctl copies bytes synchronously and retains no pointers;
    // it does not require userspace kinfo_proc alignment. No returned raw record
    // is interpreted as a Rust struct. Success length is validated before any
    // safe slice access, and errors discard all copied bytes.
    let result = unsafe {
        libc::sysctl(
            selector.as_mut_ptr(),
            4,
            buffer,
            &raw mut bytes,
            std::ptr::null_mut(),
            0,
        )
    };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    fn records(pids: &[i32]) -> Vec<u8> {
        let mut bytes = vec![0; pids.len() * RECORD_BYTES];
        for (pid, row) in pids.iter().zip(bytes.chunks_exact_mut(RECORD_BYTES)) {
            row[PID_OFFSET..PID_OFFSET + size_of::<i32>()].copy_from_slice(&pid.to_ne_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_only_positive_unique_ids_and_skips_one_kernel_row() {
        let bytes = records(&[42, 0, 1, i32::MAX]);
        let ids = decode_inventory(&bytes, bytes.len()).unwrap();
        assert_eq!(
            ids.iter().map(|pid| pid.get()).collect::<Vec<_>>(),
            [1, 42, 2_147_483_647]
        );
        let bytes = records(&[0]);
        assert!(decode_inventory(&bytes, bytes.len()).unwrap().is_empty());
        for pids in [&[-1][..], &[1, 1], &[0, 0], &[1, 0, 1], &[i32::MIN]] {
            let bytes = records(pids);
            assert_eq!(
                decode_inventory(&bytes, bytes.len()).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn rejects_truncated_oversized_and_overflowed_copied_lengths() {
        let bytes = records(&[1, 2]);
        for copied in [
            0,
            1,
            RECORD_BYTES - 1,
            RECORD_BYTES + 1,
            bytes.len() + RECORD_BYTES,
            MAX_RAW_BYTES + 1,
            usize::MAX,
        ] {
            assert_eq!(
                decode_inventory(&bytes, copied).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        assert_eq!(decode_inventory(&bytes, RECORD_BYTES).unwrap().len(), 1);
    }

    #[test]
    fn exact_raw_record_capacity_is_accepted_but_one_more_record_is_rejected() {
        let count = MAX_RAW_BYTES / RECORD_BYTES;
        let pids: Vec<_> = (1..=i32::try_from(count + 1).unwrap()).collect();
        let bytes = records(&pids);
        assert_eq!(
            decode_inventory(&bytes, count * RECORD_BYTES)
                .unwrap()
                .len(),
            count
        );
        // The scratch contains this complete extra row: the raw cap, not the
        // slice length or record alignment, must reject it.
        assert_eq!(
            decode_inventory(&bytes, bytes.len()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn size_hints_are_not_record_lengths_and_above_cap_slop_still_gets_a_data_call() {
        for hint in [
            0,
            1,
            RECORD_BYTES + 1,
            MAX_RAW_BYTES + RECORD_BYTES,
            usize::MAX,
        ] {
            let mut calls = 0;
            let result = process_ids_with(|scratch| {
                calls += 1;
                let Some(scratch) = scratch else {
                    return Ok(hint);
                };
                assert!(scratch.len() <= MAX_RAW_BYTES);
                assert!(scratch.iter().all(|byte| *byte == 0));
                if hint > MAX_RAW_BYTES {
                    assert_eq!(scratch.len(), MAX_RAW_BYTES);
                }
                scratch[..RECORD_BYTES].copy_from_slice(&records(&[17]));
                Ok(RECORD_BYTES)
            })
            .unwrap();
            assert_eq!(calls, 2);
            assert_eq!(result[0].get(), 17);
        }
    }

    #[test]
    fn enomem_discards_partial_rows_and_retries_with_bounded_growth() {
        let mut data_calls = 0;
        let mut sizes = Vec::new();
        let ids = process_ids_with(|scratch| {
            let Some(scratch) = scratch else {
                return Ok(RECORD_BYTES);
            };
            data_calls += 1;
            assert!(scratch.iter().all(|byte| *byte == 0));
            sizes.push(scratch.len());
            if data_calls < MAX_DATA_CALLS {
                scratch[..RECORD_BYTES].copy_from_slice(&records(&[-1]));
                Err(io::Error::from_raw_os_error(libc::ENOMEM))
            } else {
                scratch[..RECORD_BYTES].copy_from_slice(&records(&[23]));
                Ok(RECORD_BYTES)
            }
        })
        .unwrap();
        assert_eq!(data_calls, 3);
        assert_eq!(ids[0].get(), 23);
        assert!(sizes.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(sizes.iter().all(|size| *size <= MAX_RAW_BYTES));
    }

    #[test]
    fn enomem_exhaustion_returns_error_without_any_partial_inventory() {
        let mut queries = 0;
        let mut data_calls = 0;
        let error = process_ids_with(|scratch| {
            let Some(scratch) = scratch else {
                queries += 1;
                return Ok(MAX_RAW_BYTES);
            };
            data_calls += 1;
            assert_eq!(scratch.len(), MAX_RAW_BYTES);
            scratch[..RECORD_BYTES].copy_from_slice(&records(&[17]));
            Err(io::Error::from_raw_os_error(libc::ENOMEM))
        })
        .unwrap_err();
        assert_eq!(queries, 3);
        assert_eq!(data_calls, 3);
        assert_eq!(error.raw_os_error(), Some(libc::ENOMEM));
    }

    #[test]
    fn other_errors_are_not_retried_or_decoded() {
        for failing_call in [1, 2] {
            let mut calls = 0;
            let error = process_ids_with(|scratch| {
                calls += 1;
                if calls == failing_call {
                    if let Some(scratch) = scratch {
                        scratch[..RECORD_BYTES].copy_from_slice(&records(&[17]));
                    }
                    return Err(io::Error::from_raw_os_error(libc::EINTR));
                }
                Ok(RECORD_BYTES)
            })
            .unwrap_err();
            assert_eq!(calls, failing_call);
            assert_eq!(error.raw_os_error(), Some(libc::EINTR));
        }
    }

    #[test]
    fn query_error_after_enomem_never_publishes_the_previous_prefix() {
        let mut calls = 0;
        let error = process_ids_with(|scratch| {
            calls += 1;
            match calls {
                1 => Ok(RECORD_BYTES),
                2 => {
                    let scratch = scratch.unwrap();
                    scratch[..RECORD_BYTES].copy_from_slice(&records(&[17]));
                    Err(io::Error::from_raw_os_error(libc::ENOMEM))
                }
                3 => Err(io::Error::from_raw_os_error(libc::EIO)),
                _ => panic!("query failure must stop enumeration"),
            }
        })
        .unwrap_err();
        assert_eq!(calls, 3);
        assert_eq!(error.raw_os_error(), Some(libc::EIO));
    }

    #[test]
    fn raw_cap_covers_every_inventory_that_fits_the_existing_text_limit() {
        let mut text_bytes = 0;
        let mut pids = 0usize;
        loop {
            let next_bytes = (pids + 1).to_string().len() + 1;
            if text_bytes + next_bytes > 64 * 1024 {
                break;
            }
            pids += 1;
            text_bytes += next_bytes;
        }
        assert_eq!((pids, text_bytes), (12_773, 65_532));
        assert_eq!((pids + 1) * RECORD_BYTES, 8_277_552);
        assert!((pids + 1) * RECORD_BYTES <= MAX_RAW_BYTES);
        assert_eq!(MAX_RAW_BYTES / RECORD_BYTES, 12_945);
    }

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn live_inventory_contains_current_process_and_owned_child() {
        let child = OwnedChild(
            Command::new("/bin/sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let ids = process_ids().unwrap();
        assert!(ids.contains(&NonZeroU32::new(std::process::id()).unwrap()));
        assert!(ids.contains(&NonZeroU32::new(child.0.id()).unwrap()));
    }
}
