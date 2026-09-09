use super::*;
use crate::ask::{AskCommandExecution, AskCommandOutcome, MAX_ASK_PROMPT_BYTES, SessionSelection};
use crate::background::{BackgroundOperationalFailure, BackgroundSnapshot};
use crate::test_support::*;
use machine_god_core::BoxFuture;
use machine_god_native::{
    AI_GATEWAY_DEFAULT_MODEL, NativeBackgroundQuery, NativeRuntimeCredentialEnvironment,
    NativeRuntimeStatus, NativeRuntimeStatusError, NativeRuntimeStatusInput, PermissionMode,
    inspect_native_runtime_status,
};
use std::cell::{Cell, RefCell};

#[derive(Debug)]
struct FakeStatusHost {
    calls: Cell<usize>,
    status: Result<NativeRuntimeStatus, NativeRuntimeStatusError>,
}

impl FakeStatusHost {
    fn unavailable() -> Self {
        Self {
            calls: Cell::new(0),
            status: inspect_native_runtime_status(NativeRuntimeStatusInput::new(
                AI_GATEWAY_DEFAULT_MODEL,
                PermissionMode::Ask,
                NativeRuntimeCredentialEnvironment::new(None, None),
                "/workspace",
                None,
            )),
        }
    }
}

impl StatusCommandHost for FakeStatusHost {
    fn inspect_status(&self) -> Result<NativeRuntimeStatus, NativeRuntimeStatusError> {
        self.calls.set(self.calls.get() + 1);
        self.status.clone()
    }
}

#[derive(Debug, Default)]
struct FakeBackgroundHost {
    calls: Cell<usize>,
}

impl BackgroundCommandHost for FakeBackgroundHost {
    fn inspect_background(
        &self,
        _query: NativeBackgroundQuery,
    ) -> BoxFuture<'static, Result<BackgroundSnapshot, BackgroundOperationalFailure>> {
        self.calls.set(self.calls.get() + 1);
        Box::pin(std::future::pending())
    }
}

#[derive(Debug)]
struct FakeAskHost {
    launches: RefCell<Vec<crate::workspace::launch::LaunchWorkspaceOptions>>,
    recordings: RefCell<Vec<bool>>,
    outcome: AskCommandOutcome,
    calls: Cell<usize>,
    selections: RefCell<Vec<Option<String>>>,
    prompts: RefCell<Vec<String>>,
    output: &'static [u8],
}

impl FakeAskHost {
    fn new(outcome: AskCommandOutcome, output: &'static [u8]) -> Self {
        Self {
            launches: RefCell::new(Vec::new()),
            recordings: RefCell::new(Vec::new()),
            outcome,
            calls: Cell::new(0),
            selections: RefCell::new(Vec::new()),
            prompts: RefCell::new(Vec::new()),
            output,
        }
    }
}

impl AskCommandHost for FakeAskHost {
    fn with_launch(
        &self,
        options: crate::workspace::launch::LaunchWorkspaceOptions,
        record_requested: bool,
    ) -> Result<Box<dyn AskCommandHost + '_>, ()> {
        self.launches.borrow_mut().push(options);
        self.recordings.borrow_mut().push(record_requested);
        Ok(Box::new(self))
    }
    fn execute_stdin(&self, _output: &mut dyn io::Write) -> AskCommandExecution {
        self.calls.set(self.calls.get() + 1);
        self.prompts.borrow_mut().push("<stdin>".into());
        AskCommandExecution::without_finalizer(self.outcome)
    }
    fn execute_interactive(
        &self,
        selection: InteractiveSessionSelection,
        output: &mut dyn io::Write,
    ) -> AskCommandExecution {
        self.calls.set(self.calls.get() + 1);
        self.selections.borrow_mut().push(match selection {
            InteractiveSessionSelection::Fresh => None,
            InteractiveSessionSelection::Picker => Some("<picker>".into()),
            InteractiveSessionSelection::Latest => Some("last".into()),
            InteractiveSessionSelection::Exact(id) => Some(id.as_str().into()),
        });
        let outcome = if output.write_all(self.output).is_err() {
            AskCommandOutcome::OutputFailure
        } else {
            self.outcome
        };
        AskCommandExecution::without_finalizer(outcome)
    }
    fn execute(
        &self,
        selection: SessionSelection,
        prompt: String,
        output: &mut dyn io::Write,
    ) -> AskCommandExecution {
        self.calls.set(self.calls.get() + 1);
        self.selections.borrow_mut().push(match selection {
            SessionSelection::CreateGenerated => None,
            SessionSelection::Resume(id) => Some(id.as_str().to_owned()),
        });
        self.prompts.borrow_mut().push(prompt);
        let outcome = if output.write_all(self.output).is_err() {
            AskCommandOutcome::OutputFailure
        } else {
            self.outcome
        };
        AskCommandExecution::without_finalizer(outcome)
    }
}

