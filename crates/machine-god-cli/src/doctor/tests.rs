use super::*;
use crate::test_support::*;
use crate::{INVALID_ARGUMENTS, OUTPUT_FAILURE, run_with_hosts};
use std::cell::Cell;
use std::ffi::OsString;
#[derive(Clone, Debug)]
struct FakeDoctorHost {
    result: Result<DoctorReportSnapshot, ()>,
    calls: Cell<usize>,
}

impl FakeDoctorHost {
    fn new(result: Result<DoctorReportSnapshot, ()>) -> Self {
        Self {
            result,
            calls: Cell::new(0),
        }
    }
}

impl DoctorCommandHost for FakeDoctorHost {
    fn inspect_doctor(&self) -> Result<DoctorReportSnapshot, ()> {
        self.calls.set(self.calls.get() + 1);
        self.result
    }
}

fn doctor_report(checks: [DoctorCheckSnapshot; super::DOCTOR_CHECK_COUNT]) -> DoctorReportSnapshot {
    let ok_count = checks
        .iter()
        .filter(|check| check.status == DoctorCheckStatus::Ok)
        .count();
    let warn_count = checks
        .iter()
        .filter(|check| check.status == DoctorCheckStatus::Warn)
        .count();
    let fail_count = checks
        .iter()
        .filter(|check| check.status == DoctorCheckStatus::Fail)
        .count();
    DoctorReportSnapshot {
        ok_count,
        warn_count,
        fail_count,
        checks,
    }
}

fn check(
    name: &'static str,
    status: DoctorCheckStatus,
    detail: &'static str,
) -> DoctorCheckSnapshot {
    DoctorCheckSnapshot {
        name,
        status,
        detail,
    }
}

fn leaked(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

#[test]
fn doctor_human_output_is_exact_and_fail_findings_exit_zero() {
    let report = doctor_report([
        check(
            "configuration",
            DoctorCheckStatus::Ok,
            "configuration loaded",
        ),
        check(
            "credentials",
            DoctorCheckStatus::Warn,
            "credential is missing",
        ),
        check(
            "state",
            DoctorCheckStatus::Fail,
            "state directory is unavailable",
        ),
        check(
            "workspace",
            DoctorCheckStatus::Ok,
            "working directory is available",
        ),
    ]);
    let host = FakeDoctorHost::new(Ok(report));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = run_with_hosts(
        [OsString::from("doctor")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            doctor: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        concat!(
            "[doctor] ok=2 warn=1 fail=1\n",
            "[ok] configuration: configuration loaded\n",
            "[warn] credentials: credential is missing\n",
            "[fail] state: state directory is unavailable\n",
            "[ok] workspace: working directory is available\n",
        )
        .as_bytes()
    );
    assert!(stderr.is_empty());
    assert_eq!(host.calls.get(), 1);
}

#[test]
fn doctor_json_output_has_exact_shape_order_escaping_and_lf() {
    let report = doctor_report([
        check(
            "config\"uration",
            DoctorCheckStatus::Ok,
            "loaded\\ready\nnext",
        ),
        check(
            "cred\u{1b}",
            DoctorCheckStatus::Warn,
            "missing\u{2028}credential",
        ),
        check("state", DoctorCheckStatus::Fail, "not available"),
        check("workspace", DoctorCheckStatus::Ok, "café"),
    ]);
    let host = FakeDoctorHost::new(Ok(report));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = run_with_hosts(
        [OsString::from("doctor"), OsString::from("--json")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            doctor: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        concat!(
            "{\"kind\":\"doctor\",\"ok_count\":2,\"warn_count\":1,\"fail_count\":1,\"checks\":[",
            "{\"name\":\"config\\\"uration\",\"status\":\"ok\",\"detail\":\"loaded\\\\ready\\nnext\"},",
            "{\"name\":\"cred\\u001b\",\"status\":\"warn\",\"detail\":\"missing\\u2028credential\"},",
            "{\"name\":\"state\",\"status\":\"fail\",\"detail\":\"not available\"},",
            "{\"name\":\"workspace\",\"status\":\"ok\",\"detail\":\"café\"}]}",
            "\n",
        )
        .as_bytes()
    );
    assert!(stderr.is_empty());
    assert_eq!(host.calls.get(), 1);
}

#[test]
fn invalid_doctor_arguments_are_rejected_before_host_effects() {
    for arguments in [
        vec![OsString::from("doctor"), OsString::from("extra")],
        vec![
            OsString::from("doctor"),
            OsString::from("--json"),
            OsString::from("extra"),
        ],
    ] {
        let host = FakeDoctorHost::new(Err(()));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            arguments,
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                doctor: &host,
                ..Default::default()
            },
        );

        assert_eq!(exit, 2);
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
        assert_eq!(host.calls.get(), 0);
    }
}

#[test]
fn doctor_output_cap_and_invalid_report_fail_before_stdout() {
    let oversized = doctor_report([
        check(
            "configuration",
            DoctorCheckStatus::Ok,
            leaked("x".repeat(MAX_DOCTOR_OUTPUT_BYTES)),
        ),
        check("credentials", DoctorCheckStatus::Ok, "available"),
        check("state", DoctorCheckStatus::Ok, "available"),
        check("workspace", DoctorCheckStatus::Ok, "available"),
    ]);
    let mut invalid = oversized;
    invalid.fail_count = 1;

    for report in [oversized, invalid] {
        for json in [false, true] {
            let host = FakeDoctorHost::new(Ok(report));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let arguments = if json {
                vec![OsString::from("doctor"), OsString::from("--json")]
            } else {
                vec![OsString::from("doctor")]
            };

            let exit = run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    doctor: &host,
                    ..Default::default()
                },
            );

            assert_eq!(exit, 1);
            assert!(stdout.is_empty());
            assert_eq!(stderr, DOCTOR_RENDER_FAILURE.as_bytes());
            assert_eq!(host.calls.get(), 1);
        }
    }
}

