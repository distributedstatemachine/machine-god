use super::*;
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Host {
    result: Result<SessionsSnapshot, SessionsOperationalFailure>,
    calls: RefCell<Vec<SessionsOptions>>,
    pending: bool,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl Host {
    fn ready(result: Result<SessionsSnapshot, SessionsOperationalFailure>) -> Self {
        Self {
            result,
            calls: RefCell::new(Vec::new()),
            pending: false,
            polls: Arc::default(),
            drops: Arc::default(),
        }
    }
}
impl SessionsCommandHost for Host {
    fn list_sessions(
        &self,
        options: &SessionsOptions,
    ) -> BoxFuture<'static, Result<SessionsSnapshot, SessionsOperationalFailure>> {
        struct DropCount(Arc<AtomicUsize>);
        impl Drop for DropCount {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        self.calls.borrow_mut().push(options.clone());
        let guard = DropCount(Arc::clone(&self.drops));
        let result = self.result.clone();
        let pending = self.pending;
        let polls = Arc::clone(&self.polls);
        Box::pin(async move {
            let _guard = guard;
            std::future::poll_fn(move |_| {
                polls.fetch_add(1, Ordering::SeqCst);
                if pending {
                    Poll::Pending
                } else {
                    Poll::Ready(result.clone())
                }
            })
            .await
        })
    }
}
fn run(args: &[&str], host: &Host) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code =
        crate::run_with_sessions_host(args.iter().map(OsString::from), &mut out, &mut err, host);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}
fn row(id: &str, updated: Option<i64>) -> SessionRow {
    SessionRow {
        id: id.into(),
        title: None,
        preview: None,
        workspace: None,
        workspace_hex: None,
        origin_workspace: None,
        origin_workspace_hex: None,
        created: None,
        updated,
        history_len: 0,
        language: None,
    }
}
fn snapshot(entries: Vec<SessionRow>) -> SessionsSnapshot {
    SessionsSnapshot {
        entries,
        ..SessionsSnapshot::default()
    }
}
fn json_options() -> SessionsOptions {
    SessionsOptions {
        json: true,
        ..SessionsOptions::default()
    }
}
fn cursor(value: &str) -> NativeSessionCatalogCursor {
    NativeSessionCatalogCursor::parse(value).unwrap()
}

#[test]
fn grammar_any_order_and_default_scope_are_forwarded_once() {
    let host = Host::ready(Ok(SessionsSnapshot::default()));
    assert_eq!(
        run(&["sessions"], &host),
        (0, "[sessions] no saved sessions\n".into(), String::new())
    );
    assert_eq!(host.calls.borrow()[0], SessionsOptions::default());
    for args in [
        vec![
            "sessions",
            "--all",
            "--limit",
            "2",
            "--cursor",
            "v1:unknown:alpha:part",
            "--json",
        ],
        vec![
            "sessions",
            "--json",
            "--cursor",
            "v1:unknown:alpha:part",
            "--limit",
            "2",
            "--all",
        ],
        vec![
            "sessions",
            "--limit",
            "2",
            "--json",
            "--all",
            "--cursor",
            "v1:unknown:alpha:part",
        ],
    ] {
        assert_eq!(run(&args, &host).0, 0);
        let options = host.calls.borrow().last().unwrap().clone();
        assert!(options.all && options.json);
        assert_eq!(options.limit, 2);
        assert_eq!(options.cursor.unwrap().to_string(), "v1:unknown:alpha:part");
    }
    assert_eq!(host.polls.load(Ordering::SeqCst), 4);
    assert_eq!(host.drops.load(Ordering::SeqCst), 4);
}

#[test]
fn invalid_grammar_is_rejected_before_any_host_effect() {
    for tail in [
        vec!["--all", "--all"],
        vec!["--json", "--json"],
        vec!["--limit"],
        vec!["--limit", "0"],
        vec!["--limit", "101"],
        vec!["--limit", "+1"],
        vec!["--limit", "-1"],
        vec!["--limit", "1_0"],
        vec!["--limit", "999999999999999999999999999"],
        vec!["--limit", "1", "--limit", "2"],
        vec!["--cursor"],
        vec!["--cursor", "v1:01:alpha"],
        vec!["--cursor", "v1:0:../secret"],
        vec!["--cursor", "v1:0:alpha", "--cursor", "v1:1:beta"],
        vec!["--json=true"],
        vec!["--all", "extra"],
        vec!["--"],
    ] {
        let host = Host::ready(Ok(SessionsSnapshot::default()));
        let args = [vec!["sessions"], tail].concat();
        assert_eq!(
            run(&args, &host),
            (2, String::new(), crate::INVALID_ARGUMENTS.into()),
            "{args:?}"
        );
        assert!(host.calls.borrow().is_empty());
    }
}

