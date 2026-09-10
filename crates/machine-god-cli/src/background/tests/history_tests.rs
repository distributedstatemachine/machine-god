use super::*;

fn terminal_id(value: u64) -> TerminalSessionId {
    TerminalSessionId::new(format!("terminal-{value:032x}")).unwrap()
}

fn terminal_detail() -> BackgroundTerminalSnapshot {
    BackgroundTerminalSnapshot {
        id: terminal_id(7),
        state: "exited".into(),
        created_at_ms: 10,
        last_output_ms: 20,
        command: Some("printf '\u{1b}[red'\n\u{202e}".into()),
        cwd: "/tmp/quoted-\"".into(),
        exit_code: Some(7),
        signal: None,
        earliest: TerminalCursor::new(1, 0).unwrap(),
        latest: TerminalCursor::new(1, 32).unwrap(),
        facts_cursor: TerminalCursor::new(1, 32).unwrap(),
    }
}

fn terminal_row(value: u64, timestamp: i128) -> BackgroundHistoryRecordSnapshot {
    BackgroundHistoryRecordSnapshot {
        id: NativeBackgroundHistoryId::Terminal(terminal_id(value)),
        state: "running".into(),
        updated_at_ms: timestamp,
        command_preview: "terminal\u{1b}\n\u{202e}".into(),
        preview_truncated: false,
    }
}

fn legacy_row(value: u64, timestamp: i128) -> BackgroundHistoryRecordSnapshot {
    BackgroundHistoryRecordSnapshot {
        id: NativeBackgroundHistoryId::Legacy(value),
        state: "running".into(),
        updated_at_ms: timestamp,
        command_preview: "legacy".into(),
        preview_truncated: false,
    }
}

