use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use machine_god_native::{
    NativeStatus, PermissionMode, inspect_process_status, load_process_config,
};

const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | permissions [--json] | status [--json]]\n",
);
const CONFIGURATION_FAILURE: &str = "machine-god: failed to load configuration\n";
const OUTPUT_FAILURE: &str = "machine-god: failed to write output\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Identity,
    Help,
    Permissions { json: bool },
    Status { json: bool },
}

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    ExitCode::from(run(env::args_os().skip(1), &mut stdout, &mut stderr))
}

fn run(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let Ok(command) = parse_arguments(arguments) else {
        let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
        return 2;
    };

    let output = match command {
        Command::Identity => identity(),
        Command::Help => help(),
        Command::Permissions { json } => {
            let Ok(loaded) = load_process_config() else {
                let _ = stderr.write_all(CONFIGURATION_FAILURE.as_bytes());
                return 1;
            };
            permissions(loaded.config().permission_mode(), json)
        }
        Command::Status { json } => status(&inspect_process_status(), json),
    };

    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn parse_arguments(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, ()> {
    let mut arguments = arguments.into_iter();
    let Some(first) = arguments.next() else {
        return Ok(Command::Identity);
    };
    let Some(first) = first.to_str() else {
        return Err(());
    };

    let command = match first {
        "help" | "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Identity,
        "permissions" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Permissions { json }
        }
        "status" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Status { json }
        }
        _ => return Err(()),
    };

    if arguments.next().is_some() {
        return Err(());
    }
    Ok(command)
}

fn identity() -> String {
    format!(
        "machine-god {} (engine API {})\n",
        env!("CARGO_PKG_VERSION"),
        machine_god_native::supported_core_api_version()
    )
}

fn help() -> String {
    format!(
        concat!(
            "machine-god {}\n",
            "Embeddable coding-agent engine\n",
            "\n",
            "Usage:\n",
            "  machine-god\n",
            "  machine-god help\n",
            "  machine-god permissions [--json]\n",
            "  machine-god status [--json]\n",
            "\n",
            "Commands:\n",
            "  help         Show this help\n",
            "  permissions  Show the permission mode and rules\n",
            "  status       Show configuration and runtime information\n",
            "\n",
            "Options:\n",
            "  -h, --help       Show this help\n",
            "  -V, --version    Show version\n",
        ),
        env!("CARGO_PKG_VERSION")
    )
}

fn permissions(permission_mode: PermissionMode, json: bool) -> String {
    if json {
        json_permissions(permission_mode)
    } else {
        human_permissions(permission_mode)
    }
}

fn human_permissions(permission_mode: PermissionMode) -> String {
    let mut output = identity();
    let _ = writeln!(output, "permission_mode: {}", permission_mode.as_str());
    output.push_str("persistent_rules: unsupported\n");
    output.push_str("runtime_grants: unavailable\n");
    output
}

fn json_permissions(permission_mode: PermissionMode) -> String {
    let mut output = String::from("{\"name\":\"machine-god\",\"version\":");
    push_json_string(&mut output, env!("CARGO_PKG_VERSION"));
    let _ = write!(
        output,
        ",\"engine_api_version\":{},\"kind\":\"permissions\",\"permission_mode\":",
        machine_god_native::supported_core_api_version()
    );
    push_json_string(&mut output, permission_mode.as_str());
    output.push_str(",\"persistent_rules_supported\":false,\"runtime_grants_available\":false}\n");
    output
}

fn status(status: &NativeStatus, json: bool) -> String {
    if json {
        json_status(status)
    } else {
        human_status(status)
    }
}

fn human_status(status: &NativeStatus) -> String {
    let mut output = identity();
    let _ = writeln!(
        output,
        "permission_mode: {}",
        status.permission_mode().as_str()
    );
    let _ = write!(
        output,
        "config_file: state={} path=",
        status.config_file_state().as_str()
    );
    push_json_path(&mut output, status.config_file_path());
    output.push('\n');
    let _ = write!(
        output,
        "state_directory: state={} path=",
        status.state_directory_state().as_str()
    );
    push_json_path(&mut output, status.state_directory_path());
    output.push('\n');
    output
}

