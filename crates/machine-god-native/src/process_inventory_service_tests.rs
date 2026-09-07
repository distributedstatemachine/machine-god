//! Lifecycle evidence for explicitly prepared, leased inventory services.

use super::*;
use crate::NativeOwnedWorkerScope;
use crate::background_process::GROUP_SNAPSHOT_TIMEOUT;
use crate::process_inventory_protocol as wire;
use crate::terminal_helper::MAX_STARTUP_TIMEOUT;
use futures_executor::block_on;
use std::io::Read;
use std::time::Duration;

fn startup_deadline() -> Instant {
    Instant::now() + Duration::from_secs(5).min(MAX_STARTUP_TIMEOUT)
}

fn prepare_in(scope: &NativeOwnedWorkerScope, helper: &ProcessInventoryHelper) -> InventoryLease {
    let helper = helper.clone();
    let deadline = startup_deadline();
    let prepared = block_on(scope.run(move || helper.prepare(deadline, &CancellationToken::new())))
        .unwrap()
        .unwrap();
    let PreparedProcessInventory::Service(lease) = prepared else {
        panic!("explicit service must produce a service lease");
    };
    lease
}

fn query_in(
    scope: &NativeOwnedWorkerScope,
    lease: &InventoryLease,
) -> Result<Vec<u8>, TerminalHelperError> {
    let lease = lease.clone();
    block_on(scope.run(move || lease.query(Instant::now() + GROUP_SNAPSHOT_TIMEOUT))).unwrap()
}

fn close_scope(scope: &NativeOwnedWorkerScope) {
    scope.close();
    scope.completion().wait_on_worker().unwrap();
    assert!(scope.completion().is_complete());
}

#[test]
fn service_registration_is_inert_and_precancelled_or_expired_prepare_never_spawns() {
    let helper = test_service();
    let retained = helper.clone();
    let scope = NativeOwnedWorkerScope::new();
    let deferred = helper.clone();
    let future = scope.run(move || deferred.prepare(startup_deadline(), &CancellationToken::new()));
    assert_eq!(helper.service_spawn_count_for_test(), 0);
    drop(future);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        helper
            .prepare(startup_deadline(), &cancellation)
            .unwrap_err()
            .kind,
        TerminalHelperErrorKind::Cancelled
    );
    assert_eq!(
        helper
            .prepare(Instant::now(), &CancellationToken::new())
            .unwrap_err()
            .kind,
        TerminalHelperErrorKind::Timeout
    );
    assert_eq!(retained.service_spawn_count_for_test(), 0);
    close_scope(&scope);
}

#[test]
fn same_scope_different_workers_and_repeated_queries_share_one_helper() {
    let helper = test_service();
    let scope = NativeOwnedWorkerScope::new();
    let first = prepare_in(&scope, &helper);
    // Each prepare uses a distinct enrolled worker/ticket in the same scope.
    let second = prepare_in(&scope, &helper.clone());
    for lease in [&first, &second, &first] {
        let bytes = query_in(&scope, lease).unwrap();
        assert!(
            decode_inventory(&bytes)
                .unwrap()
                .iter()
                .any(|pid| { pid.as_raw_nonzero().get().cast_unsigned() == std::process::id() })
        );
        assert_eq!(helper.service_spawn_count_for_test(), 1);
    }
    drop((first, second));
    close_scope(&scope);
    assert_eq!(helper.service_spawn_count_for_test(), 1);
}

#[test]
fn last_lease_drop_settles_its_scope_even_with_registration_retained() {
    let helper = test_service();
    let retained = helper.clone();
    let scope = NativeOwnedWorkerScope::new();
    let unrelated = NativeOwnedWorkerScope::new();
    let lease = prepare_in(&scope, &helper);
    let clone = lease.clone();
    scope.close();
    close_scope(&unrelated);
    assert!(!scope.completion().is_complete());
    drop(lease);
    assert!(!scope.completion().is_complete());
    drop(clone);
    scope.completion().wait_on_worker().unwrap();
    assert!(scope.completion().is_complete());
    assert_eq!(retained.service_spawn_count_for_test(), 1);
}

