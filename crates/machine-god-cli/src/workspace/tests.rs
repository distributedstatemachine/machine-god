use super::*;
use std::cell::Cell;

struct Host {
    result: Result<WorkspaceSnapshot, WorkspaceOperationalFailure>,
    calls: Cell<usize>,
}
impl WorkspaceCommandHost for Host {
    fn execute_workspace(
        &self,
        _: &WorkspaceAction,
    ) -> Result<WorkspaceSnapshot, WorkspaceOperationalFailure> {
        self.calls.set(self.calls.get() + 1);
        self.result.clone()
    }
}
fn snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        primary: "/workspace".into(),
        generation: 7,
        saved_suppressed: false,
        entries: vec![],
        saved_changed: Some(false),
        runtime_changed: Some(false),
        reconciliation: Reconciliation::Refreshed,
        launch_flag_can_restore: false,
    }
}
fn options(json: bool) -> WorkspaceOptions {
    WorkspaceOptions {
        action: WorkspaceAction::List,
        json,
    }
}

#[test]
fn workspace_grammar_accepts_actions_and_singleton_json_in_every_position() {
    for (args, action) in [
        (vec![], WorkspaceAction::List),
        (vec!["list"], WorkspaceAction::List),
        (vec!["clear"], WorkspaceAction::Clear),
        (
            vec!["add", "directory"],
            WorkspaceAction::Add("directory".into()),
        ),
        (
            vec!["remove", "directory"],
            WorkspaceAction::Remove("directory".into()),
        ),
    ] {
        let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
        assert_eq!(
            parse_options(args.clone()).unwrap(),
            WorkspaceOptions {
                action: action.clone(),
                json: false
            }
        );
        for position in 0..=args.len() {
            let mut args = args.clone();
            args.insert(position, "--json".into());
            assert_eq!(
                parse_options(args).unwrap(),
                WorkspaceOptions {
                    action: action.clone(),
                    json: true
                }
            );
        }
    }
}

#[test]
fn workspace_invalid_grammar_precedes_host_effects() {
    for args in [
        vec!["add"],
        vec!["remove"],
        vec!["add", "--json"],
        vec!["add", "--flag"],
        vec!["clear", "extra"],
        vec!["--json", "--json"],
        vec!["list", "list"],
        vec!["--json=true"],
        vec!["add", ""],
        vec!["--", "list"],
    ] {
        let host = Host {
            result: Err(WorkspaceOperationalFailure::Unavailable),
            calls: Cell::new(0),
        };
        let arguments = std::iter::once(OsString::from("workspace"))
            .chain(args.into_iter().map(OsString::from));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            crate::run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    workspace: &host,
                    ..Default::default()
                }
            ),
            2
        );
        assert_eq!(host.calls.get(), 0);
        assert!(stdout.is_empty());
        assert_eq!(stderr, crate::INVALID_ARGUMENTS.as_bytes());
    }
}

#[test]
fn workspace_path_bounds_are_checked_without_effects() {
    assert!(parse_options(["add".into(), OsString::from("x".repeat(4097))]).is_err());
    assert!(parse_options(["add".into(), OsString::from("with\0nul")]).is_err());
    assert_eq!(
        parse_options(["add".into(), "./-name".into()])
            .unwrap()
            .action,
        WorkspaceAction::Add("./-name".into())
    );
    assert!(parse_options(["add".into(), OsString::from("x".repeat(4096))]).is_ok());
}

#[cfg(unix)]
#[test]
fn workspace_parser_and_renderer_preserve_non_unicode_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let operand = OsString::from_vec(vec![b'/', 0xff, b'x']);
    assert_eq!(
        parse_options(["add".into(), operand.clone()])
            .unwrap()
            .action,
        WorkspaceAction::Add(operand.clone().into())
    );
    let mut value = snapshot();
    value.primary = operand.into();
    let json: serde_json::Value =
        serde_json::from_str(&render(&value, &options(true)).unwrap()).unwrap();
    assert!(json["primary_directory"]["text"].is_null());
    assert_eq!(json["primary_directory"]["bytes_hex"], "2fff78");
    assert!(
        render(&value, &options(false))
            .unwrap()
            .contains("bytes_hex=\"2fff78\"")
    );
}

