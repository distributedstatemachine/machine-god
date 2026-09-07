//! Framing for the explicitly owned, independently killable inventory service.
#![cfg(target_os = "macos")]

use crate::background_process::{GROUP_SNAPSHOT_TIMEOUT, MAX_GROUP_SNAPSHOT_BYTES};
use crate::terminal_helper::{
    TerminalHelperError, TerminalHelperErrorKind, check_deadline, decode_helper_deadline,
    encode_helper_deadline,
};
use machine_god_core::{CancellationToken, MAX_TERMINAL_EXEC_DURATION};
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::time::Instant;

/// Exact private mode, dispatched before ordinary CLI configuration.
#[doc(hidden)]
pub const PROCESS_INVENTORY_SERVICE_ARGUMENT: &str = "--machine-god-process-inventory-service";
pub(crate) const STARTUP_ENV: &str = "MACHINE_GOD_PROCESS_INVENTORY_SERVICE_DEADLINE";
pub(crate) const READY: [u8; 8] = *b"MGIRDY01";

const fn failure(kind: TerminalHelperErrorKind) -> TerminalHelperError {
    TerminalHelperError { kind }
}

pub(crate) fn encode_request(
    sequence: u64,
    deadline: Instant,
) -> Result<[u8; 24], TerminalHelperError> {
    if sequence == 0 {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    let stamp = encode_helper_deadline(deadline, GROUP_SNAPSHOT_TIMEOUT)?;
    let (seconds, nanos) = stamp
        .split_once(':')
        .expect("deadline encoder has fixed format");
    let seconds = seconds.parse::<u64>().expect("deadline seconds are u64");
    let nanos = nanos.parse::<u32>().expect("deadline nanoseconds are u32");
    let mut frame = [0; 24];
    frame[..4].copy_from_slice(b"MGI1");
    frame[4..12].copy_from_slice(&sequence.to_be_bytes());
    frame[12..20].copy_from_slice(&seconds.to_be_bytes());
    frame[20..].copy_from_slice(&nanos.to_be_bytes());
    Ok(frame)
}

pub(crate) fn decode_request(frame: &[u8; 24]) -> Result<(u64, Instant), TerminalHelperError> {
    let sequence = u64::from_be_bytes(frame[4..12].try_into().expect("fixed sequence width"));
    if &frame[..4] != b"MGI1" || sequence == 0 {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    let seconds = u64::from_be_bytes(frame[12..20].try_into().expect("fixed seconds width"));
    let nanos = u32::from_be_bytes(frame[20..].try_into().expect("fixed nanoseconds width"));
    let deadline = decode_helper_deadline(&format!("{seconds}:{nanos}"), GROUP_SNAPSHOT_TIMEOUT)?;
    Ok((sequence, deadline))
}

pub(crate) fn encode_response_header(
    sequence: u64,
    length: usize,
) -> Result<[u8; 17], TerminalHelperError> {
    if sequence == 0 || !(1..=MAX_GROUP_SNAPSHOT_BYTES).contains(&length) {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    let mut frame = [0; 17];
    frame[..4].copy_from_slice(b"MGO1");
    frame[4..12].copy_from_slice(&sequence.to_be_bytes());
    // Byte 12 is the sole accepted status, zero (success).
    frame[13..].copy_from_slice(
        &u32::try_from(length)
            .expect("bounded payload length")
            .to_be_bytes(),
    );
    Ok(frame)
}

pub(crate) fn decode_response_header(
    frame: &[u8; 17],
    expected: u64,
) -> Result<usize, TerminalHelperError> {
    let sequence = u64::from_be_bytes(frame[4..12].try_into().expect("fixed sequence width"));
    let length = u32::from_be_bytes(frame[13..].try_into().expect("fixed length width"));
    let length = usize::try_from(length).map_err(|_| failure(TerminalHelperErrorKind::Protocol))?;
    if &frame[..4] != b"MGO1"
        || expected == 0
        || sequence != expected
        || frame[12] != 0
        || !(1..=MAX_GROUP_SNAPSHOT_BYTES).contains(&length)
    {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    Ok(length)
}

pub(crate) fn encode_completion(sequence: u64) -> [u8; 12] {
    let mut frame = [0; 12];
    frame[..4].copy_from_slice(b"MGD1");
    frame[4..].copy_from_slice(&sequence.to_be_bytes());
    frame
}

pub(crate) fn validate_completion(
    frame: &[u8; 12],
    expected: u64,
) -> Result<(), TerminalHelperError> {
    if expected == 0 || *frame != encode_completion(expected) {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    Ok(())
}

/// Runs the private service on inherited pipes, without threads or shared pools.
/// Readiness indicates protocol availability, not process identity authority.
///
/// # Errors
/// Returns a fixed error for invalid startup metadata, malformed or stale
/// requests, expired deadlines, query failures, or incomplete pipe operations.
/// The parent must still enforce the original deadline and own kill/reap.
#[doc(hidden)]
pub fn run_process_inventory_service() -> Result<(), TerminalHelperError> {
    run_with_io(&mut std::io::stdin().lock(), &mut std::io::stdout().lock())
}

fn run_with_io(input: &mut impl Read, output: &mut impl Write) -> Result<(), TerminalHelperError> {
    let stamp =
        std::env::var(STARTUP_ENV).map_err(|_| failure(TerminalHelperErrorKind::InvalidRequest))?;
    let startup = decode_helper_deadline(&stamp, MAX_TERMINAL_EXEC_DURATION)?;
    let cancellation = CancellationToken::new();
    serve(
        input,
        output,
        startup,
        || {
            machine_god_terminal_sys::process_ids()
                .map_err(|_| failure(TerminalHelperErrorKind::Process))
        },
        |deadline| check_deadline(deadline, &cancellation),
    )
}

fn serve(
    input: &mut impl Read,
    output: &mut impl Write,
    startup: Instant,
    mut query: impl FnMut() -> Result<Vec<NonZeroU32>, TerminalHelperError>,
    mut check: impl FnMut(Instant) -> Result<(), TerminalHelperError>,
) -> Result<(), TerminalHelperError> {
    check(startup)?;
    output
        .write_all(&READY)
        .and_then(|()| output.flush())
        .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
    check(startup)?;
    let mut expected = 1;
    loop {
        // Idle EOF is a clean parent close. EOF within a frame is never success.
        let mut request = [0; 24];
        match input.read_exact(&mut request[..1]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(_) => return Err(failure(TerminalHelperErrorKind::Process)),
        }
        input
            .read_exact(&mut request[1..])
            .map_err(|_| failure(TerminalHelperErrorKind::Protocol))?;
        let (sequence, deadline) = decode_request(&request)?;
        let next = next_sequence(sequence, expected)?;
        check(deadline)?;
        // A potentially blocking query stays inside this killable child.
        let pids = query()?;
        check(deadline)?;
        let payload = crate::process_inventory_helper::encode_inventory(&pids)?;
        let header = encode_response_header(sequence, payload.len())?;
        check(deadline)?;
        output
            .write_all(&header)
            .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
        check(deadline)?;
        output
            .write_all(&payload)
            .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
        check(deadline)?;
        output
            .write_all(&encode_completion(sequence))
            .and_then(|()| output.flush())
            .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
        check(deadline)?;
        expected = next;
    }
}

fn next_sequence(sequence: u64, expected: u64) -> Result<u64, TerminalHelperError> {
    if sequence != expected || sequence == 0 {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    sequence
        .checked_add(1)
        .ok_or_else(|| failure(TerminalHelperErrorKind::Protocol))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_entry() {
        if std::env::var_os(STARTUP_ENV).is_none() {
            return;
        }
        let result = run_with_io(&mut std::io::stdin().lock(), &mut std::io::stderr().lock());
        std::process::exit(if result.is_ok() { 0 } else { 125 });
    }

    fn deadline() -> Instant {
        Instant::now() + GROUP_SNAPSHOT_TIMEOUT
    }

    fn pids() -> Vec<NonZeroU32> {
        vec![NonZeroU32::new(1).unwrap(), NonZeroU32::new(42).unwrap()]
    }

    #[test]
    fn request_is_versioned_big_endian_and_conservatively_deadline_bound() {
        let original = deadline();
        let frame = encode_request(42, original).unwrap();
        assert_eq!(&frame[..12], b"MGI1\0\0\0\0\0\0\0*");
        let (sequence, decoded) = decode_request(&frame).unwrap();
        assert_eq!(sequence, 42);
        assert!(decoded <= original);
        assert!(encode_request(0, original).is_err());
        assert!(encode_request(1, Instant::now()).is_err());
        assert!(encode_request(1, Instant::now() + MAX_TERMINAL_EXEC_DURATION).is_err());
        for range in [0..4, 4..12] {
            let mut invalid = frame;
            invalid[range].fill(0);
            assert!(decode_request(&invalid).is_err());
        }
        let mut invalid = frame;
        invalid[20..].copy_from_slice(&1_000_000_000u32.to_be_bytes());
        assert!(decode_request(&invalid).is_err());
        invalid[12..20].fill(0);
        invalid[20..].fill(0);
        assert_eq!(
            decode_request(&invalid).unwrap_err().kind,
            TerminalHelperErrorKind::Timeout
        );
        invalid[12..20].fill(255);
        assert!(decode_request(&invalid).is_err());
    }

    #[test]
    fn response_headers_and_completion_reject_malformed_stale_or_excessive_frames() {
        for length in [1, MAX_GROUP_SNAPSHOT_BYTES] {
            let frame = encode_response_header(42, length).unwrap();
            assert_eq!(&frame[..13], b"MGO1\0\0\0\0\0\0\0*\0");
            assert_eq!(decode_response_header(&frame, 42).unwrap(), length);
            assert!(decode_response_header(&frame, 41).is_err());
            assert!(decode_response_header(&frame, 0).is_err());
            for index in [0, 3, 4, 11, 12] {
                let mut invalid = frame;
                invalid[index] ^= 1;
                assert!(decode_response_header(&invalid, 42).is_err());
            }
        }
        for length in [0, MAX_GROUP_SNAPSHOT_BYTES + 1, usize::MAX] {
            assert!(encode_response_header(1, length).is_err());
        }
        assert!(encode_response_header(0, 1).is_err());
        for length in [0, 65_537, u32::MAX] {
            let mut invalid = encode_response_header(1, 1).unwrap();
            invalid[13..].copy_from_slice(&length.to_be_bytes());
            assert!(decode_response_header(&invalid, 1).is_err());
        }
        let footer = encode_completion(42);
        assert_eq!(&footer, b"MGD1\0\0\0\0\0\0\0*");
        validate_completion(&footer, 42).unwrap();
        assert!(validate_completion(&footer, 41).is_err());
        assert!(validate_completion(&encode_completion(0), 0).is_err());
        for index in 0..footer.len() {
            let mut invalid = footer;
            invalid[index] ^= 1;
            assert!(validate_completion(&invalid, 42).is_err());
        }
    }

    #[test]
    fn sequence_is_strictly_monotonic_and_overflow_fails_before_query() {
        assert_eq!(next_sequence(1, 1).unwrap(), 2);
        assert_eq!(next_sequence(u64::MAX - 1, u64::MAX - 1).unwrap(), u64::MAX);
        for (sequence, expected) in [(0, 0), (1, 2), (3, 2), (u64::MAX, u64::MAX)] {
            assert!(next_sequence(sequence, expected).is_err());
        }
    }

    #[test]
    fn loop_serves_multiple_requests_and_idle_eof_without_refreshing_deadlines() {
        let first = deadline();
        let second = first
            .checked_sub(std::time::Duration::from_millis(1))
            .unwrap();
        let requests = [
            encode_request(1, first).unwrap(),
            encode_request(2, second).unwrap(),
        ]
        .concat();
        let startup = Instant::now() + MAX_TERMINAL_EXEC_DURATION;
        let mut checked = Vec::new();
        let mut output = Vec::new();
        serve(
            &mut requests.as_slice(),
            &mut output,
            startup,
            || Ok(pids()),
            |value| {
                checked.push(value);
                Ok(())
            },
        )
        .unwrap();
        let mut expected = READY.to_vec();
        for sequence in [1, 2] {
            expected.extend(encode_response_header(sequence, 5).unwrap());
            expected.extend(b"1\n42\n");
            expected.extend(encode_completion(sequence));
        }
        assert_eq!(output, expected);
        assert_eq!(&checked[..2], &[startup, startup]);
        assert_eq!(checked.len(), 14);
        assert!(checked[2..8].iter().all(|value| *value <= first));
        assert!(checked[8..].iter().all(|value| *value <= second));
        assert!(checked[2..8].iter().all(|value| *value == checked[2]));
        assert!(checked[8..].iter().all(|value| *value == checked[8]));
    }

    #[test]
    fn partial_requests_and_bad_sequences_never_invoke_query() {
        let request = encode_request(1, deadline()).unwrap();
        for length in 1..request.len() {
            let mut output = Vec::new();
            assert!(
                serve(
                    &mut &request[..length],
                    &mut output,
                    deadline(),
                    || panic!("partial request queried"),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(output, READY);
        }
        for sequence in [2, u64::MAX] {
            let request = encode_request(sequence, deadline()).unwrap();
            let mut output = Vec::new();
            assert!(
                serve(
                    &mut request.as_slice(),
                    &mut output,
                    deadline(),
                    || panic!("out-of-order query"),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(output, READY);
        }
        let mut output = Vec::new();
        serve(
            &mut &b""[..],
            &mut output,
            deadline(),
            || panic!("EOF queried"),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(output, READY);
    }

    #[test]
    fn malformed_or_expired_request_is_rejected_before_query() {
        let request = encode_request(1, deadline()).unwrap();
        let mut invalid = Vec::new();
        for range in [0..4, 4..12, 12..24] {
            let mut frame = request;
            frame[range].fill(0);
            invalid.push(frame);
        }
        let mut bad_nanos = request;
        bad_nanos[20..].copy_from_slice(&u32::MAX.to_be_bytes());
        invalid.push(bad_nanos);
        for frame in invalid {
            let mut output = Vec::new();
            assert!(
                serve(
                    &mut frame.as_slice(),
                    &mut output,
                    deadline(),
                    || panic!("invalid request queried"),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(output, READY);
        }
    }

    #[test]
    fn duplicate_request_stops_after_first_complete_response() {
        let request = encode_request(1, deadline()).unwrap();
        let requests = [request, request].concat();
        let mut queries = 0;
        let mut output = Vec::new();
        assert!(
            serve(
                &mut requests.as_slice(),
                &mut output,
                deadline(),
                || {
                    queries += 1;
                    Ok(pids())
                },
                |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(queries, 1);
        assert_eq!(output.len(), READY.len() + 17 + 5 + 12);
    }

    #[test]
    fn query_failure_and_invalid_inventory_never_emit_success_header() {
        for result in [
            Err(failure(TerminalHelperErrorKind::Process)),
            Ok(Vec::new()),
            Ok(vec![NonZeroU32::new(1).unwrap(); 2]),
        ] {
            let request = encode_request(1, deadline()).unwrap();
            let mut output = Vec::new();
            assert!(
                serve(
                    &mut request.as_slice(),
                    &mut output,
                    deadline(),
                    || result.clone(),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(output, READY);
        }
    }

    #[test]
    fn deadline_failure_before_completion_never_emits_complete_success() {
        // Checkpoints: startup, readiness, request, query, header, payload, footer.
        for fail_at in 1..=7 {
            let request = encode_request(1, deadline()).unwrap();
            let mut output = Vec::new();
            let mut checks = 0;
            let result = serve(
                &mut request.as_slice(),
                &mut output,
                deadline(),
                || Ok(pids()),
                |_| {
                    checks += 1;
                    if checks == fail_at {
                        Err(failure(TerminalHelperErrorKind::Timeout))
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(result.unwrap_err().kind, TerminalHelperErrorKind::Timeout);
            assert!(!output.ends_with(&encode_completion(1)));
        }
    }

    #[test]
    fn partial_writes_and_io_errors_cannot_produce_complete_response() {
        struct FailingWriter {
            bytes: Vec<u8>,
            limit: usize,
        }
        impl Write for FailingWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                let amount = bytes.len().min(self.limit - self.bytes.len()).min(2);
                if amount == 0 {
                    return Err(std::io::Error::other("scripted write failure"));
                }
                self.bytes.extend_from_slice(&bytes[..amount]);
                Ok(amount)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for limit in 0..(READY.len() + 17 + 5 + 12) {
            let request = encode_request(1, deadline()).unwrap();
            let mut output = FailingWriter {
                bytes: Vec::new(),
                limit,
            };
            assert!(
                serve(
                    &mut request.as_slice(),
                    &mut output,
                    deadline(),
                    || Ok(pids()),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(output.bytes.len(), limit);
            assert!(!output.bytes.ends_with(&encode_completion(1)));
        }
    }

    #[test]
    fn chunked_interrupted_reads_preserve_frames_and_read_errors_stop_queries() {
        struct ScriptedReader<'a> {
            bytes: &'a [u8],
            interrupted: bool,
            fail: bool,
        }
        impl Read for ScriptedReader<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                if self.bytes.is_empty() && self.fail {
                    return Err(std::io::Error::other("scripted read failure"));
                }
                let amount = output.len().min(self.bytes.len()).min(1);
                output[..amount].copy_from_slice(&self.bytes[..amount]);
                self.bytes = &self.bytes[amount..];
                Ok(amount)
            }
        }
        let request = encode_request(1, deadline()).unwrap();
        let mut output = Vec::new();
        serve(
            &mut ScriptedReader {
                bytes: &request,
                interrupted: false,
                fail: false,
            },
            &mut output,
            deadline(),
            || Ok(pids()),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(output.len(), READY.len() + 17 + 5 + 12);
        for length in 0..request.len() {
            let mut output = Vec::new();
            assert!(
                serve(
                    &mut ScriptedReader {
                        bytes: &request[..length],
                        interrupted: false,
                        fail: true
                    },
                    &mut output,
                    deadline(),
                    || panic!("read error queried"),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(output, READY);
        }
    }
}