#[test]
fn active_registration_and_lease_reject_foreign_scope_but_rebind_is_independent() {
    let helper = test_service();
    let first_scope = NativeOwnedWorkerScope::new();
    let second_scope = NativeOwnedWorkerScope::new();
    let first = prepare_in(&first_scope, &helper);
    let foreign = helper.clone();
    let deadline = startup_deadline();
    assert_eq!(
        block_on(second_scope.run(move || foreign.prepare(deadline, &CancellationToken::new())))
            .unwrap()
            .unwrap_err()
            .kind,
        TerminalHelperErrorKind::InvalidRequest
    );
    assert_eq!(
        query_in(&second_scope, &first).unwrap_err().kind,
        TerminalHelperErrorKind::InvalidRequest
    );
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    let rebound = helper.rebind();
    assert_eq!(rebound.service_spawn_count_for_test(), 0);
    let second = prepare_in(&second_scope, &rebound);
    assert!(query_in(&first_scope, &first).is_ok());
    assert!(query_in(&second_scope, &second).is_ok());
    assert_eq!(rebound.service_spawn_count_for_test(), 1);
    drop(first);
    close_scope(&first_scope);
    assert!(!second_scope.completion().is_complete());
    assert!(query_in(&second_scope, &second).is_ok());
    drop(second);
    close_scope(&second_scope);
}

#[test]
fn expired_weak_lease_cannot_bypass_pending_retirement_before_restart() {
    let helper = test_service();
    let original_scope = NativeOwnedWorkerScope::new();
    let original = prepare_in(&original_scope, &helper);
    drop(original);
    close_scope(&original_scope);
    // Only completion metadata remains; no strong lease or child is retained.
    // Hold the same pending-reap predicate that a transferred child leaves set.
    let retirement = helper.hold_retirement_for_test();
    let next_scope = NativeOwnedWorkerScope::new();
    let next = helper.clone();
    let deadline = startup_deadline();
    assert!(
        block_on(next_scope.run(move || next.prepare(deadline, &CancellationToken::new())))
            .unwrap()
            .is_err()
    );
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    drop(retirement);
    let restarted = prepare_in(&next_scope, &helper);
    assert!(query_in(&next_scope, &restarted).is_ok());
    assert_eq!(helper.service_spawn_count_for_test(), 2);
    drop(restarted);
    close_scope(&next_scope);
}

#[test]
fn malformed_query_fails_without_same_call_respawn_and_retains_owned_cleanup() {
    let helper = controlled_helper("malformed");
    let scope = NativeOwnedWorkerScope::new();
    let lease = prepare_in(&scope, &helper);
    assert_eq!(
        query_in(&scope, &lease).unwrap_err().kind,
        TerminalHelperErrorKind::Protocol
    );
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    drop(lease);
    close_scope(&scope);
}

#[test]
fn trailing_reply_bytes_reject_complete_frame_without_respawn() {
    let helper = controlled_helper("trailing");
    let scope = NativeOwnedWorkerScope::new();
    let lease = prepare_in(&scope, &helper);
    assert_eq!(
        query_in(&scope, &lease).unwrap_err().kind,
        TerminalHelperErrorKind::Protocol
    );
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    drop(lease);
    close_scope(&scope);
}

#[test]
fn stalled_query_times_out_without_respawn_and_settles_its_scope() {
    let helper = controlled_helper("stalled");
    let scope = NativeOwnedWorkerScope::new();
    let lease = prepare_in(&scope, &helper);
    assert_eq!(
        query_in(&scope, &lease).unwrap_err().kind,
        TerminalHelperErrorKind::Timeout
    );
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    drop(lease);
    close_scope(&scope);
}

#[test]
fn cancellation_after_spawn_while_startup_is_pending_reaps_the_helper() {
    let helper = stalled_service_for_test();
    let scope = NativeOwnedWorkerScope::new();
    let cancellation = CancellationToken::new();
    let pending = helper.clone();
    let worker_cancel = cancellation.clone();
    let deadline = startup_deadline();
    let future = scope.run(move || pending.prepare(deadline, &worker_cancel));
    let result = std::thread::scope(|threads| {
        let preparation = threads.spawn(move || block_on(future).unwrap());
        // Positive child-spawn observation, not a sleep assuming admission.
        // The fixture cannot emit READY, so cancellation cannot race success.
        while helper.service_spawn_count_for_test() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let spawned = helper.service_spawn_count_for_test();
        cancellation.cancel();
        let result = preparation.join().unwrap();
        assert_eq!(spawned, 1, "cancellation must follow an owned child spawn");
        result
    });
    assert_eq!(result.unwrap_err().kind, TerminalHelperErrorKind::Cancelled);
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    close_scope(&scope);
}

#[test]
fn exhausted_sequence_fails_before_request_and_does_not_respawn() {
    let helper = test_service();
    let scope = NativeOwnedWorkerScope::new();
    let lease = prepare_in(&scope, &helper);
    lease.set_next_sequence_for_test(u64::MAX);
    assert_eq!(
        query_in(&scope, &lease).unwrap_err().kind,
        TerminalHelperErrorKind::Protocol
    );
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    drop(lease);
    close_scope(&scope);
}