impl AskCommandHost for &FakeAskHost {
    fn execute_stdin(&self, output: &mut dyn io::Write) -> AskCommandExecution {
        FakeAskHost::execute_stdin(self, output)
    }
    fn execute_interactive(
        &self,
        selection: InteractiveSessionSelection,
        output: &mut dyn io::Write,
    ) -> AskCommandExecution {
        FakeAskHost::execute_interactive(self, selection, output)
    }
    fn execute(
        &self,
        selection: SessionSelection,
        prompt: String,
        output: &mut dyn io::Write,
    ) -> AskCommandExecution {
        FakeAskHost::execute(self, selection, prompt, output)
    }
}

#[test]
fn launch_workspace_modifiers_reach_only_validated_conversation_hosts() {
    for suffix in [
        vec![],
        vec!["-r"],
        vec!["--continue"],
        vec!["ask", "prompt"],
        vec!["ask"],
        vec!["resume", "session-id", "prompt"],
        vec!["session", "resume", "session-id"],
    ] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"selected\n");
        let args = ["--add-dir", "shared one", "--no-additional-dirs"]
            .into_iter()
            .chain(suffix)
            .map(OsString::from);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                args,
                &mut stdout,
                &mut stderr,
                CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            0
        );
        assert_eq!(host.calls.get(), 1);
        assert_eq!(
            host.launches.borrow().as_slice(),
            &[crate::workspace::launch::LaunchWorkspaceOptions {
                directories: vec!["shared one".into()],
                suppress_saved: true
            }]
        );
        assert!(stderr.is_empty());
    }
    for args in [
        vec!["--add-dir"],
        vec!["--no-additional-dirs", "--no-additional-dirs"],
        vec!["--add-dir=one", "status"],
        vec!["--add-dir=one", "workspace"],
        vec!["--add-dir=one", "session", "session-id"],
        vec!["--add-dir=one", "ask", "--bad"],
        vec!["--add-dir=one", "--resume", "id", "extra"],
        vec!["ask", "--add-dir=one"],
    ] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"not reached");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                args.into_iter().map(OsString::from),
                &mut stdout,
                &mut stderr,
                CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            2
        );
        assert_eq!(host.calls.get(), 0);
        assert!(host.launches.borrow().is_empty());
        assert!(stdout.is_empty());
    }
}

#[test]
fn recording_modifier_reaches_only_validated_interactive_hosts() {
    for args in [
        vec!["--record"],
        vec!["--resume", "--record"],
        vec!["session", "resume", "--record"],
        vec!["resume", "saved", "--record"],
        vec![
            "--add-dir",
            "shared",
            "--no-additional-dirs",
            "-r",
            "--record",
        ],
    ] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"interactive\n");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                args.into_iter().map(OsString::from),
                &mut stdout,
                &mut stderr,
                CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            0
        );
        assert_eq!(host.calls.get(), 1);
        assert_eq!(*host.recordings.borrow(), vec![true]);
        assert_eq!(stdout, b"interactive\n");
        assert!(stderr.is_empty());
    }
    for args in [
        vec!["ask", "prompt", "--record"],
        vec!["resume", "saved", "prompt", "--record"],
        vec!["--add-dir=shared", "--record", "resume"],
        vec!["--record", "--record"],
    ] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"not reached");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                args.into_iter().map(OsString::from),
                &mut stdout,
                &mut stderr,
                CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            2
        );
        assert_eq!(host.calls.get(), 0);
        assert!(host.recordings.borrow().is_empty());
        assert!(host.launches.borrow().is_empty());
        assert!(stdout.is_empty());
    }
}

