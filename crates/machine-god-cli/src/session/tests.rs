use super::*;
use crate::test_support::*;
use crate::{INVALID_ARGUMENTS, OUTPUT_FAILURE, run_with_hosts};
use std::cell::{Cell, RefCell};
use std::ffi::OsString;
#[derive(Clone, Debug)]
struct FakeSessionHost {
    result: Result<SessionSnapshot, SessionOperationalFailure>,
    calls: Cell<usize>,
    requested_ids: RefCell<Vec<String>>,
    pending: bool,
}

impl FakeSessionHost {
    fn ready(result: Result<SessionSnapshot, SessionOperationalFailure>) -> Self {
        Self {
            result,
            calls: Cell::new(0),
            requested_ids: RefCell::new(Vec::new()),
            pending: false,
        }
    }

    fn pending() -> Self {
        Self {
            result: Err(SessionOperationalFailure::Unavailable),
            calls: Cell::new(0),
            requested_ids: RefCell::new(Vec::new()),
            pending: true,
        }
    }
}

impl SessionCommandHost for FakeSessionHost {
    fn inspect_session(
        &self,
        id: machine_god_core::SessionId,
    ) -> BoxFuture<'static, Result<SessionSnapshot, SessionOperationalFailure>> {
        self.calls.set(self.calls.get() + 1);
        self.requested_ids.borrow_mut().push(id.as_str().to_owned());
        if self.pending {
            Box::pin(std::future::pending())
        } else {
            Box::pin(std::future::ready(self.result.clone()))
        }
    }
}

fn session_snapshot(id: &str) -> SessionSnapshot {
    SessionSnapshot {
        id: id.to_owned(),
        incarnation_id: format!("incarnation-{id}"),
        revision: 7,
        next_turn_sequence: 4,
        message_count: 3,
        metadata_entry_count: 2,
    }
}

#[test]
fn session_human_output_is_exact_and_requests_the_validated_id_once() {
    let host = FakeSessionHost::ready(Ok(session_snapshot("alpha")));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [OsString::from("session"), OsString::from("alpha")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            session: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        concat!(
            "[session] alpha\n",
            " - incarnation_id: incarnation-alpha\n",
            " - revision: 7\n",
            " - next_turn_sequence: 4\n",
            " - message_count: 3\n",
            " - metadata_entry_count: 2\n",
        )
        .as_bytes()
    );
    assert!(stderr.is_empty());
    assert_eq!(host.calls.get(), 1);
    assert_eq!(&*host.requested_ids.borrow(), &["alpha"]);
}

#[test]
fn session_json_output_has_exact_shape_key_order_and_lf() {
    let host = FakeSessionHost::ready(Ok(session_snapshot("alpha")));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from("--json"),
        ],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            session: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        b"{\"kind\":\"session\",\"id\":\"alpha\",\"incarnation_id\":\"incarnation-alpha\",\"revision\":7,\"next_turn_sequence\":4,\"message_count\":3,\"metadata_entry_count\":2}\n"
    );
    assert!(stderr.is_empty());
    assert_eq!(host.calls.get(), 1);
    assert_eq!(&*host.requested_ids.borrow(), &["alpha"]);
}

#[test]
fn native_session_inspection_errors_collapse_to_the_frozen_cli_categories() {
    for (kind, expected) in [
        (
            NativeSessionInspectionErrorKind::UnsupportedPlatform,
            SessionOperationalFailure::Unsupported,
        ),
        (
            NativeSessionInspectionErrorKind::InvalidEnvironment,
            SessionOperationalFailure::Unavailable,
        ),
        (
            NativeSessionInspectionErrorKind::UnsafeStateRoot,
            SessionOperationalFailure::Unavailable,
        ),
        (
            NativeSessionInspectionErrorKind::NotFound,
            SessionOperationalFailure::NotFound,
        ),
        (
            NativeSessionInspectionErrorKind::Corrupt,
            SessionOperationalFailure::Corrupt,
        ),
        (
            NativeSessionInspectionErrorKind::Unavailable,
            SessionOperationalFailure::Unavailable,
        ),
    ] {
        assert_eq!(classify_session_inspection_error_kind(kind), expected);
    }
}

#[test]
fn session_failures_use_exact_human_and_json_channels() {
    for failure in [
        SessionOperationalFailure::NotFound,
        SessionOperationalFailure::Corrupt,
        SessionOperationalFailure::Unavailable,
        SessionOperationalFailure::Unsupported,
        SessionOperationalFailure::ResourceLimit,
    ] {
        let category = failure.category();
        let host = FakeSessionHost::ready(Err(failure));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            [OsString::from("session"), OsString::from("alpha")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                session: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            format!("machine-god session: could not inspect session: {category}\n").as_bytes()
        );
        assert_eq!(host.calls.get(), 1);

        let host = FakeSessionHost::ready(Err(failure));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            [
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
            ],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                session: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert_eq!(
            stdout,
            format!(
                "{{\"kind\":\"session\",\"error\":\"could not inspect session: {category}\",\"code\":\"{category}\"}}\n"
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
    }
}

#[test]
fn pending_session_inspection_is_polled_once_and_maps_to_unavailable() {
    let host = FakeSessionHost::pending();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [OsString::from("session"), OsString::from("alpha")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            session: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 1);
    assert!(stdout.is_empty());
    assert_eq!(
        stderr,
        b"machine-god session: could not inspect session: Unavailable\n"
    );
    assert_eq!(host.calls.get(), 1);
}

#[test]
fn invalid_session_arguments_are_rejected_before_host_effects() {
    for arguments in [
        vec![OsString::from("session")],
        vec![OsString::from("session"), OsString::from("last")],
        vec![OsString::from("session"), OsString::from("--id")],
        vec![
            OsString::from("session"),
            OsString::from("--id"),
            OsString::from("alpha"),
        ],
        vec![OsString::from("session"), OsString::from("--json")],
        vec![
            OsString::from("session"),
            OsString::from("--json"),
            OsString::from("alpha"),
        ],
        vec![
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from("--json=true"),
        ],
        vec![
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from("--json"),
            OsString::from("extra"),
        ],
        vec![OsString::from("session"), OsString::from("bad/session")],
    ] {
        let host = FakeSessionHost::ready(Err(SessionOperationalFailure::Corrupt));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            arguments,
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                session: &host,
                ..Default::default()
            },
        );

        assert_eq!(exit, 2);
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
        assert_eq!(host.calls.get(), 0);
        assert!(host.requested_ids.borrow().is_empty());
    }
}