#[test]
#[cfg(unix)]
fn non_unicode_flag_values_fail_before_effects() {
    use std::os::unix::ffi::OsStringExt as _;
    for flag in [None, Some("--cursor"), Some("--limit")] {
        let mut args = vec![OsString::from("sessions")];
        args.extend(flag.map(OsString::from));
        args.push(OsString::from_vec(vec![0xff]));
        let host = Host::ready(Ok(SessionsSnapshot::default()));
        assert_eq!(
            crate::run_with_sessions_host(args, &mut Vec::new(), &mut Vec::new(), &host),
            2
        );
        assert!(host.calls.borrow().is_empty());
    }
}

#[test]
fn exact_empty_and_unknown_rich_json() {
    assert_eq!(
        render_sessions(&SessionsSnapshot::default(), &json_options()).unwrap(),
        "{\"kind\":\"sessions\",\"count\":0,\"sessions\":[]}\n"
    );
    assert_eq!(
        render_sessions(&snapshot(vec![row("alpha", None)]), &json_options()).unwrap(),
        concat!(
            "{\"kind\":\"sessions\",\"count\":1,\"sessions\":[{\"id\":\"alpha\",\"title\":\"Untitled session\",",
            "\"preview\":null,\"workspace_root\":null,\"origin_workspace_root\":null,\"created_at_ms\":null,",
            "\"updated_at_ms\":null,\"history_len\":0,\"conversation_language\":null}]}\n"
        )
    );
}

#[test]
fn rich_human_time_language_and_terminal_controls() {
    let mut entry = row("alpha", Some(951_827_696_789));
    entry.title = Some("fix\u{85}\u{202e}\n\"\\".into());
    entry.history_len = 1;
    entry.language = Some("en-US".into());
    let page = snapshot(vec![entry]);
    assert_eq!(
        render_sessions(&page, &SessionsOptions::default()).unwrap(),
        "[sessions] 1 saved\n - fix\\u0085\\u202e\\n\\\"\\\\\n   id=alpha | 1 turn | English | updated 2000-02-29 12:34:56.789 UTC\n"
    );
    let json: serde_json::Value =
        serde_json::from_str(&render_sessions(&page, &json_options()).unwrap()).unwrap();
    assert_eq!(json["sessions"][0]["title"], "fix\u{85}\u{202e}\n\"\\");
    assert_eq!(language_label("und"), None);
    assert_eq!(language_label("UND-hAnI"), Some("Han script"));
    assert_eq!(language_label("custom"), Some("custom"));
}

#[test]
fn timestamp_boundaries_and_gregorian_centuries() {
    for (time, expected) in [
        (None, "unknown"),
        (Some(-1), "unknown"),
        (Some(i64::MAX), "unknown"),
        (Some(0), "1970-01-01 00:00:00.000 UTC"),
        (Some(253_402_300_799_999), "9999-12-31 23:59:59.999 UTC"),
        (Some(4_107_542_400_000), "2100-03-01 00:00:00.000 UTC"),
    ] {
        let mut out = String::new();
        write_timestamp(&mut out, time).unwrap();
        assert_eq!(out, expected);
    }
}