#[test]
fn parser_accepts_only_the_documented_grammar() {
    assert_eq!(
        parse_arguments([]),
        Ok(Command::Interactive {
            selection: InteractiveSessionSelection::Fresh
        })
    );
    for alias in ["help", "--help", "-h"] {
        assert_eq!(parse_arguments([OsString::from(alias)]), Ok(Command::Help));
        assert_eq!(
            parse_arguments([
                OsString::from(alias),
                OsString::from("ignored"),
                OsString::from("--json"),
            ]),
            Ok(Command::Help)
        );
    }
    for alias in ["--version", "-V"] {
        assert_eq!(
            parse_arguments([OsString::from(alias)]),
            Ok(Command::Identity)
        );
    }
    assert_eq!(
        parse_arguments([OsString::from("doctor")]),
        Ok(Command::Doctor { json: false })
    );
    assert_eq!(
        parse_arguments([OsString::from("doctor"), OsString::from("--json")]),
        Ok(Command::Doctor { json: true })
    );
    assert_eq!(
        parse_arguments([OsString::from("models")]),
        Ok(Command::Models { json: false })
    );
    assert_eq!(
        parse_arguments([OsString::from("models"), OsString::from("--json")]),
        Ok(Command::Models { json: true })
    );
    assert_eq!(
        parse_arguments([OsString::from("permissions")]),
        Ok(Command::Permissions { json: false })
    );
    assert_eq!(
        parse_arguments([OsString::from("permissions"), OsString::from("--json"),]),
        Ok(Command::Permissions { json: true })
    );
    assert_eq!(
        parse_arguments([OsString::from("sessions")]),
        Ok(Command::Sessions {
            options: SessionsOptions::default()
        })
    );
    assert_eq!(
        parse_arguments([OsString::from("sessions"), OsString::from("--json")]),
        Ok(Command::Sessions {
            options: SessionsOptions {
                json: true,
                ..SessionsOptions::default()
            }
        })
    );
    for arguments in [
        vec![OsString::from("unknown")],
        vec![OsString::from("--json"), OsString::from("status")],
        vec![OsString::from("doctor"), OsString::from("--json=true")],
        vec![
            OsString::from("doctor"),
            OsString::from("--json"),
            OsString::from("--json"),
        ],
        vec![OsString::from("doctor"), OsString::from("extra")],
        vec![OsString::from("models"), OsString::from("--json=true")],
        vec![
            OsString::from("models"),
            OsString::from("--json"),
            OsString::from("--json"),
        ],
        vec![OsString::from("permissions"), OsString::from("--json=true")],
        vec![
            OsString::from("permissions"),
            OsString::from("--json"),
            OsString::from("--json"),
        ],
        vec![OsString::from("sessions"), OsString::from("--json=true")],
        vec![
            OsString::from("sessions"),
            OsString::from("--json"),
            OsString::from("extra"),
        ],
    ] {
        assert_eq!(parse_arguments(arguments), Err(()));
    }
}

#[test]
fn named_hosts_are_inert_and_dispatch_to_the_exact_selected_dependency() {
    let ask = FakeAskHost::new(AskCommandOutcome::Completed, b"selected host\n");
    let status = FakeStatusHost::unavailable();
    let hosts = CommandHosts {
        ask: &ask,
        status: &status,
        ..Default::default()
    };
    assert_eq!(ask.calls.get(), 0);
    assert_eq!(status.calls.get(), 0);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_with_hosts(
            [OsString::from("ask"), OsString::from("hello")],
            &mut stdout,
            &mut stderr,
            hosts,
        ),
        0
    );
    assert_eq!(ask.calls.get(), 1);
    assert_eq!(status.calls.get(), 0);
    assert_eq!(stdout, b"selected host\n");
    assert!(stderr.is_empty());
}

#[test]
fn global_help_first_token_ignores_every_tail_without_status_effects() {
    let host = FakeStatusHost::unavailable();
    for arguments in [
        vec![OsString::from("help"), OsString::from("status")],
        vec![OsString::from("--help"), OsString::from("--json")],
        vec![OsString::from("-h"), OsString::from("unknown")],
    ] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    status: &host,
                    ..Default::default()
                }
            ),
            0
        );
        assert_eq!(stdout, help().as_bytes());
        assert!(stderr.is_empty());
    }
    assert_eq!(host.calls.get(), 0);
}