#[test]
fn delayed_service_start_uses_startup_budget_then_queries_without_another_spawn() {
    let helper = controlled_helper("delayed");
    let scope = NativeOwnedWorkerScope::new();
    // This helper cannot become ready within the inventory deadline. It must
    // be prepared under the terminal's original, separate startup deadline.
    let lease = prepare_in(&scope, &helper);
    let first = query_in(&scope, &lease).unwrap();
    let second = query_in(&scope, &lease).unwrap();
    assert_eq!(first, second);
    assert_eq!(decode_inventory(&first).unwrap().len(), 1);
    assert_eq!(helper.service_spawn_count_for_test(), 1);
    drop(lease);
    close_scope(&scope);
}

const CONTROL_MODE: &str = "MACHINE_GOD_INVENTORY_TEST_MODE";

pub(super) fn controlled_helper(mode: &str) -> ProcessInventoryHelper {
    assert!(matches!(
        mode,
        "malformed" | "trailing" | "delayed" | "stalled" | "startup_stalled"
    ));
    let executable = std::env::current_exe().unwrap();
    let script = format!(
        "export {CONTROL_MODE}={mode}; exec '{}' --exact process_inventory_helper::service_tests::controlled_service_entry --nocapture 2>&1 1>/dev/null",
        executable.to_str().unwrap().replace('\'', "'\\''")
    );
    ProcessInventoryHelper::new_service("/bin/sh".into(), vec!["-c".into(), script.into()]).unwrap()
}

#[test]
fn controlled_service_entry() {
    let Ok(mode) = std::env::var(CONTROL_MODE) else {
        return;
    };
    let result = controlled_service(&mode);
    std::process::exit(if result.is_ok() { 0 } else { 125 });
}

fn controlled_service(mode: &str) -> Result<(), TerminalHelperError> {
    let stamp = std::env::var(wire::STARTUP_ENV).unwrap();
    let startup = decode_helper_deadline(&stamp, machine_god_core::MAX_TERMINAL_EXEC_DURATION)?;
    let cancellation = CancellationToken::new();
    let mut input = std::io::stdin().lock();
    if mode == "startup_stalled" {
        let mut byte = [0];
        // Parent cancellation must close the inherited pipe and retire this
        // exact owned child; no helper-created worker or subprocess is involved.
        let _ = input.read(&mut byte).map_err(io_failure)?;
        return Ok(());
    }
    if mode == "delayed" {
        std::thread::sleep(GROUP_SNAPSHOT_TIMEOUT + Duration::from_millis(50));
    }
    check_deadline(startup, &cancellation)?;
    let mut output = std::io::stderr().lock();
    output.write_all(&wire::READY).map_err(io_failure)?;
    output.flush().map_err(io_failure)?;
    let payload = format!("{}\n", std::process::id());
    let mut expected = 1_u64;
    loop {
        let mut request = [0; 24];
        match input.read_exact(&mut request[..1]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(io_failure(error)),
        }
        input.read_exact(&mut request[1..]).map_err(io_failure)?;
        let (sequence, deadline) = wire::decode_request(&request)?;
        assert_eq!(sequence, expected);
        expected = expected.checked_add(1).unwrap();
        check_deadline(deadline, &cancellation)?;
        if mode == "stalled" {
            continue;
        }
        let mut header = wire::encode_response_header(sequence, payload.len())?;
        if mode == "malformed" {
            header[0] ^= 1;
            output.write_all(&header).map_err(io_failure)?;
        } else if mode == "trailing" {
            // Publish the small complete frame and surplus byte together. The
            // helper then remains alive waiting for the next request, so the
            // rejection does not depend on an exit-observation race.
            let mut frame = header.to_vec();
            frame.extend_from_slice(payload.as_bytes());
            frame.extend_from_slice(&wire::encode_completion(sequence));
            frame.push(0x7f);
            output.write_all(&frame).map_err(io_failure)?;
        } else {
            output.write_all(&header).map_err(io_failure)?;
            output.write_all(payload.as_bytes()).map_err(io_failure)?;
            output
                .write_all(&wire::encode_completion(sequence))
                .map_err(io_failure)?;
        }
        output.flush().map_err(io_failure)?;
    }
}

fn io_failure(_: std::io::Error) -> TerminalHelperError {
    failure(TerminalHelperErrorKind::Process)
}
