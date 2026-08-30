use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;
use std::task::{Context, Poll, Waker};

use machine_god_core::{BoxFuture, CancellationToken};
use machine_god_native::{
    TerminalTapeReplayError, TerminalTapeReplayRequest, replay_terminal_tape,
};

const REPLAY_HELP: &str = concat!(
    "machine-god replay\n",
    "\n",
    "Replay a recorded terminal session\n",
    "\n",
    "Usage:\n",
    "  machine-god replay <tape> [--frames] [--json] [--golden <path>] [--frames-dir <path>]\n",
    "\n",
    "Options:\n",
    "  --frames             Render each captured frame\n",
    "  --golden <path>      Write the final rendered grid to a file\n",
    "  --frames-dir <path>  Write rendered frames to a directory\n",
    "  --json               Emit machine-readable JSON instead of text\n",
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReplayInvocation {
    tape: PathBuf,
    frames: bool,
    json: bool,
    golden: Option<PathBuf>,
    frames_dir: Option<PathBuf>,
}

impl ReplayInvocation {
    fn into_native_request(self) -> TerminalTapeReplayRequest {
        TerminalTapeReplayRequest::new(
            self.tape,
            self.frames,
            self.json,
            self.golden,
            self.frames_dir,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplayParseError {
    MissingTapePath,
    TooManyArgs,
    UnknownFlag,
    MissingGoldenPath,
    MissingFramesDirPath,
}

impl ReplayParseError {
    const fn code(self) -> &'static str {
        match self {
            Self::MissingTapePath => "MissingTapePath",
            Self::TooManyArgs => "TooManyArgs",
            Self::UnknownFlag => "UnknownFlag",
            Self::MissingGoldenPath => "MissingGoldenPath",
            Self::MissingFramesDirPath => "MissingFramesDirPath",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::MissingTapePath => concat!(
                "machine-god replay: missing tape path\n",
                "usage: machine-god replay <tape> [--frames] [--json] ",
                "[--golden <path>] [--frames-dir <path>]\n",
            ),
            Self::TooManyArgs => "machine-god replay: too many positional arguments\n",
            Self::UnknownFlag => "machine-god replay: unknown flag\n",
            Self::MissingGoldenPath => "machine-god replay: --golden requires a path\n",
            Self::MissingFramesDirPath => "machine-god replay: --frames-dir requires a path\n",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReplayParseOutcome {
    Help,
    Run(ReplayInvocation),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReplayCommandOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ReplayCommandOutput {
    #[cfg(test)]
    fn new(stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReplayCommandFailure {
    code: &'static str,
    message: &'static str,
}

impl ReplayCommandFailure {
    const fn invariant() -> Self {
        Self {
            code: "ResourceLimit",
            message: "replay invariant failed: ResourceLimit",
        }
    }
}

impl From<TerminalTapeReplayError> for ReplayCommandFailure {
    fn from(error: TerminalTapeReplayError) -> Self {
        Self {
            code: error.code(),
            message: error.message(),
        }
    }
}

pub(crate) trait ReplayCommandHost {
    fn replay(
        &self,
        invocation: ReplayInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<ReplayCommandOutput, ReplayCommandFailure>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProductionReplayCommandHost;

impl ReplayCommandHost for ProductionReplayCommandHost {
    fn replay(
        &self,
        invocation: ReplayInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<ReplayCommandOutput, ReplayCommandFailure>> {
        Box::pin(async move {
            replay_terminal_tape(invocation.into_native_request(), cancellation)
                .await
                .map(|output| {
                    let (stdout, stderr) = output.into_parts();
                    ReplayCommandOutput { stdout, stderr }
                })
                .map_err(ReplayCommandFailure::from)
        })
    }
}

pub(crate) fn is_replay_command(argument: &OsStr) -> bool {
    argument == OsStr::new("replay")
}

pub(crate) fn run_replay(
    host: &impl ReplayCommandHost,
    arguments: &[OsString],
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &str,
) -> u8 {
    let json_error = arguments
        .iter()
        .any(|argument| argument == OsStr::new("--json"));
    let invocation = match parse_arguments(arguments) {
        Ok(ReplayParseOutcome::Help) => {
            return write_success(REPLAY_HELP.as_bytes(), &[], stdout, stderr, output_failure);
        }
        Ok(ReplayParseOutcome::Run(invocation)) => invocation,
        Err(error) => {
            return write_failure(
                error.code(),
                error.message(),
                json_error,
                stdout,
                stderr,
                output_failure,
            );
        }
    };
    let json = invocation.json;
    let mut replay = host.replay(invocation, CancellationToken::new());
    let mut context = Context::from_waker(Waker::noop());
    let result = match replay.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => Err(ReplayCommandFailure::invariant()),
    };
    match result {
        Ok(output) => write_success(
            &output.stdout,
            &output.stderr,
            stdout,
            stderr,
            output_failure,
        ),
        Err(failure) => {
            let mut message = String::from("machine-god replay: ");
            message.push_str(failure.message);
            message.push('\n');
            write_failure(failure.code, &message, json, stdout, stderr, output_failure)
        }
    }
}

fn parse_arguments(arguments: &[OsString]) -> Result<ReplayParseOutcome, ReplayParseError> {
    if arguments
        .iter()
        .any(|argument| matches!(argument.to_str(), Some("-h" | "--help")))
    {
        return Ok(ReplayParseOutcome::Help);
    }

    let mut invocation = ReplayInvocation {
        tape: PathBuf::new(),
        frames: false,
        json: false,
        golden: None,
        frames_dir: None,
    };
    let mut positional_seen = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == OsStr::new("--frames") {
            invocation.frames = true;
        } else if argument == OsStr::new("--json") {
            invocation.json = true;
        } else if argument == OsStr::new("--golden") {
            index += 1;
            let Some(path) = arguments.get(index) else {
                return Err(ReplayParseError::MissingGoldenPath);
            };
            invocation.golden = Some(PathBuf::from(path.as_os_str()));
        } else if argument == OsStr::new("--frames-dir") {
            index += 1;
            let Some(path) = arguments.get(index) else {
                return Err(ReplayParseError::MissingFramesDirPath);
            };
            invocation.frames_dir = Some(PathBuf::from(path.as_os_str()));
        } else if argument.as_encoded_bytes().starts_with(b"--") {
            return Err(ReplayParseError::UnknownFlag);
        } else if positional_seen {
            return Err(ReplayParseError::TooManyArgs);
        } else {
            invocation.tape = PathBuf::from(argument.as_os_str());
            positional_seen = true;
        }
        index += 1;
    }

    if !positional_seen {
        return Err(ReplayParseError::MissingTapePath);
    }
    Ok(ReplayParseOutcome::Run(invocation))
}

fn write_success(
    replay_stdout: &[u8],
    replay_stderr: &[u8],
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &str,
) -> u8 {
    if stdout.write_all(replay_stdout).is_err() || stderr.write_all(replay_stderr).is_err() {
        let _ = stderr.write_all(output_failure.as_bytes());
        return 1;
    }
    0
}

fn write_failure(
    code: &str,
    message: &str,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &str,
) -> u8 {
    let result = if json {
        let mut rendered = String::with_capacity(message.len().saturating_add(code.len() + 48));
        rendered.push_str("{\"kind\":\"replay\",\"error\":");
        write_json_string(&mut rendered, message.trim_end_matches(['\r', '\n']));
        rendered.push_str(",\"code\":");
        write_json_string(&mut rendered, code);
        rendered.push_str("}\n");
        stdout.write_all(rendered.as_bytes())
    } else {
        stderr.write_all(message.as_bytes())
    };
    if result.is_err() {
        let _ = stderr.write_all(output_failure.as_bytes());
    }
    1
}

fn write_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{00}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{061c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => {
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            _ => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::{
        REPLAY_HELP, ReplayCommandFailure, ReplayCommandHost, ReplayCommandOutput,
        ReplayInvocation, ReplayParseError, ReplayParseOutcome, parse_arguments, run_replay,
    };
    use machine_god_core::{BoxFuture, CancellationToken};
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::io;

    #[derive(Debug)]
    struct FakeReplayHost {
        result: Result<ReplayCommandOutput, ReplayCommandFailure>,
        calls: Cell<usize>,
        invocations: RefCell<Vec<ReplayInvocation>>,
        cancellations: RefCell<Vec<CancellationToken>>,
        pending: bool,
    }

    impl FakeReplayHost {
        fn ready(result: Result<ReplayCommandOutput, ReplayCommandFailure>) -> Self {
            Self {
                result,
                calls: Cell::new(0),
                invocations: RefCell::new(Vec::new()),
                cancellations: RefCell::new(Vec::new()),
                pending: false,
            }
        }

        fn pending() -> Self {
            Self {
                result: Err(ReplayCommandFailure::invariant()),
                calls: Cell::new(0),
                invocations: RefCell::new(Vec::new()),
                cancellations: RefCell::new(Vec::new()),
                pending: true,
            }
        }
    }

    impl ReplayCommandHost for FakeReplayHost {
        fn replay(
            &self,
            invocation: ReplayInvocation,
            cancellation: CancellationToken,
        ) -> BoxFuture<'static, Result<ReplayCommandOutput, ReplayCommandFailure>> {
            self.calls.set(self.calls.get() + 1);
            self.invocations.borrow_mut().push(invocation);
            self.cancellations.borrow_mut().push(cancellation);
            if self.pending {
                Box::pin(std::future::pending())
            } else {
                Box::pin(std::future::ready(self.result.clone()))
            }
        }
    }

    #[test]
    fn parser_matches_the_pinned_fx_grammar() {
        assert_eq!(
            parse_arguments(&[
                "--frames".into(),
                "first.fxtape".into(),
                "--json".into(),
                "--golden".into(),
                "one.txt".into(),
                "--golden".into(),
                "two.txt".into(),
                "--frames-dir".into(),
                "frames-one".into(),
                "--frames-dir".into(),
                "frames-two".into(),
                "--frames".into(),
                "--json".into(),
            ]),
            Ok(ReplayParseOutcome::Run(ReplayInvocation {
                tape: "first.fxtape".into(),
                frames: true,
                json: true,
                golden: Some("two.txt".into()),
                frames_dir: Some("frames-two".into()),
            }))
        );
        for tape in ["", "-"] {
            assert!(matches!(
                parse_arguments(&[OsString::from(tape)]),
                Ok(ReplayParseOutcome::Run(_))
            ));
        }
        assert_eq!(
            parse_arguments(&["tape".into(), "second".into()]),
            Err(ReplayParseError::TooManyArgs)
        );
        assert_eq!(
            parse_arguments(&["tape".into(), "--unknown".into()]),
            Err(ReplayParseError::UnknownFlag)
        );
        assert_eq!(
            parse_arguments(&["tape".into(), "--".into()]),
            Err(ReplayParseError::UnknownFlag)
        );
        assert_eq!(
            parse_arguments(&["--golden".into()]),
            Err(ReplayParseError::MissingGoldenPath)
        );
        assert_eq!(
            parse_arguments(&["--frames-dir".into()]),
            Err(ReplayParseError::MissingFramesDirPath)
        );
        assert_eq!(
            parse_arguments(&["--golden".into(), "--json".into()]),
            Err(ReplayParseError::MissingTapePath)
        );
    }

    #[test]
    fn help_anywhere_preempts_errors_and_effects() {
        for arguments in [
            vec!["--help".into()],
            vec!["tape".into(), "extra".into(), "-h".into()],
            vec!["--golden".into(), "--help".into()],
        ] {
            let host = FakeReplayHost::ready(Ok(ReplayCommandOutput::new([], [])));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_replay(
                    &host,
                    &arguments,
                    &mut stdout,
                    &mut stderr,
                    "output failed\n"
                ),
                0
            );
            assert_eq!(stdout, REPLAY_HELP.as_bytes());
            assert!(stderr.is_empty());
            assert_eq!(host.calls.get(), 0);
        }
    }

    #[test]
    fn runner_passes_exact_options_and_writes_stdout_before_stderr() {
        let host = FakeReplayHost::ready(Ok(ReplayCommandOutput::new("grid\n", "warning\n")));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_replay(
                &host,
                &[
                    "tape".into(),
                    "--frames".into(),
                    "--json".into(),
                    "--golden".into(),
                    "golden".into(),
                    "--frames-dir".into(),
                    "frames".into(),
                ],
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            0
        );
        assert_eq!(stdout, b"grid\n");
        assert_eq!(stderr, b"warning\n");
        assert_eq!(host.calls.get(), 1);
        assert!(!host.cancellations.borrow()[0].is_cancelled());
        assert_eq!(
            host.invocations.borrow()[0],
            ReplayInvocation {
                tape: "tape".into(),
                frames: true,
                json: true,
                golden: Some("golden".into()),
                frames_dir: Some("frames".into()),
            }
        );
    }

    #[test]
    fn parse_and_native_failures_use_upstream_compatible_codes() {
        let host = FakeReplayHost::ready(Err(ReplayCommandFailure {
            code: "BadTape",
            message: "bad tape: BadTape",
        }));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_replay(
                &host,
                &["tape".into(), "--json".into()],
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            1
        );
        assert_eq!(
            stdout,
            b"{\"kind\":\"replay\",\"error\":\"machine-god replay: bad tape: BadTape\",\"code\":\"BadTape\"}\n"
        );
        assert!(stderr.is_empty());

        let host = FakeReplayHost::ready(Ok(ReplayCommandOutput::new([], [])));
        stdout.clear();
        assert_eq!(
            run_replay(
                &host,
                &["--golden".into(), "--json".into()],
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            1
        );
        assert_eq!(
            stdout,
            b"{\"kind\":\"replay\",\"error\":\"machine-god replay: missing tape path\\nusage: machine-god replay <tape> [--frames] [--json] [--golden <path>] [--frames-dir <path>]\",\"code\":\"MissingTapePath\"}\n"
        );
        assert_eq!(host.calls.get(), 0);
    }

    #[test]
    fn unexpected_pending_is_a_bounded_resource_limit_failure() {
        let host = FakeReplayHost::pending();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_replay(
                &host,
                &["tape".into()],
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            1
        );
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            b"machine-god replay: replay invariant failed: ResourceLimit\n"
        );
    }

    #[test]
    fn output_failure_is_reported_without_replaying() {
        struct Broken;
        impl io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let host = FakeReplayHost::ready(Ok(ReplayCommandOutput::new("grid", [])));
        let mut stdout = Broken;
        let mut stderr = Vec::new();
        assert_eq!(
            run_replay(
                &host,
                &["tape".into()],
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            1
        );
        assert_eq!(stderr, b"output failed\n");
        assert_eq!(host.calls.get(), 1);
    }
}