#[test]
fn main_dispatches_background_through_the_injected_host_only_after_validation() {
    let host = FakeBackgroundHost::default();
    for arguments in [
        vec![OsString::from("help"), OsString::from("background")],
        vec![
            OsString::from("background"),
            OsString::from("last"),
            OsString::from("last"),
        ],
    ] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let expected_exit = if arguments[0] == "help" { 0 } else { 2 };
        assert_eq!(
            run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    background: &host,
                    ..Default::default()
                }
            ),
            expected_exit
        );
    }
    assert_eq!(host.calls.get(), 0);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_with_hosts(
            [OsString::from("background"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                background: &host,
                ..Default::default()
            }
        ),
        1
    );
    assert_eq!(host.calls.get(), 1);
    assert_eq!(
        stdout,
        b"{\"kind\":\"background\",\"error\":\"could not inspect background history: Unavailable\",\"code\":\"Unavailable\"}\n"
    );
    assert!(stderr.is_empty());
}

#[test]
fn main_dispatches_status_through_the_injected_host_only_after_validation() {
    let host = FakeStatusHost::unavailable();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_with_hosts(
            [OsString::from("status"), OsString::from("unknown")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                status: &host,
                ..Default::default()
            }
        ),
        1
    );
    assert!(stdout.is_empty());
    assert_eq!(stderr, b"usage: machine-god status [--json]\n");
    assert_eq!(host.calls.get(), 0);

    stdout.clear();
    stderr.clear();
    assert_eq!(
        run_with_hosts(
            [
                OsString::from("status"),
                OsString::from("--json"),
                OsString::from("--json"),
            ],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                status: &host,
                ..Default::default()
            }
        ),
        0
    );
    assert!(stdout.starts_with(b"{\"kind\":\"status\""));
    assert!(stderr.is_empty());
    assert_eq!(host.calls.get(), 1);
}

#[test]
fn global_help_output_failure_is_fixed_without_status_effects() {
    let host = FakeStatusHost::unavailable();
    let mut stdout = BrokenWriter;
    let mut stderr = Vec::new();
    assert_eq!(
        run_with_hosts(
            [OsString::from("help"), OsString::from("status")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                status: &host,
                ..Default::default()
            }
        ),
        1
    );
    assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
    assert_eq!(host.calls.get(), 0);
}

#[test]
fn ask_parser_accepts_only_the_documented_top_level_grammar() {
    for arguments in [vec![OsString::from("ask")], vec!["ask".into(), "--".into()]] {
        assert_eq!(parse_arguments(arguments), Ok(Command::AskStdin));
    }
    assert_eq!(
        parse_arguments([
            OsString::from("ask"),
            OsString::from("hello"),
            OsString::from("世界"),
        ]),
        Ok(Command::Ask {
            prompt: "hello 世界".to_owned(),
        })
    );
    assert_eq!(
        parse_arguments([
            OsString::from("ask"),
            OsString::from("--"),
            OsString::from("--flag"),
        ]),
        Ok(Command::Ask {
            prompt: "--flag".to_owned(),
        })
    );

    for arguments in [
        vec![OsString::from("ask"), OsString::from("--flag")],
        vec![OsString::from("ask"), OsString::from(" \t\r\n")],
        vec![
            OsString::from("ask"),
            OsString::from("hello"),
            OsString::from("--"),
        ],
    ] {
        assert_eq!(parse_arguments(arguments), Err(()));
    }
}

#[test]
fn session_parser_accepts_only_the_documented_grammar() {
    assert_eq!(
        parse_arguments([OsString::from("session"), OsString::from("alpha")]),
        Ok(Command::Session {
            id: machine_god_core::SessionId::new("alpha").unwrap(),
            json: false,
        })
    );
    assert_eq!(
        parse_arguments([
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from("--json"),
        ]),
        Ok(Command::Session {
            id: machine_god_core::SessionId::new("alpha").unwrap(),
            json: true,
        })
    );
    assert_eq!(
        parse_arguments([OsString::from("session"), OsString::from("--flag")]),
        Ok(Command::Session {
            id: machine_god_core::SessionId::new("--flag").unwrap(),
            json: false,
        })
    );

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
            OsString::from("--json"),
        ],
        vec![
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from("--json"),
            OsString::from("extra"),
        ],
        vec![OsString::from("session"), OsString::from("bad/session")],
        vec![OsString::from("session"), OsString::from("café")],
        vec![OsString::from("session"), OsString::from("a".repeat(129))],
    ] {
        assert_eq!(parse_arguments(arguments), Err(()));
    }
}