fn json_status(status: &NativeStatus) -> String {
    let mut output = String::from("{\"name\":\"machine-god\",\"version\":");
    push_json_string(&mut output, env!("CARGO_PKG_VERSION"));
    let _ = write!(
        output,
        ",\"engine_api_version\":{},\"permission_mode\":",
        machine_god_native::supported_core_api_version()
    );
    push_json_string(&mut output, status.permission_mode().as_str());
    output.push_str(",\"config_file\":{\"path\":");
    push_json_path(&mut output, status.config_file_path());
    output.push_str(",\"state\":");
    push_json_string(&mut output, status.config_file_state().as_str());
    output.push_str("},\"state_directory\":{\"path\":");
    push_json_path(&mut output, status.state_directory_path());
    output.push_str(",\"state\":");
    push_json_string(&mut output, status.state_directory_state().as_str());
    output.push_str("}}\n");
    output
}

fn push_json_path(output: &mut String, path: Option<&Path>) {
    if let Some(path) = path.and_then(Path::to_str) {
        push_json_string(output, path);
    } else {
        output.push_str("null");
    }
}

fn push_json_string(output: &mut String, value: &str) {
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
        Command, INVALID_ARGUMENTS, OUTPUT_FAILURE, PermissionMode, json_permissions,
        parse_arguments, permissions, push_json_string, run,
    };
    use std::ffi::OsString;
    use std::io;

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
    fn parser_accepts_only_the_documented_grammar() {
        assert_eq!(parse_arguments([]), Ok(Command::Identity));
        for alias in ["help", "--help", "-h"] {
            assert_eq!(parse_arguments([OsString::from(alias)]), Ok(Command::Help));
        }
        for alias in ["--version", "-V"] {
            assert_eq!(
                parse_arguments([OsString::from(alias)]),
                Ok(Command::Identity)
            );
        }
        assert_eq!(
            parse_arguments([OsString::from("permissions")]),
            Ok(Command::Permissions { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("permissions"), OsString::from("--json"),]),
            Ok(Command::Permissions { json: true })
        );
        assert_eq!(
            parse_arguments([OsString::from("status")]),
            Ok(Command::Status { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("status"), OsString::from("--json")]),
            Ok(Command::Status { json: true })
        );

        for arguments in [
            vec![OsString::from("unknown")],
            vec![OsString::from("help"), OsString::from("extra")],
            vec![OsString::from("--json"), OsString::from("status")],
            vec![OsString::from("permissions"), OsString::from("--json=true")],
            vec![
                OsString::from("permissions"),
                OsString::from("--json"),
                OsString::from("--json"),
            ],
            vec![OsString::from("status"), OsString::from("--json=true")],
            vec![
                OsString::from("status"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
        ] {
            assert_eq!(parse_arguments(arguments), Err(()));
        }
    }

    #[test]
    fn permissions_outputs_are_exact() {
        assert_eq!(
            permissions(PermissionMode::Ask, false),
            concat!(
                "machine-god 0.1.0 (engine API 1)\n",
                "permission_mode: ask\n",
                "persistent_rules: unsupported\n",
                "runtime_grants: unavailable\n",
            )
        );
        assert_eq!(
            json_permissions(PermissionMode::Ask),
            concat!(
                "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
                "\"engine_api_version\":1,\"kind\":\"permissions\",",
                "\"permission_mode\":\"ask\",",
                "\"persistent_rules_supported\":false,",
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
        let exit = run([], &mut stdout, &mut stderr);

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

    #[cfg(unix)]
    #[test]
    fn non_unicode_arguments_are_rejected() {
        use std::os::unix::ffi::OsStringExt;

        assert_eq!(parse_arguments([OsString::from_vec(vec![0xff])]), Err(()));
    }
}