#[test]
fn canonical_terminal_targets_preserve_identity_in_both_flag_positions() {
    let id = terminal_id(7);
    for args in [
        vec![id.as_str()],
        vec!["--json", id.as_str()],
        vec![id.as_str(), "--json"],
    ] {
        let host = FakeHost::ready(Ok(BackgroundSnapshot::TerminalDetail(terminal_detail())));
        assert_eq!(invoke(&host, &args).0, 0);
        assert_eq!(
            *host.queries.borrow(),
            vec![NativeBackgroundHistoryQuery::Terminal(id.clone())]
        );
        assert_eq!(host.polls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn malformed_terminal_targets_and_live_commands_are_rejected_before_effects() {
    let host = FakeHost::ready(Ok(empty_list()));
    for target in [
        "terminal-",
        "terminal-0000000000000000000000000000000",
        "terminal-000000000000000000000000000000000",
        "terminal-0000000000000000000000000000000A",
        "terminal-0000000000000000000000000000000g",
        "terminal-0000000000000000000000000000000\n",
        "000000000000000000000",
        "stop",
        "open",
        "logs",
        "list",
        "terminal/00000000000000000000000000000000",
    ] {
        let (exit, stdout, stderr) = invoke(&host, &[target, "--json"]);
        assert_eq!(exit, 2, "{target:?}");
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID.as_bytes());
    }
    assert_eq!(host.calls.get(), 0);
}

#[test]
fn union_json_retains_string_terminal_ids_and_lossless_numeric_timestamps() {
    let snapshot = BackgroundSnapshot::HistoryList(BackgroundHistoryListSnapshot {
        records: vec![
            legacy_row(7, i128::from(u64::MAX)),
            terminal_row(7, i128::from(i64::MAX)),
            legacy_row(8, i128::from(i64::MAX)),
        ],
        truncated: false,
    });
    for args in [&["--json"][..], &[][..]] {
        let (exit, stdout, stderr) = invoke(&FakeHost::ready(Ok(snapshot.clone())), args);
        assert_eq!(exit, 0);
        assert!(stderr.is_empty());
        assert!(!stdout.contains(&0x1b));
        assert!(!String::from_utf8_lossy(&stdout).contains('\u{202e}'));
        if !args.is_empty() {
            let json: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
            assert_eq!(json["records"][0]["id"], 7);
            assert_eq!(json["records"][0]["updated_at_ms"], u64::MAX);
            assert_eq!(json["records"][1]["id"], terminal_id(7).as_str());
        }
    }
}

#[test]
fn legacy_only_union_rendering_is_unchanged() {
    let old = BackgroundSnapshot::List(BackgroundListSnapshot {
        records: vec![BackgroundRecordSnapshot {
            id: 7,
            state: "running".into(),
            updated_at_ms: 20,
            command_preview: "legacy".into(),
            preview_truncated: false,
        }],
        truncated: true,
    });
    let new = BackgroundSnapshot::HistoryList(BackgroundHistoryListSnapshot {
        records: vec![legacy_row(7, 20)],
        truncated: true,
    });
    for args in [&[][..], &["--json"][..]] {
        assert_eq!(
            invoke(&FakeHost::ready(Ok(old.clone())), args),
            invoke(&FakeHost::ready(Ok(new.clone())), args)
        );
    }
}

#[test]
fn terminal_details_are_recorded_only_and_render_nullable_fields_safely() {
    for state in ["starting", "running", "exited", "lost", "closed"] {
        let mut detail = terminal_detail();
        detail.state = state.into();
        detail.command = None;
        detail.exit_code = None;
        detail.signal = (state == "exited").then_some(9);
        for args in [&["last"][..], &["last", "--json"][..]] {
            let (exit, stdout, stderr) = invoke(
                &FakeHost::ready(Ok(BackgroundSnapshot::TerminalDetail(detail.clone()))),
                args,
            );
            assert_eq!(exit, 0);
            assert!(stderr.is_empty());
            let text = String::from_utf8(stdout).unwrap();
            assert!(!text.contains("pid"));
            assert!(!text.contains("server_url"));
            if args.len() == 2 {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(json["kind"], "background_terminal_detail");
                assert_eq!(json["recorded_only"], true);
                assert!(json["command"].is_null());
                assert_eq!(json["latest"]["offset"], 32);
            }
        }
    }
}

#[test]
fn terminal_detail_invariants_fail_before_success_output() {
    let mut cases = Vec::new();
    let base = terminal_detail();
    let mut add = |mutate: fn(&mut BackgroundTerminalSnapshot)| {
        let mut value = base.clone();
        mutate(&mut value);
        cases.push(value);
    };
    add(|v| v.id = TerminalSessionId::new("alias").unwrap());
    add(|v| v.state = "stale".into());
    add(|v| v.created_at_ms = -1);
    add(|v| v.last_output_ms = 9);
    add(|v| v.exit_code = Some(256));
    add(|v| v.signal = Some(9));
    add(|v| {
        v.exit_code = None;
        v.signal = Some(0);
    });
    add(|v| v.exit_code = None);
    add(|v| v.state = "lost".into());
    add(|v| v.state = "running".into());
    add(|v| v.earliest = TerminalCursor::new(2, 0).unwrap());
    add(|v| v.facts_cursor = TerminalCursor::new(2, 0).unwrap());
    add(|v| v.command = Some(String::new()));
    add(|v| v.command = Some("a".repeat(MAX_BACKGROUND_COMMAND_BYTES + 1)));
    add(|v| v.cwd = "relative".into());
    for detail in cases {
        let (exit, stdout, _) = invoke(
            &FakeHost::ready(Ok(BackgroundSnapshot::TerminalDetail(detail))),
            &["last"],
        );
        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
    }
}

#[test]
fn mixed_list_rejects_bad_order_duplicates_counts_and_timestamps() {
    let cases = vec![
        vec![legacy_row(1, 20), terminal_row(1, 20)],
        vec![terminal_row(1, 20), terminal_row(1, 10)],
        vec![terminal_row(1, -1)],
        vec![terminal_row(1, i128::from(i64::MAX) + 1)],
        vec![legacy_row(1, i128::from(u64::MAX) + 1)],
        (1..=129)
            .rev()
            .map(|id| terminal_row(id, i128::from(id)))
            .collect(),
        (1..=101)
            .rev()
            .map(|id| legacy_row(id, i128::from(id)))
            .collect(),
    ];
    for records in cases {
        let (exit, stdout, _) = invoke(
            &FakeHost::ready(Ok(BackgroundSnapshot::HistoryList(
                BackgroundHistoryListSnapshot {
                    records,
                    truncated: false,
                },
            ))),
            &[],
        );
        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
    }
}

#[test]
fn maximum_union_and_terminal_detail_fit_the_render_bound() {
    let mut records = (1..=128)
        .rev()
        .map(|id| terminal_row(id, 1000))
        .chain((1..=100).rev().map(|id| legacy_row(id, 0)))
        .collect::<Vec<_>>();
    for row in &mut records {
        row.command_preview = "\u{1b}".repeat(MAX_BACKGROUND_COMMAND_PREVIEW_BYTES);
        row.preview_truncated = true;
    }
    let mut terminal = terminal_detail();
    terminal.command = Some("\u{1b}".repeat(MAX_BACKGROUND_COMMAND_BYTES));
    for (snapshot, target) in [
        (
            BackgroundSnapshot::HistoryList(BackgroundHistoryListSnapshot {
                records,
                truncated: false,
            }),
            None,
        ),
        (BackgroundSnapshot::TerminalDetail(terminal), Some("last")),
    ] {
        for json in [false, true] {
            let mut args = target.into_iter().collect::<Vec<_>>();
            if json {
                args.push("--json");
            }
            let (exit, stdout, stderr) = invoke(&FakeHost::ready(Ok(snapshot.clone())), &args);
            assert_eq!(exit, 0);
            assert!(stderr.is_empty());
            assert!(stdout.len() <= MAX_BACKGROUND_OUTPUT_BYTES);
            assert!(!stdout.contains(&0x1b));
        }
    }
}

#[test]
fn exact_queries_never_alias_the_other_identity_domain() {
    let terminal = BackgroundSnapshot::TerminalDetail(terminal_detail());
    assert_eq!(invoke(&FakeHost::ready(Ok(terminal)), &["7"]).0, 1);
    assert_eq!(
        invoke(&FakeHost::ready(Ok(detail())), &[terminal_id(7).as_str()]).0,
        1
    );
}