#[test]
fn continuation_preserves_scope_limit_and_native_order() {
    let mut page = snapshot(vec![
        row("z", Some(5)),
        row("a", Some(5)),
        row("unknown", None),
    ]);
    page.next_cursor = Some(cursor("v1:unknown:unknown"));
    let options = SessionsOptions {
        all: true,
        limit: 3,
        ..SessionsOptions::default()
    };
    let output = render_sessions(&page, &options).unwrap();
    assert!(output.contains("machine-god sessions --all --limit 3 --cursor v1:unknown:unknown"));
    let json: serde_json::Value = serde_json::from_str(
        &render_sessions(
            &page,
            &SessionsOptions {
                json: true,
                ..options
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(json["has_more"], true);
    assert_eq!(json["next_cursor"], "v1:unknown:unknown");
    assert_eq!(json["sessions"][0]["id"], "z");
    page.entries.swap(0, 1);
    assert_eq!(
        render_sessions(&page, &json_options()),
        Err(SessionsOperationalFailure::ResourceLimit)
    );
}

#[test]
fn incomplete_and_skipped_pages_are_not_false_empty_or_false_cursors() {
    let page = SessionsSnapshot {
        incomplete: true,
        skipped_invalid: 2,
        ..SessionsSnapshot::default()
    };
    let output = render_sessions(&page, &SessionsOptions::default()).unwrap();
    assert!(output.starts_with("[sessions] 0 saved\n"));
    assert!(!output.contains("no readable saved sessions"));
    assert!(output.contains("listing incomplete"));
    assert!(output.contains("skipped 2 unreadable saved sessions; run `machine-god doctor`"));
    let json: serde_json::Value =
        serde_json::from_str(&render_sessions(&page, &json_options()).unwrap()).unwrap();
    assert_eq!(json["scan_complete"], false);
    assert_eq!(json["skipped_invalid"], 2);
    assert!(json.get("next_cursor").is_none() && json.get("has_more").is_none());
    let page = SessionsSnapshot {
        incomplete: true,
        ..SessionsSnapshot::default()
    };
    assert_eq!(
        render_sessions(&page, &SessionsOptions::default()).unwrap(),
        "[sessions] 0 saved\n[sessions] listing incomplete: a resource limit was reached\n"
    );
    let page = SessionsSnapshot {
        skipped_invalid: 1,
        ..SessionsSnapshot::default()
    };
    assert_eq!(
        render_sessions(&page, &SessionsOptions::default()).unwrap(),
        "[sessions] no readable saved sessions\n[sessions] warning: skipped 1 unreadable saved session; run `machine-god doctor` for recovery guidance\n"
    );
}

#[test]
fn snapshot_bounds_fail_before_output_and_debug_is_redacted() {
    let mut entry = row("secret-marker", None);
    entry.title = Some("secret-marker".repeat(30));
    let page = snapshot(vec![entry]);
    assert!(!format!("{page:?}").contains("secret-marker"));
    let host = Host::ready(Ok(page));
    assert_eq!(run(&["sessions", "--json"], &host), (1, "{\"kind\":\"sessions\",\"error\":\"could not list sessions: ResourceLimit\",\"code\":\"ResourceLimit\"}\n".into(), String::new()));
    let mut bad = snapshot(vec![row("a", None)]);
    bad.next_cursor = Some(cursor("v1:unknown:b"));
    assert!(render_sessions(&bad, &json_options()).is_err());
    bad.next_cursor = Some(cursor("v1:unknown:a"));
    bad.incomplete = true;
    assert!(render_sessions(&bad, &json_options()).is_err());
}

#[test]
fn maximum_escaped_rows_fit_and_cap_is_inclusive() {
    let entries = (0..100)
        .rev()
        .map(|index| {
            let mut entry = row(&format!("{index:03}{}", "a".repeat(125)), None);
            entry.title = Some("\u{1}".repeat(240));
            entry.preview = Some("\u{1}".repeat(240));
            entry.workspace = Some("\u{1}".repeat(4096));
            entry.origin_workspace = Some("\u{1}".repeat(4096));
            entry.language = Some("\u{1}".repeat(24));
            entry.history_len = usize::MAX;
            entry
        })
        .collect();
    let page = snapshot(entries);
    for options in [SessionsOptions::default(), json_options()] {
        let out = render_sessions(&page, &options).unwrap();
        assert!(out.len() <= MAX_OUTPUT_BYTES);
    }
    let mut out = BoundedOutput(String::new());
    out.write_str(&"x".repeat(MAX_OUTPUT_BYTES)).unwrap();
    assert!(out.write_char('x').is_err());
    assert_eq!(out.0.len(), MAX_OUTPUT_BYTES);
}

#[test]
fn non_utf8_workspace_has_exact_hex_not_lossy_replacement() {
    let mut entry = row("a", None);
    entry.workspace_hex = Some("2f776f726b2fff".into());
    entry.origin_workspace_hex = Some("2f6f726967696e2ffe".into());
    let json: serde_json::Value =
        serde_json::from_str(&render_sessions(&snapshot(vec![entry]), &json_options()).unwrap())
            .unwrap();
    assert_eq!(
        json["sessions"][0]["workspace_root"],
        serde_json::Value::Null
    );
    assert_eq!(json["sessions"][0]["workspace_root_hex"], "2f776f726b2fff");
    assert!(json["sessions"][0]["origin_workspace_root"].is_null());
    assert_eq!(
        json["sessions"][0]["origin_workspace_root_hex"],
        "2f6f726967696e2ffe"
    );
}

#[test]
fn origin_is_independent_of_current_workspace_and_bounded_before_output() {
    let mut entry = row("a", None);
    entry.workspace = Some("/current".into());
    entry.origin_workspace = Some("/original\u{202e}".into());
    let output = render_sessions(&snapshot(vec![entry.clone()]), &json_options()).unwrap();
    assert!(!output.contains('\u{202e}'));
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["sessions"][0]["workspace_root"], "/current");
    assert_eq!(
        json["sessions"][0]["origin_workspace_root"],
        "/original\u{202e}"
    );
    entry.origin_workspace = None;
    let output = render_sessions(&snapshot(vec![entry.clone()]), &json_options()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(json["sessions"][0]["origin_workspace_root"].is_null());
    for invalid in ["g0".into(), "ffFf".into(), "0".into(), "ff".repeat(4097)] {
        entry.origin_workspace_hex = Some(invalid);
        assert!(render_sessions(&snapshot(vec![entry.clone()]), &json_options()).is_err());
    }
    entry.origin_workspace_hex = Some("2f".into());
    entry.origin_workspace = Some("/known".into());
    assert!(render_sessions(&snapshot(vec![entry.clone()]), &json_options()).is_err());
    entry.origin_workspace_hex = None;
    entry.origin_workspace = Some("x".repeat(4097));
    assert!(render_sessions(&snapshot(vec![entry]), &json_options()).is_err());
}

#[test]
fn pending_future_is_polled_once_and_dropped() {
    let mut host = Host::ready(Ok(SessionsSnapshot::default()));
    host.pending = true;
    assert_eq!(
        run(&["sessions"], &host),
        (
            1,
            String::new(),
            "machine-god sessions: could not list sessions: Unavailable\n".into()
        )
    );
    assert_eq!(host.polls.load(Ordering::SeqCst), 1);
    assert_eq!(host.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn errors_are_fixed_and_channel_specific() {
    for failure in [
        SessionsOperationalFailure::Corrupt,
        SessionsOperationalFailure::Unavailable,
        SessionsOperationalFailure::Unsupported,
        SessionsOperationalFailure::ResourceLimit,
    ] {
        let host = Host::ready(Err(failure));
        let category = failure.category();
        assert_eq!(
            run(&["sessions"], &host),
            (
                1,
                String::new(),
                format!("machine-god sessions: could not list sessions: {category}\n")
            )
        );
        assert_eq!(
            run(&["sessions", "--json"], &host),
            (
                1,
                format!(
                    "{{\"kind\":\"sessions\",\"error\":\"could not list sessions: {category}\",\"code\":\"{category}\"}}\n"
                ),
                String::new()
            )
        );
    }
}

#[test]
fn output_failures_report_standard_diagnostic() {
    struct Fails {
        remaining: usize,
        bytes: Vec<u8>,
        zero: bool,
    }
    impl io::Write for Fails {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return if self.zero {
                    Ok(0)
                } else {
                    Err(io::Error::other("private-writer-detail"))
                };
            }
            let count = bytes.len().min(self.remaining);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.remaining -= count;
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    for remaining in [0, 7] {
        for zero in [false, true] {
            for json in [false, true] {
                let host = Host::ready(Ok(snapshot(vec![row("a", None)])));
                let mut out = Fails {
                    remaining,
                    bytes: Vec::new(),
                    zero,
                };
                let mut err = Vec::new();
                assert_eq!(
                    run_sessions(
                        &host,
                        &SessionsOptions {
                            json,
                            ..SessionsOptions::default()
                        },
                        &mut out,
                        &mut err
                    ),
                    1
                );
                assert_eq!(err, crate::OUTPUT_FAILURE.as_bytes());
                assert_eq!(out.bytes.len(), remaining);
            }
        }
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_error_categories_remain_redacted() {
    for kind in [
        NativeSessionCatalogErrorKind::InvalidEnvironment,
        NativeSessionCatalogErrorKind::UnsafeStateRoot,
        NativeSessionCatalogErrorKind::Unavailable,
    ] {
        assert_eq!(
            classify_error(kind),
            SessionsOperationalFailure::Unavailable
        );
    }
    assert_eq!(
        classify_error(NativeSessionCatalogErrorKind::Corrupt),
        SessionsOperationalFailure::Corrupt
    );
}

#[test]
fn failed_error_writes_use_output_diagnostic_without_raw_writer_detail() {
    #[derive(Default)]
    struct FirstWriteFails {
        failed: bool,
        captured: Vec<u8>,
    }
    impl io::Write for FirstWriteFails {
        fn write(&mut self, value: &[u8]) -> io::Result<usize> {
            if !self.failed {
                self.failed = true;
                return Err(io::Error::other("secret-writer-detail"));
            }
            self.captured.extend_from_slice(value);
            Ok(value.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let host = Host::ready(Err(SessionsOperationalFailure::Unavailable));
    let mut out = Vec::new();
    let mut err = FirstWriteFails::default();
    assert_eq!(
        run_sessions(&host, &SessionsOptions::default(), &mut out, &mut err),
        1
    );
    assert!(out.is_empty());
    assert_eq!(err.captured, crate::OUTPUT_FAILURE.as_bytes());
    let mut out = FirstWriteFails::default();
    let mut err = Vec::new();
    assert_eq!(run_sessions(&host, &json_options(), &mut out, &mut err), 1);
    assert!(out.captured.is_empty());
    assert_eq!(err, crate::OUTPUT_FAILURE.as_bytes());
}