#[test]
fn resume_parser_accepts_only_an_explicit_id_and_bounded_prompt() {
    assert_eq!(
        parse_arguments([
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("hello"),
            OsString::from("世界"),
        ]),
        Ok(Command::Resume {
            id: machine_god_core::SessionId::new("alpha").unwrap(),
            prompt: "hello 世界".to_owned(),
        })
    );
    assert_eq!(
        parse_arguments([
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("--"),
            OsString::from("--flag"),
        ]),
        Ok(Command::Resume {
            id: machine_god_core::SessionId::new("alpha").unwrap(),
            prompt: "--flag".to_owned(),
        })
    );

    let oversized_prompt = "x".repeat(MAX_ASK_PROMPT_BYTES + 1);
    for arguments in [
        vec![
            OsString::from("resume"),
            OsString::from("last"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("--id"),
            OsString::from("alpha"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("--json"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("--flag"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("--"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("--flag"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from(" \t\r\n"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("nul\0prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("bad/session"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("café"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("a".repeat(129)),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from(oversized_prompt),
        ],
    ] {
        assert_eq!(parse_arguments(arguments), Err(()));
    }
}

#[test]
fn resume_aliases_select_typed_targets_and_dispatch_once_without_a_prompt() {
    use InteractiveSessionSelection::{Exact, Latest, Picker};
    let exact = |id| Exact(machine_god_core::SessionId::new(id).unwrap());
    for (arguments, expected) in [
        (vec!["-r"], Picker),
        (vec!["-c"], Latest),
        (vec!["--continue"], Latest),
        (vec!["--resume-last"], Latest),
        (vec!["--resume"], Latest),
        (vec!["--resume", " \tlast\r\n"], Latest),
        (vec!["--resume", " \talpha\r\n"], exact("alpha")),
        (vec!["--resume-alpha"], exact("alpha")),
        (vec!["--resume- last "], exact("last")),
        (vec!["resume", " \talpha\r\n"], exact("alpha")),
        (vec!["resume", "--id", "last"], exact("last")),
        (vec!["resume", "--id", " \talpha\r\n"], exact("alpha")),
        (vec!["resume", "--resume", "--last"], Latest),
        (vec!["session", "resume"], Latest),
        (vec!["session", "resume", " \tlast\r\n"], Latest),
        (vec!["session", "resume", "alpha"], exact("alpha")),
        (vec!["session", "resume", "--id", "last"], exact("last")),
        (vec!["session", "resume", "--resume", "--last"], Latest),
    ] {
        assert_eq!(
            parse_arguments(arguments.iter().map(OsString::from)),
            Ok(Command::Interactive {
                selection: expected.clone()
            }),
            "{arguments:?}"
        );
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"interactive\n");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                arguments.iter().map(OsString::from),
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            0,
            "{arguments:?}"
        );
        assert_eq!(host.calls.get(), 1);
        assert!(host.prompts.borrow().is_empty());
        let target = match expected {
            Picker => "<picker>".to_owned(),
            Latest => "last".to_owned(),
            Exact(id) => id.as_str().to_owned(),
            InteractiveSessionSelection::Fresh => panic!("resume must not select fresh"),
        };
        assert_eq!(*host.selections.borrow(), vec![Some(target)]);
        assert_eq!(stdout, b"interactive\n");
        assert!(stderr.is_empty());
    }
}

#[test]
fn resume_aliases_reject_malformed_targets_and_tails_before_host_effects() {
    for arguments in [
        vec!["-r", "alpha"],
        vec!["-c", "alpha"],
        vec!["--continue", "alpha"],
        vec!["--resume-last", "alpha"],
        vec!["--resume-alpha", "prompt"],
        vec!["--resume-"],
        vec!["--resume- \t\r\n"],
        vec!["--resume--flag"],
        vec!["--resume", "alpha", "prompt"],
        vec!["--resume", "--id", "alpha"],
        vec!["--resume", " \t\r\n"],
        vec!["--resume", "\u{a0}alpha\u{a0}"],
        vec!["--resume", "bad/session"],
        vec!["resume", "--id"],
        vec!["resume", "--id", ""],
        vec!["resume", "--id", "--record"],
        vec!["resume", "--id", "last", "prompt"],
        vec!["resume", "--resume"],
        vec!["resume", "--resume", "last"],
        vec!["resume", "--resume", "--last", "extra"],
        vec!["session", "resume", "alpha", "prompt"],
        vec!["session", "resume", "--id", "last", "prompt"],
        vec!["session", "resume", "last", "--json"],
    ] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"must not run");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            parse_arguments(arguments.iter().map(OsString::from)),
            Err(()),
            "{arguments:?}"
        );
        assert_eq!(
            run_with_hosts(
                arguments.iter().map(OsString::from),
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            2,
            "{arguments:?}"
        );
        assert_eq!(host.calls.get(), 0);
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
    }
}

#[test]
fn interactive_startup_selects_fresh_latest_or_exact_without_inventing_a_prompt() {
    for (arguments, expected) in [
        (vec![], None),
        (vec!["resume"], Some("last")),
        (vec!["resume", "last"], Some("last")),
        (vec!["resume", "alpha"], Some("alpha")),
    ] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"interactive\n");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            arguments.into_iter().map(OsString::from),
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                ask: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 0);
        assert_eq!(stdout, b"interactive\n");
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
        assert_eq!(*host.selections.borrow(), vec![expected.map(str::to_owned)]);
        assert!(host.prompts.borrow().is_empty());
    }
}

#[test]
fn help_lists_doctor_before_models_with_the_frozen_summary() {
    let output = help();
    assert!(output.contains("  machine-god ask [--] [<prompt...>]\n"));
    assert!(output.contains("  ask          Run one noninteractive prompt\n"));
    let doctor_usage = output
        .find("  machine-god doctor [--json]\n")
        .expect("doctor usage");
    let models_usage = output
        .find("  machine-god models [--json]\n")
        .expect("models usage");
    assert!(doctor_usage < models_usage);

    let doctor_command = output
        .find("  doctor       Run local health and preflight checks\n")
        .expect("doctor command");
    let models_command = output
        .find("  models       List available models\n")
        .expect("models command");
    assert!(doctor_command < models_command);

    let permissions_usage = output
        .find("  machine-god permissions [--json]\n")
        .expect("permissions usage");
    let resume_usage = output
        .find("  machine-god resume [last | <id>]\n")
        .expect("resume usage");
    let inspection_usage = output
        .find("  machine-god session <id> [--json]\n")
        .expect("session usage");
    let listing_usage = output
        .find("  machine-god sessions [--all] [--limit <1-100>] [--cursor <cursor>] [--json]\n")
        .expect("sessions usage");
    let status_usage = output
        .find("  machine-god status [--json]\n")
        .expect("status usage");
    assert!(permissions_usage < resume_usage);
    assert!(resume_usage < inspection_usage);
    assert!(inspection_usage < listing_usage);
    assert!(listing_usage < status_usage);

    let permissions_command = output
        .find("  permissions  Show the permission mode and rules\n")
        .expect("permissions command");
    let resume_command = output
        .find("  resume       Resume interactively or with one prompt\n")
        .expect("resume command");
    let inspection_command = output
        .find("  session      Inspect a saved session\n")
        .expect("session command");
    let listing_command = output
        .find("  sessions     List saved sessions\n")
        .expect("sessions command");
    let status_command = output
        .find("  status       Show configuration and runtime information\n")
        .expect("status command");
    assert!(permissions_command < resume_command);
    assert!(resume_command < inspection_command);
    assert!(inspection_command < listing_command);
    assert!(listing_command < status_command);
}

#[test]
fn ask_dispatches_one_valid_prompt_and_preserves_host_output() {
    let host = FakeAskHost::new(AskCommandOutcome::Completed, "one\0β".as_bytes());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [
            OsString::from("ask"),
            OsString::from("hello"),
            OsString::from("world"),
        ],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            ask: &host,
            ..Default::default()
        },
    );
    assert_eq!(exit, 0);
    assert_eq!(host.calls.get(), 1);
    assert_eq!(*host.selections.borrow(), [None]);
    assert_eq!(*host.prompts.borrow(), ["hello world"]);
    assert_eq!(stdout, "one\0β".as_bytes());
    assert!(stderr.is_empty());
}