#[test]
fn doctor_render_cap_is_inclusive() {
    let baseline = doctor_report([
        check("configuration", DoctorCheckStatus::Ok, ""),
        check("credentials", DoctorCheckStatus::Ok, "available"),
        check("state", DoctorCheckStatus::Ok, "available"),
        check("workspace", DoctorCheckStatus::Ok, "available"),
    ]);
    let baseline_len = render_doctor(&baseline, false)
        .expect("baseline report renders")
        .len();
    let report = doctor_report([
        check(
            "configuration",
            DoctorCheckStatus::Ok,
            leaked("x".repeat(MAX_DOCTOR_OUTPUT_BYTES - baseline_len)),
        ),
        check("credentials", DoctorCheckStatus::Ok, "available"),
        check("state", DoctorCheckStatus::Ok, "available"),
        check("workspace", DoctorCheckStatus::Ok, "available"),
    ]);

    let output = render_doctor(&report, false).expect("inclusive limit is accepted");

    assert_eq!(output.len(), MAX_DOCTOR_OUTPUT_BYTES);
}

#[test]
fn doctor_broken_stdout_uses_fixed_output_diagnostic() {
    let report = doctor_report([
        check("configuration", DoctorCheckStatus::Ok, "loaded"),
        check("credentials", DoctorCheckStatus::Warn, "missing"),
        check("state", DoctorCheckStatus::Ok, "available"),
        check("workspace", DoctorCheckStatus::Ok, "available"),
    ]);
    for json in [false, true] {
        let host = FakeDoctorHost::new(Ok(report));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let arguments = if json {
            vec![OsString::from("doctor"), OsString::from("--json")]
        } else {
            vec![OsString::from("doctor")]
        };

        let exit = run_with_hosts(
            arguments,
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                doctor: &host,
                ..Default::default()
            },
        );

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        assert_eq!(host.calls.get(), 1);
    }
}

#[test]
fn doctor_zero_progress_stdout_uses_fixed_output_diagnostic() {
    let report = doctor_report([
        check("configuration", DoctorCheckStatus::Ok, "loaded"),
        check("credentials", DoctorCheckStatus::Warn, "missing"),
        check("state", DoctorCheckStatus::Ok, "available"),
        check("workspace", DoctorCheckStatus::Ok, "available"),
    ]);
    for json in [false, true] {
        let host = FakeDoctorHost::new(Ok(report));
        let mut stdout = ZeroProgressWriter::default();
        let mut stderr = Vec::new();
        let arguments = if json {
            vec![OsString::from("doctor"), OsString::from("--json")]
        } else {
            vec![OsString::from("doctor")]
        };

        let exit = run_with_hosts(
            arguments,
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                doctor: &host,
                ..Default::default()
            },
        );

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        assert!(stdout.captured.is_empty());
        assert_eq!(stdout.calls, 1);
        assert_eq!(host.calls.get(), 1);
    }
}

#[test]
fn doctor_partial_stdout_failure_uses_fixed_output_diagnostic() {
    let report = doctor_report([
        check("configuration", DoctorCheckStatus::Ok, "loaded"),
        check("credentials", DoctorCheckStatus::Warn, "missing"),
        check("state", DoctorCheckStatus::Ok, "available"),
        check("workspace", DoctorCheckStatus::Ok, "available"),
    ]);
    for json in [false, true] {
        let complete = render_doctor(&report, json).expect("valid report renders");
        let host = FakeDoctorHost::new(Ok(report));
        let mut stdout = PartialThenBrokenWriter::new(7);
        let mut stderr = Vec::new();
        let arguments = if json {
            vec![OsString::from("doctor"), OsString::from("--json")]
        } else {
            vec![OsString::from("doctor")]
        };

        let exit = run_with_hosts(
            arguments,
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                doctor: &host,
                ..Default::default()
            },
        );

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        assert!(!stdout.prefix.is_empty());
        assert!(stdout.prefix.len() < complete.len());
        assert_eq!(stdout.prefix, complete.as_bytes()[..stdout.prefix.len()]);
        assert_eq!(host.calls.get(), 1);
    }
}