#[test]
fn workspace_receipts_escape_paths_and_preserve_uncertain_facts() {
    let mut value = snapshot();
    value.primary = "/quoted-\"-\u{1b}-\u{202e}-\n".into();
    value.saved_changed = None;
    value.runtime_changed = Some(true);
    value.launch_flag_can_restore = true;
    for reconciliation in [
        Reconciliation::AmbiguousIntended,
        Reconciliation::AmbiguousBefore,
        Reconciliation::Indeterminate,
        Reconciliation::ReloadFailed(WorkspaceOperationalFailure::InvalidConfiguration),
    ] {
        value.reconciliation = reconciliation;
        for json in [false, true] {
            let host = Host {
                result: Ok(value.clone()),
                calls: Cell::new(0),
            };
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_workspace(&host, &options(json), &mut stdout, &mut stderr),
                1
            );
            let text = String::from_utf8(stdout).unwrap();
            assert!(stderr.is_empty());
            assert!(!text.contains('\u{1b}'));
            assert!(!text.contains('\u{202e}'));
            assert!(text.contains("\\u001b"));
            assert!(text.contains("\\u202e"));
            if json {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert!(value["saved_changed"].is_null());
                assert_eq!(value["runtime_changed"], true);
                assert_eq!(value["launch_flag_can_restore"], true);
            }
        }
    }
}

#[test]
fn workspace_maximum_cardinality_and_escaped_paths_fit_atomic_output_bound() {
    let mut value = snapshot();
    let path = PathBuf::from(format!("/{}", "\u{7f}".repeat(4095)));
    value.primary.clone_from(&path);
    value.entries = (0..16)
        .map(|_| Entry {
            source: path.clone(),
            identity: path.clone(),
            identity_canonical: true,
            provenance: Provenance {
                saved: true,
                launch: false,
            },
            availability: Availability {
                available: true,
                active: true,
            },
        })
        .collect();
    for json in [false, true] {
        let output = render(&value, &options(json)).unwrap();
        assert!(output.len() <= MAX_OUTPUT_BYTES);
        assert!(output.ends_with('\n'));
    }
    value.entries.push(value.entries[0].clone());
    assert_eq!(
        render(&value, &options(true)),
        Err(WorkspaceOperationalFailure::ResourceLimit)
    );
    let mut output = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 0);
    output.write_str(&"x".repeat(MAX_OUTPUT_BYTES)).unwrap();
    assert!(output.write_str("x").is_err());
    assert_eq!(output.finish().len(), MAX_OUTPUT_BYTES);
}

struct Broken;
impl io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn workspace_errors_use_exact_redacted_output_channels() {
    use WorkspaceOperationalFailure as Failure;
    for (failure, category) in [
        (Failure::Busy, "Busy"),
        (Failure::InvalidPath, "InvalidPath"),
        (Failure::UnknownDirectory, "UnknownDirectory"),
        (Failure::ResourceLimit, "ResourceLimit"),
        (Failure::DuplicateRoot, "DuplicateRoot"),
        (Failure::OverlappingState, "OverlappingState"),
        (Failure::Conflict, "Conflict"),
        (Failure::InvalidConfiguration, "InvalidConfiguration"),
        (Failure::UnsafePath, "UnsafePath"),
        (Failure::Persistence, "Persistence"),
        (Failure::Ambiguous, "Ambiguous"),
        (Failure::Unavailable, "Unavailable"),
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        (Failure::Unsupported, "Unsupported"),
    ] {
        for json in [false, true] {
            let host = Host {
                result: Err(failure),
                calls: Cell::new(0),
            };
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_workspace(&host, &options(json), &mut stdout, &mut stderr),
                1
            );
            if json {
                assert_eq!(stdout, format!("{{\"kind\":\"workspace\",\"action\":\"list\",\"error\":\"workspace operation failed\",\"code\":\"{category}\"}}\n").as_bytes());
                assert!(stderr.is_empty());
            } else {
                assert!(stdout.is_empty());
                assert_eq!(
                    stderr,
                    format!("machine-god workspace: operation failed: {category}\n").as_bytes()
                );
            }
            assert_eq!(host.calls.get(), 1);
        }
    }
}

#[test]
fn workspace_output_failures_use_global_diagnostic_after_host_settlement() {
    for result in [
        Ok(snapshot()),
        Err(WorkspaceOperationalFailure::Unavailable),
    ] {
        let host = Host {
            result,
            calls: Cell::new(0),
        };
        let mut stderr = Vec::new();
        assert_eq!(
            run_workspace(&host, &options(true), &mut Broken, &mut stderr),
            1
        );
        assert_eq!(host.calls.get(), 1);
        assert_eq!(stderr, crate::OUTPUT_FAILURE.as_bytes());
    }
}

#[test]
fn workspace_confirmed_and_cached_receipts_have_success_exit() {
    for reconciliation in [
        Reconciliation::Confirmed,
        Reconciliation::Refreshed,
        Reconciliation::CachedBusy,
    ] {
        let mut value = snapshot();
        value.reconciliation = reconciliation;
        let host = Host {
            result: Ok(value),
            calls: Cell::new(0),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_workspace(&host, &options(true), &mut stdout, &mut stderr),
            0
        );
        assert!(stderr.is_empty());
    }
}