#[test]
fn no_argument_ask_dispatches_only_the_owned_stdin_host_seam() {
    for args in [vec!["ask"], vec!["ask", "--"]] {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"unused");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                args.into_iter().map(OsString::from),
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            0
        );
        assert_eq!(host.calls.get(), 1);
        assert_eq!(*host.prompts.borrow(), ["<stdin>"]);
        assert!(host.selections.borrow().is_empty());
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }
}

#[test]
fn invalid_ask_arguments_precede_all_host_effects() {
    let host = FakeAskHost::new(AskCommandOutcome::Completed, b"never");
    for arguments in [
        vec![OsString::from("ask"), OsString::from("--json")],
        vec![OsString::from("ask"), OsString::from(" \t\r\n")],
        vec![OsString::from("ask"), OsString::from("nul\0prompt")],
    ] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            2
        );
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
    }
    assert_eq!(host.calls.get(), 0);
    assert!(host.selections.borrow().is_empty());
    assert!(host.prompts.borrow().is_empty());
}

#[test]
fn resume_dispatches_the_validated_id_and_prompt_once() {
    let host = FakeAskHost::new(AskCommandOutcome::Completed, b"continued");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("continue"),
            OsString::from("now"),
        ],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            ask: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 0);
    assert_eq!(host.calls.get(), 1);
    assert_eq!(*host.selections.borrow(), [Some("alpha".to_owned())]);
    assert_eq!(*host.prompts.borrow(), ["continue now"]);
    assert_eq!(stdout, b"continued");
    assert!(stderr.is_empty());
}

