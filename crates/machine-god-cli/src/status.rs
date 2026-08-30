use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::io;

use machine_god_native::{
    NativeRuntimeStatus, NativeRuntimeStatusError, inspect_process_runtime_status,
};

use super::{write_json_string, write_json_string_content};

pub(crate) const MAX_STATUS_OUTPUT_BYTES: usize = 64 * 1024;

const STATUS_HELP: &str = concat!(
    "machine-god status\n",
    "\n",
    "Show configuration and runtime information\n",
    "\n",
    "Usage:\n",
    "  machine-god status [--json]\n",
    "\n",
    "Options:\n",
    "  --json  Emit machine-readable JSON instead of text\n",
);
const STATUS_USAGE: &str = "usage: machine-god status [--json]\n";
const STATUS_INVALID_JSON: &str = concat!(
    "{\"kind\":\"status\",\"error\":\"invalid arguments\",",
    "\"code\":\"InvalidLocalSurfaceArgs\"}\n",
);
const STATUS_RENDER_FAILURE: &str = "machine-god status: could not render report\n";
const STATUS_INSPECTION_FAILURE: &str = "machine-god status: could not inspect runtime\n";

pub(crate) trait StatusCommandHost {
    fn inspect_status(&self) -> Result<NativeRuntimeStatus, NativeRuntimeStatusError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProductionStatusCommandHost;

impl StatusCommandHost for ProductionStatusCommandHost {
    fn inspect_status(&self) -> Result<NativeRuntimeStatus, NativeRuntimeStatusError> {
        inspect_process_runtime_status()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatusParseOutcome {
    Help,
    Invoke { json: bool },
    Invalid { json: bool },
}

#[derive(Debug)]
struct BoundedStatusOutput {
    value: String,
}

impl BoundedStatusOutput {
    fn new() -> Self {
        Self {
            value: String::with_capacity(1024),
        }
    }

    fn finish(self) -> String {
        self.value
    }
}

impl std::fmt::Write for BoundedStatusOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.value.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > MAX_STATUS_OUTPUT_BYTES {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

pub(crate) fn is_status_command(argument: &OsStr) -> bool {
    argument == "status"
}

pub(crate) fn run_status(
    host: &impl StatusCommandHost,
    arguments: &[OsString],
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &str,
) -> u8 {
    match parse_arguments(arguments) {
        StatusParseOutcome::Help => {
            write_stdout(STATUS_HELP.as_bytes(), stdout, stderr, output_failure)
        }
        StatusParseOutcome::Invalid { json: true } => {
            if stdout.write_all(STATUS_INVALID_JSON.as_bytes()).is_err() {
                let _ = stderr.write_all(output_failure.as_bytes());
            }
            1
        }
        StatusParseOutcome::Invalid { json: false } => {
            if stderr.write_all(STATUS_USAGE.as_bytes()).is_err() {
                let _ = stderr.write_all(output_failure.as_bytes());
            }
            1
        }
        StatusParseOutcome::Invoke { json } => {
            let Ok(status) = host.inspect_status() else {
                let _ = stderr.write_all(STATUS_INSPECTION_FAILURE.as_bytes());
                return 1;
            };
            let Ok(output) = render_status(&status, json) else {
                let _ = stderr.write_all(STATUS_RENDER_FAILURE.as_bytes());
                return 1;
            };
            write_stdout(output.as_bytes(), stdout, stderr, output_failure)
        }
    }
}

fn parse_arguments(arguments: &[OsString]) -> StatusParseOutcome {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return StatusParseOutcome::Help;
    }

    let mut json = false;
    let mut invalid = false;
    for argument in arguments {
        if argument == "--json" {
            json = true;
        } else {
            invalid = true;
        }
    }
    if invalid {
        StatusParseOutcome::Invalid { json }
    } else {
        StatusParseOutcome::Invoke { json }
    }
}

fn render_status(status: &NativeRuntimeStatus, json: bool) -> Result<String, std::fmt::Error> {
    if json {
        json_status(status)
    } else {
        human_status(status)
    }
}

fn human_status(status: &NativeRuntimeStatus) -> Result<String, std::fmt::Error> {
    let mut output = BoundedStatusOutput::new();
    output.write_str("[status] model=")?;
    write_json_string_content(&mut output, status.model())?;
    output.write_char('\n')?;
    writeln!(
        output,
        "[status] update_channel={}",
        status.update_channel()
    )?;
    writeln!(output, "[status] build_channel={}", status.build_channel())?;
    if !status.build_revision().is_empty() {
        writeln!(
            output,
            "[status] build_revision={}",
            status.build_revision()
        )?;
    }
    writeln!(output, "[status] auth={}", status.auth_label())?;
    writeln!(
        output,
        "[status] auth_refreshable={}",
        status.auth_refreshable()
    )?;
    if let Some(help) = status.auth_help() {
        writeln!(output, "[status] auth_help={help}")?;
    }
    writeln!(
        output,
        "[status] permission_mode={}",
        status.permission_mode().as_str()
    )?;
    writeln!(output, "[status] sandbox={}", status.sandbox())?;
    output.write_str("[status] workspace=")?;
    write_json_string_content(
        &mut output,
        status.workspace().to_str().ok_or(std::fmt::Error)?,
    )?;
    output.write_char('\n')?;
    writeln!(output, "[status] history_turns={}", status.history_turns())?;
    writeln!(
        output,
        "[status] session_permission_grants={}",
        status.session_permission_grants()
    )?;
    writeln!(
        output,
        "[status] agent_step_limit={}",
        status.agent_step_limit()
    )?;
    Ok(output.finish())
}

fn json_status(status: &NativeRuntimeStatus) -> Result<String, std::fmt::Error> {
    let mut output = BoundedStatusOutput::new();
    output.write_str("{\"kind\":\"status\",\"model\":")?;
    write_json_string(&mut output, status.model())?;
    output.write_str(",\"update_channel\":")?;
    write_json_string(&mut output, status.update_channel())?;
    output.write_str(",\"build_channel\":")?;
    write_json_string(&mut output, status.build_channel())?;
    output.write_str(",\"build_revision\":")?;
    write_json_string(&mut output, status.build_revision())?;
    output.write_str(",\"auth\":")?;
    write_json_string(&mut output, status.auth_label())?;
    write!(
        output,
        ",\"auth_refreshable\":{}",
        status.auth_refreshable()
    )?;
    if let Some(help) = status.auth_help() {
        output.write_str(",\"auth_help\":")?;
        write_json_string(&mut output, help)?;
    }
    output.write_str(",\"permission_mode\":")?;
    write_json_string(&mut output, status.permission_mode().as_str())?;
    output.write_str(",\"sandbox\":")?;
    write_json_string(&mut output, status.sandbox())?;
    output.write_str(",\"workspace\":")?;
    write_json_status_path(&mut output, status.workspace())?;
    write!(
        output,
        ",\"history_turns\":{},\"session_permission_grants\":{},\"agent_step_limit\":{}",
        status.history_turns(),
        status.session_permission_grants(),
        status.agent_step_limit()
    )?;
    output.write_str("}\n")?;
    Ok(output.finish())
}

fn write_json_status_path(
    output: &mut impl std::fmt::Write,
    path: &std::path::Path,
) -> std::fmt::Result {
    write_json_string(output, path.to_str().ok_or(std::fmt::Error)?)
}

fn write_stdout(
    output: &[u8],
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &str,
) -> u8 {
    if stdout.write_all(output).is_err() {
        let _ = stderr.write_all(output_failure.as_bytes());
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::fmt::Write as _;
    use std::io;

    use machine_god_native::{
        AI_GATEWAY_DEFAULT_MODEL, NativeRuntimeCredentialEnvironment, NativeRuntimeStatus,
        NativeRuntimeStatusError, NativeRuntimeStatusInput, PermissionMode,
        inspect_native_runtime_status,
    };

    use super::{
        BoundedStatusOutput, MAX_STATUS_OUTPUT_BYTES, STATUS_HELP, STATUS_INSPECTION_FAILURE,
        STATUS_INVALID_JSON, STATUS_USAGE, StatusCommandHost, StatusParseOutcome, parse_arguments,
        run_status, write_json_string,
    };

    const OUTPUT_FAILURE: &str = "machine-god: failed to write output\n";

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
    struct BrokenWriter;

    impl io::Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parser_preempts_with_help_and_treats_json_as_idempotent() {
        for arguments in [
            vec![OsString::from("--help")],
            vec![OsString::from("unknown"), OsString::from("-h")],
            vec![OsString::from("--json"), OsString::from("--help")],
        ] {
            assert_eq!(parse_arguments(&arguments), StatusParseOutcome::Help);
        }
        assert_eq!(
            parse_arguments(&[]),
            StatusParseOutcome::Invoke { json: false }
        );
        assert_eq!(
            parse_arguments(&[OsString::from("--json"), OsString::from("--json")]),
            StatusParseOutcome::Invoke { json: true }
        );
        assert_eq!(
            parse_arguments(&[OsString::from("unknown")]),
            StatusParseOutcome::Invalid { json: false }
        );
        assert_eq!(
            parse_arguments(&[OsString::from("unknown"), OsString::from("--json")]),
            StatusParseOutcome::Invalid { json: true }
        );
    }

    #[cfg(unix)]
    #[test]
    fn parser_rejects_non_unicode_but_still_honors_raw_help_and_json() {
        use std::os::unix::ffi::OsStringExt;

        let invalid = OsString::from_vec(vec![0xff]);
        assert_eq!(
            parse_arguments(std::slice::from_ref(&invalid)),
            StatusParseOutcome::Invalid { json: false }
        );
        assert_eq!(
            parse_arguments(&[invalid.clone(), OsString::from("--json")]),
            StatusParseOutcome::Invalid { json: true }
        );
        assert_eq!(
            parse_arguments(&[invalid, OsString::from("--help")]),
            StatusParseOutcome::Help
        );
    }

    #[test]
    fn help_and_invalid_arguments_have_exact_outputs_without_host_calls() {
        let host = FakeStatusHost::unavailable();
        for (arguments, expected_exit, expected_stdout, expected_stderr) in [
            (
                vec![OsString::from("--help")],
                0,
                STATUS_HELP.as_bytes(),
                &[][..],
            ),
            (
                vec![OsString::from("unknown")],
                1,
                &[][..],
                STATUS_USAGE.as_bytes(),
            ),
            (
                vec![OsString::from("unknown"), OsString::from("--json")],
                1,
                STATUS_INVALID_JSON.as_bytes(),
                &[][..],
            ),
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_status(&host, &arguments, &mut stdout, &mut stderr, OUTPUT_FAILURE);
            assert_eq!(exit, expected_exit);
            assert_eq!(stdout, expected_stdout);
            assert_eq!(stderr, expected_stderr);
        }
        assert_eq!(host.calls.get(), 0);
    }

    #[test]
    fn valid_status_calls_the_host_once_and_preserves_exact_bytes() {
        let host = FakeStatusHost::unavailable();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_status(
            &host,
            &[OsString::from("--json"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            OUTPUT_FAILURE,
        );
        assert_eq!(exit, 0);
        assert_eq!(host.calls.get(), 1);
        assert_eq!(
            stdout,
            concat!(
                "{\"kind\":\"status\",\"model\":\"zai/glm-5.2\",",
                "\"update_channel\":\"stable\",\"build_channel\":\"stable\",",
                "\"build_revision\":\"\",\"auth\":\"missing\",",
                "\"auth_refreshable\":false,\"auth_help\":",
                "\"Machine God needs access to Vercel AI Gateway. Set ",
                "VERCEL_OIDC_TOKEN or AI_GATEWAY_API_KEY.\",",
                "\"permission_mode\":\"ask\",\"sandbox\":\"none\",",
                "\"workspace\":\"/workspace\",\"history_turns\":0,",
                "\"session_permission_grants\":0,\"agent_step_limit\":8}\n",
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn human_status_escapes_valid_model_metacharacters() {
        let host = FakeStatusHost {
            calls: Cell::new(0),
            status: inspect_native_runtime_status(NativeRuntimeStatusInput::new(
                "provider/\"model\\variant",
                PermissionMode::Ask,
                NativeRuntimeCredentialEnvironment::new(None, None),
                "/workspace",
                None,
            )),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_status(&host, &[], &mut stdout, &mut stderr, OUTPUT_FAILURE),
            0
        );
        assert!(
            stdout.starts_with(b"[status] model=provider/\\\"model\\\\variant\n"),
            "unexpected human status output: {}",
            String::from_utf8_lossy(&stdout)
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn bounded_writer_accepts_the_cap_rejects_one_over_and_counts_escaping() {
        let mut exact = BoundedStatusOutput::new();
        exact
            .write_str(&"x".repeat(MAX_STATUS_OUTPUT_BYTES))
            .unwrap();
        assert_eq!(exact.value.len(), MAX_STATUS_OUTPUT_BYTES);
        assert!(exact.write_char('x').is_err());

        let mut escaped = BoundedStatusOutput::new();
        write_json_string(&mut escaped, "\u{1b}\u{202e}\n\\\"").unwrap();
        assert_eq!(escaped.value, "\"\\u001b\\u202e\\n\\\\\\\"\"");
    }

    #[test]
    fn inspection_failure_is_atomic_and_uses_the_fixed_diagnostic() {
        let host = FakeStatusHost {
            calls: Cell::new(0),
            status: inspect_native_runtime_status(NativeRuntimeStatusInput::new(
                AI_GATEWAY_DEFAULT_MODEL,
                PermissionMode::Ask,
                NativeRuntimeCredentialEnvironment::new(
                    Some(OsString::from("invalid credential")),
                    None,
                ),
                "/workspace",
                None,
            )),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_status(&host, &[], &mut stdout, &mut stderr, OUTPUT_FAILURE),
            1
        );
        assert_eq!(host.calls.get(), 1);
        assert!(stdout.is_empty());
        assert_eq!(stderr, STATUS_INSPECTION_FAILURE.as_bytes());
    }

    #[test]
    fn output_failures_are_fixed_and_do_not_repeat_host_effects() {
        let host = FakeStatusHost::unavailable();
        for arguments in [vec![OsString::from("--help")], Vec::new()] {
            let mut stdout = BrokenWriter;
            let mut stderr = Vec::new();
            assert_eq!(
                run_status(&host, &arguments, &mut stdout, &mut stderr, OUTPUT_FAILURE,),
                1
            );
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        }
        assert_eq!(host.calls.get(), 1);

        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        assert_eq!(
            run_status(
                &host,
                &[OsString::from("unknown"), OsString::from("--json")],
                &mut stdout,
                &mut stderr,
                OUTPUT_FAILURE,
            ),
            1
        );
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        assert_eq!(host.calls.get(), 1);
    }
}