#[test]
fn session_snapshot_invariants_fail_closed_as_resource_limit() {
    let mut invalid_id = session_snapshot("alpha");
    invalid_id.id = "bad/session".to_owned();
    let mut invalid_incarnation = session_snapshot("alpha");
    invalid_incarnation.incarnation_id = "bad incarnation".to_owned();
    let mut zero_revision = session_snapshot("alpha");
    zero_revision.revision = 0;
    let mut zero_allocator = session_snapshot("alpha");
    zero_allocator.next_turn_sequence = 0;
    let wrong_id = session_snapshot("beta");

    for snapshot in [
        invalid_id,
        invalid_incarnation,
        zero_revision,
        zero_allocator,
        wrong_id,
    ] {
        for json in [false, true] {
            let host = FakeSessionHost::ready(Ok(snapshot.clone()));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let arguments = if json {
                vec![
                    OsString::from("session"),
                    OsString::from("alpha"),
                    OsString::from("--json"),
                ]
            } else {
                vec![OsString::from("session"), OsString::from("alpha")]
            };
            let exit = run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    session: &host,
                    ..Default::default()
                },
            );

            assert_eq!(exit, 1);
            if json {
                assert_eq!(
                    stdout,
                    b"{\"kind\":\"session\",\"error\":\"could not inspect session: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"
                );
                assert!(stderr.is_empty());
            } else {
                assert!(stdout.is_empty());
                assert_eq!(
                    stderr,
                    b"machine-god session: could not inspect session: ResourceLimit\n"
                );
            }
            assert_eq!(host.calls.get(), 1);
        }
    }
}

#[test]
fn maximal_session_snapshot_stays_within_the_output_cap() {
    let snapshot = SessionSnapshot {
        id: "a".repeat(128),
        incarnation_id: "b".repeat(128),
        revision: u64::MAX,
        next_turn_sequence: u64::MAX,
        message_count: usize::MAX,
        metadata_entry_count: usize::MAX,
    };

    for json in [false, true] {
        let output = render_session(&snapshot, json).expect("valid maximum snapshot renders");
        assert!(output.len() <= MAX_SESSION_OUTPUT_BYTES);
        assert!(output.ends_with('\n'));
    }
}

#[test]
fn session_output_cap_is_inclusive() {
    let mut output = BoundedOutput::with_capacity(MAX_SESSION_OUTPUT_BYTES, 512);
    output
        .write_str(&"x".repeat(MAX_SESSION_OUTPUT_BYTES))
        .expect("inclusive output limit is accepted");
    assert!(output.write_char('x').is_err());
    assert_eq!(output.finish().len(), MAX_SESSION_OUTPUT_BYTES);
}

#[test]
fn session_broken_zero_progress_and_partial_stdout_use_output_diagnostic() {
    for json in [false, true] {
        let arguments = if json {
            vec![
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
            ]
        } else {
            vec![OsString::from("session"), OsString::from("alpha")]
        };
        let snapshot = session_snapshot("alpha");

        let host = FakeSessionHost::ready(Ok(snapshot.clone()));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            arguments.clone(),
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                session: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

        let host = FakeSessionHost::ready(Ok(snapshot.clone()));
        let mut stdout = ZeroProgressWriter::default();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            arguments.clone(),
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                session: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert!(stdout.captured.is_empty());
        assert_eq!(stdout.calls, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

        let complete = render_session(&snapshot, json).expect("snapshot renders");
        let host = FakeSessionHost::ready(Ok(snapshot));
        let mut stdout = PartialThenBrokenWriter::new(5);
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            arguments,
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                session: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert!(!stdout.prefix.is_empty());
        assert_eq!(stdout.prefix, complete.as_bytes()[..stdout.prefix.len()]);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
    }
}

#[test]
fn session_json_failure_write_errors_use_output_diagnostic() {
    let host = FakeSessionHost::ready(Err(SessionOperationalFailure::Corrupt));
    let mut stdout = BrokenWriter;
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from("--json"),
        ],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            session: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 1);
    assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
    assert_eq!(host.calls.get(), 1);
}

#[test]
fn session_human_failure_write_errors_use_output_diagnostic() {
    let host = FakeSessionHost::ready(Err(SessionOperationalFailure::Corrupt));
    let mut stdout = Vec::new();
    let mut stderr = FirstWriteFailsThenCaptures::default();
    let exit = run_with_hosts(
        [OsString::from("session"), OsString::from("alpha")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            session: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 1);
    assert!(stdout.is_empty());
    assert_eq!(stderr.captured, OUTPUT_FAILURE.as_bytes());
    assert_eq!(host.calls.get(), 1);
}