#[test]
fn invalid_resume_arguments_precede_all_host_effects() {
    let host = FakeAskHost::new(AskCommandOutcome::Completed, b"never");
    let oversized_prompt = "x".repeat(MAX_ASK_PROMPT_BYTES + 1);
    for arguments in [
        vec![
            OsString::from("resume"),
            OsString::from("last"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("--flag"),
            OsString::from("prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("--flag"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from("nul\0prompt"),
        ],
        vec![
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from(oversized_prompt),
        ],
    ] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    ask: &host,
                    ..Default::default()
                }
            ),
            2
        );
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
    }
    assert_eq!(host.calls.get(), 0);
    assert!(host.selections.borrow().is_empty());
    assert!(host.prompts.borrow().is_empty());
}

#[test]
fn permissions_outputs_are_exact() {
    let loaded = machine_god_native::load_native_config(
        &machine_god_native::NativeEnvironment::new(None, None, None),
    )
    .unwrap();
    let report =
        machine_god_native::inspect_native_permissions(&loaded, std::path::Path::new("/workspace"))
            .unwrap();
    assert_eq!(
        crate::permissions::render(&report, false).unwrap(),
        concat!(
            "machine-god 0.1.0 (engine API 1)\n",
            "permission_mode: ask\n",
            "configuration_origin: built_in_defaults\n",
            "configured_rules_source: user\n",
            "user_rules: 0\nlocal_rules: absent\n",
            "saved_exact_rules: unavailable\n",
            "runtime_grants: unavailable\n",
        )
    );
    assert_eq!(
        crate::permissions::render(&report, true).unwrap(),
        concat!(
            "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
            "\"engine_api_version\":1,\"kind\":\"permissions\",",
            "\"permission_mode\":\"ask\",",
            "\"configuration_origin\":\"built_in_defaults\",",
            "\"configured_rules\":{\"effective_source\":\"user\",\"user\":[],\"local\":null},",
            "\"saved_exact_rules_available\":false,",
            "\"runtime_grants_available\":false}\n",
        )
    );
}

#[test]
fn json_encoder_escapes_terminal_controls_and_json_metacharacters() {
    let mut encoded = String::new();
    push_json_string(
        &mut encoded,
        concat!(
            "quote\" slash\\ controls\n\r\t\u{1b}\u{7f}\u{85} ",
            "bidi\u{061c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069} ",
            "separators\u{2028}\u{2029}",
        ),
    );
    assert_eq!(
        encoded,
        concat!(
            "\"quote\\\" slash\\\\ controls\\n\\r\\t\\u001b\\u007f\\u0085 ",
            "bidi\\u061c\\u200e\\u200f\\u202a\\u202e\\u2066\\u2069 ",
            "separators\\u2028\\u2029\"",
        )
    );
}

#[test]
fn output_failure_is_a_fixed_diagnostic_without_panicking() {
    let mut stdout = BrokenWriter;
    let mut stderr = Vec::new();
    let exit = run([OsString::from("--version")], &mut stdout, &mut stderr);

    assert_eq!(exit, 1);
    assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
}

#[test]
fn invalid_arguments_do_not_touch_stdout() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run([OsString::from("nope")], &mut stdout, &mut stderr);

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn private_background_helper_dispatch_requires_the_exact_single_argument() {
    let helper = machine_god_native::BACKGROUND_PROCESS_HELPER_ARGUMENT;
    assert!(is_background_process_helper_arguments([OsString::from(
        helper
    )]));
    assert!(!is_background_process_helper_arguments([]));
    assert!(!is_background_process_helper_arguments([
        OsString::from(helper),
        OsString::from("extra"),
    ]));
    assert!(!is_background_process_helper_arguments([OsString::from(
        "background"
    )]));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn private_terminal_helpers_require_exact_single_argument_without_normal_cli_dispatch() {
    for helper in [
        machine_god_native::INTERACTIVE_INPUT_HELPER_ARGUMENT,
        #[cfg(target_os = "macos")]
        machine_god_native::PROCESS_INVENTORY_HELPER_ARGUMENT,
        #[cfg(target_os = "macos")]
        machine_god_native::PROCESS_INVENTORY_SERVICE_ARGUMENT,
        machine_god_native::TERMINAL_CAPTURED_HELPER_ARGUMENT,
        machine_god_native::TERMINAL_PTY_HELPER_ARGUMENT,
        machine_god_native::TERMINAL_STARTUP_MARKER_ARGUMENT,
    ] {
        assert!(super::is_exact_helper_arguments(
            [OsString::from(helper)],
            helper
        ));
        assert!(!super::is_exact_helper_arguments([], helper));
        assert!(!super::is_exact_helper_arguments(
            [OsString::from(helper), OsString::from("extra")],
            helper
        ));
        assert!(!super::is_exact_helper_arguments(
            [OsString::from("terminal")],
            helper
        ));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // The ordinary parser never treats private process modes as a
        // user command or starts a host as a side effect of parsing one.
        assert_eq!(run([OsString::from(helper)], &mut stdout, &mut stderr), 2);
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn private_tmux_helper_dispatch_is_exact_bounded_and_not_an_ordinary_command() {
    let flag = OsString::from(machine_god_native::TERMINAL_TMUX_HELPER_ARGUMENT);
    let values = [
        OsString::from("pane"),
        OsString::from("/private/socket"),
        OsString::from("nonce"),
        OsString::from("identity"),
    ];
    let exact = std::iter::once(flag.clone()).chain(values.clone());
    assert_eq!(
        super::terminal_tmux_helper_arguments(exact),
        Some(Ok(values))
    );
    assert_eq!(
        super::terminal_tmux_helper_arguments([flag.clone()]),
        Some(Err(()))
    );
    assert_eq!(
        super::terminal_tmux_helper_arguments(
            std::iter::once(flag.clone()).chain(std::iter::repeat(OsString::from("overflow")))
        ),
        Some(Err(()))
    );
    assert_eq!(
        super::terminal_tmux_helper_arguments([OsString::from("help")]),
        None
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(run([flag], &mut stdout, &mut stderr), 2);
    assert!(stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn non_unicode_arguments_are_rejected() {
    use std::os::unix::ffi::OsStringExt;

    assert_eq!(parse_arguments([OsString::from_vec(vec![0xff])]), Err(()));
    assert_eq!(
        parse_arguments([OsString::from("doctor"), OsString::from_vec(vec![0xff]),]),
        Err(())
    );
    assert_eq!(
        parse_arguments([OsString::from("models"), OsString::from_vec(vec![0xff]),]),
        Err(())
    );
    assert_eq!(
        parse_arguments([OsString::from("session"), OsString::from_vec(vec![0xff]),]),
        Err(())
    );
    assert_eq!(
        parse_arguments([OsString::from("resume"), OsString::from_vec(vec![0xff]),]),
        Err(())
    );
    assert_eq!(
        parse_arguments([
            OsString::from("resume"),
            OsString::from("alpha"),
            OsString::from_vec(vec![0xff]),
        ]),
        Err(())
    );
    assert_eq!(
        parse_arguments([
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from_vec(vec![0xff]),
        ]),
        Err(())
    );
    assert_eq!(
        parse_arguments([OsString::from("sessions"), OsString::from_vec(vec![0xff]),]),
        Err(())
    );
    assert_eq!(
        parse_arguments([OsString::from("workspace"), OsString::from_vec(vec![0xff]),]),
        Err(())
    );
    assert_eq!(
        parse_arguments([
            OsString::from("workspace"),
            OsString::from("list"),
            OsString::from_vec(vec![0xff]),
        ]),
        Err(())
    );
}

// Keep the externally selected private child entrypoint stable after extraction.
#[cfg(unix)]
#[test]
fn models_signal_output_subprocess_child() {
    crate::models::tests::models_signal_output_subprocess_child();
}
