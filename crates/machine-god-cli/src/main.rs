use machine_god_core::SessionId;
use machine_god_native::{PermissionMode, load_process_config};
use std::fmt::Write as _;
use std::{env, ffi::OsString, io, process::ExitCode};
mod ask;
mod background;
mod bounded_output;
mod doctor;
mod models;
mod recording_launch;
mod replay;
mod session;
mod sessions;
mod status;
#[cfg(test)]
mod test_support;
mod workspace;
use ask::{
    AskCommandHost, InteractiveSessionSelection, parse_ask_arguments, parse_prompt_arguments,
    run_ask, run_interactive, run_piped_ask, run_resume,
};
use background::{
    BackgroundCommandHost, ProductionBackgroundCommandHost, is_background_command, run_background,
};
use doctor::{DoctorCommandHost, ProductionDoctorCommandHost, run_doctor};
use models::{ModelsCommandHost, ProductionModelsCommandHost, run_models};
use replay::{ProductionReplayCommandHost, ReplayCommandHost, is_replay_command, run_replay};
use session::{ProductionSessionCommandHost, SessionCommandHost, run_session};
use sessions::{ProductionSessionsCommandHost, SessionsCommandHost, SessionsOptions, run_sessions};
use status::{ProductionStatusCommandHost, StatusCommandHost, is_status_command, run_status};
use workspace::{
    ProductionWorkspaceCommandHost, WorkspaceCommandHost, WorkspaceOptions, run_workspace,
};

const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | ask [--] <prompt...> | background [last | <unsigned-decimal-u64>] [--json] | doctor [--json] | models [--json] | permissions [--json] | replay <tape> [--frames] [--json] [--golden <path>] [--frames-dir <path>] | -r | --resume [last | <id>] | --resume-last | --continue | -c | --resume-<id> | resume [last | <id>] | resume --id <id> | resume --resume --last | session resume [last | <id>] | session resume --id <id> | resume <id> [--] <prompt...> | session <id> [--json] | sessions [--all] [--limit <1-100>] [--cursor <cursor>] [--json] | status [--json] | workspace [list | add <path> | remove <path> | clear] [--json]]\n",
);
const CONFIGURATION_FAILURE: &str = "machine-god: failed to load configuration\n";
const OUTPUT_FAILURE: &str = "machine-god: failed to write output\n";

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Identity,
    Interactive {
        selection: InteractiveSessionSelection,
    },
    Help,
    Ask {
        prompt: String,
    },
    AskStdin,
    Doctor {
        json: bool,
    },
    Models {
        json: bool,
    },
    Permissions {
        json: bool,
    },
    Resume {
        id: SessionId,
        prompt: String,
    },
    Session {
        id: SessionId,
        json: bool,
    },
    Sessions {
        options: SessionsOptions,
    },
    Workspace {
        options: WorkspaceOptions,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionIdGrammar {
    Inspection,
    Resume,
    ResumeExact,
}

fn main() -> ExitCode {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if is_exact_helper_arguments(
        env::args_os().skip(1),
        machine_god_native::INTERACTIVE_INPUT_HELPER_ARGUMENT,
    ) {
        return ExitCode::from(
            if machine_god_native::run_interactive_input_helper().is_ok() {
                0
            } else {
                125
            },
        );
    }
    #[cfg(target_os = "macos")]
    if is_exact_helper_arguments(
        env::args_os().skip(1),
        machine_god_native::PROCESS_INVENTORY_SERVICE_ARGUMENT,
    ) {
        return ExitCode::from(
            if machine_god_native::run_process_inventory_service().is_ok() {
                0
            } else {
                125
            },
        );
    }
    #[cfg(target_os = "macos")]
    if is_exact_helper_arguments(
        env::args_os().skip(1),
        machine_god_native::PROCESS_INVENTORY_HELPER_ARGUMENT,
    ) {
        return ExitCode::from(
            if machine_god_native::run_process_inventory_helper().is_ok() {
                0
            } else {
                125
            },
        );
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if is_exact_helper_arguments(
        env::args_os().skip(1),
        machine_god_native::TERMINAL_CAPTURED_HELPER_ARGUMENT,
    ) {
        let _ = machine_god_native::run_terminal_captured_helper();
        return ExitCode::from(125);
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(arguments) = terminal_tmux_helper_arguments(env::args_os().skip(1)) {
        return ExitCode::from(match arguments {
            Ok(arguments) if machine_god_native::run_terminal_tmux_helper(&arguments).is_ok() => 0,
            _ => 125,
        });
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if is_background_process_helper_arguments(env::args_os().skip(1)) {
        let _ = machine_god_native::run_background_process_helper();
        return ExitCode::from(125);
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if is_exact_helper_arguments(
        env::args_os().skip(1),
        machine_god_native::TERMINAL_PTY_HELPER_ARGUMENT,
    ) {
        let _ = machine_god_native::run_terminal_pty_helper();
        return ExitCode::from(125);
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if is_exact_helper_arguments(
        env::args_os().skip(1),
        machine_god_native::TERMINAL_STARTUP_MARKER_ARGUMENT,
    ) {
        return ExitCode::from(
            if machine_god_native::run_terminal_startup_marker().is_ok() {
                0
            } else {
                125
            },
        );
    }
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    ExitCode::from(run(env::args_os().skip(1), &mut stdout, &mut stderr))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn terminal_tmux_helper_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Option<Result<[OsString; 4], ()>> {
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref()
        != Some(std::ffi::OsStr::new(
            machine_god_native::TERMINAL_TMUX_HELPER_ARGUMENT,
        ))
    {
        return None;
    }
    // Four arguments plus one overflow witness; never collect an unbounded iterator.
    Some(
        arguments
            .take(5)
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| ()),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_background_process_helper_arguments(arguments: impl IntoIterator<Item = OsString>) -> bool {
    is_exact_helper_arguments(
        arguments,
        machine_god_native::BACKGROUND_PROCESS_HELPER_ARGUMENT,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_exact_helper_arguments(
    arguments: impl IntoIterator<Item = OsString>,
    expected: &str,
) -> bool {
    let mut arguments = arguments.into_iter();
    arguments.next().as_deref() == Some(std::ffi::OsStr::new(expected))
        && arguments.next().is_none()
}

/// Explicit borrowed command dependencies; construction does not execute a host.
#[derive(Clone, Copy)]
struct CommandHosts<'a> {
    models: &'a dyn ModelsCommandHost,
    doctor: &'a dyn DoctorCommandHost,
    session: &'a dyn SessionCommandHost,
    sessions: &'a dyn SessionsCommandHost,
    workspace: &'a dyn WorkspaceCommandHost,
    replay: &'a dyn ReplayCommandHost,
    ask: &'a dyn AskCommandHost,
    status: &'a dyn StatusCommandHost,
    background: &'a dyn BackgroundCommandHost,
}

impl Default for CommandHosts<'_> {
    fn default() -> Self {
        Self {
            models: &ProductionModelsCommandHost,
            doctor: &ProductionDoctorCommandHost,
            session: &ProductionSessionCommandHost,
            sessions: &ProductionSessionsCommandHost,
            workspace: &ProductionWorkspaceCommandHost,
            replay: &ProductionReplayCommandHost,
            ask: &ask::PRODUCTION_ASK_HOST,
            status: &ProductionStatusCommandHost,
            background: &ProductionBackgroundCommandHost,
        }
    }
}

fn run(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    run_with_hosts(arguments, stdout, stderr, CommandHosts::default())
}

fn run_with_hosts(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    hosts: CommandHosts<'_>,
) -> u8 {
    let mut arguments = arguments.into_iter().peekable();
    let Ok(launch) = workspace::launch::LaunchWorkspaceOptions::parse(&mut arguments) else {
        let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
        return 2;
    };
    let first = arguments.next();
    if launch.selected()
        && first.as_deref().is_some_and(|first| {
            is_help_command(first)
                || is_background_command(first)
                || is_status_command(first)
                || is_replay_command(first)
        })
    {
        let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
        return 2;
    }
    if first.as_deref().is_some_and(is_help_command) {
        let output = help();
        if stdout.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
            return 1;
        }
        return 0;
    }
    if first.as_deref().is_some_and(is_background_command) {
        return run_background(
            hosts.background,
            arguments,
            stdout,
            stderr,
            INVALID_ARGUMENTS,
            OUTPUT_FAILURE,
        );
    }
    if first.as_deref().is_some_and(is_status_command) {
        let status_arguments = arguments.collect::<Vec<_>>();
        return run_status(
            hosts.status,
            &status_arguments,
            stdout,
            stderr,
            OUTPUT_FAILURE,
        );
    }
    if first.as_deref().is_some_and(is_replay_command) {
        let replay_arguments = arguments.collect::<Vec<_>>();
        return run_replay(
            hosts.replay,
            &replay_arguments,
            stdout,
            stderr,
            OUTPUT_FAILURE,
        );
    }
    let Ok((command, record_requested)) =
        recording_launch::parse(first.into_iter().chain(arguments))
    else {
        let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
        return 2;
    };

    let configured_ask;
    let ask_host = if launch.selected() || record_requested {
        if !matches!(
            command,
            Command::Interactive { .. }
                | Command::Ask { .. }
                | Command::AskStdin
                | Command::Resume { .. }
        ) {
            let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
            return 2;
        }
        let Ok(host) = hosts.ask.with_launch(launch, record_requested) else {
            let _ = stderr.write_all(CONFIGURATION_FAILURE.as_bytes());
            return 1;
        };
        configured_ask = host;
        configured_ask.as_ref()
    } else {
        hosts.ask
    };

    run_parsed_command(
        command,
        stdout,
        stderr,
        CommandHosts {
            ask: ask_host,
            ..hosts
        },
    )
}

fn run_parsed_command(
    command: Command,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    hosts: CommandHosts<'_>,
) -> u8 {
    let CommandHosts {
        models: models_host,
        doctor: doctor_host,
        session: inspection_host,
        sessions: listing_host,
        workspace: workspace_host,
        ask: ask_host,
        ..
    } = hosts;

    let output = match command {
        Command::Identity => identity(),
        Command::Interactive { selection } => {
            return run_interactive(ask_host, selection, stdout, stderr, OUTPUT_FAILURE);
        }
        Command::Help => help(),
        Command::Ask { prompt } => {
            return run_ask(ask_host, prompt, stdout, stderr, OUTPUT_FAILURE);
        }
        Command::AskStdin => {
            return run_piped_ask(ask_host, stdout, stderr, OUTPUT_FAILURE);
        }
        Command::Doctor { json } => {
            return run_doctor(doctor_host, json, stdout, stderr);
        }
        Command::Models { json } => {
            return run_models(models_host, json, stdout, stderr);
        }
        Command::Permissions { json } => {
            let Ok(loaded) = load_process_config() else {
                let _ = stderr.write_all(CONFIGURATION_FAILURE.as_bytes());
                return 1;
            };
            permissions(loaded.config().permission_mode(), json)
        }
        Command::Resume { id, prompt } => {
            return run_resume(ask_host, id, prompt, stdout, stderr, OUTPUT_FAILURE);
        }
        Command::Session { id, json } => {
            return run_session(inspection_host, id, json, stdout, stderr);
        }
        Command::Sessions { options } => {
            return run_sessions(listing_host, &options, stdout, stderr);
        }
        Command::Workspace { options } => {
            return run_workspace(workspace_host, &options, stdout, stderr);
        }
    };

    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn is_help_command(argument: &std::ffi::OsStr) -> bool {
    argument == "help" || argument == "--help" || argument == "-h"
}

fn parse_arguments(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, ()> {
    let mut arguments = arguments.into_iter();
    let Some(first) = arguments.next() else {
        return Ok(Command::Interactive {
            selection: InteractiveSessionSelection::Fresh,
        });
    };
    let Some(first) = first.to_str() else {
        return Err(());
    };

    let command = match first {
        // Help owns the complete tail. Keep the reusable parser consistent
        // with the process entry point, which dispatches this fast path before
        // any command-specific parsing or effects.
        "help" | "--help" | "-h" => return Ok(Command::Help),
        "--version" | "-V" => Command::Identity,
        "ask" => match parse_ask_arguments(arguments.by_ref())? {
            Some(prompt) => Command::Ask { prompt },
            None => Command::AskStdin,
        },
        "doctor" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Doctor { json }
        }
        "models" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Models { json }
        }
        "permissions" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Permissions { json }
        }
        "-r" => Command::Interactive {
            selection: InteractiveSessionSelection::Picker,
        },
        "--resume-last" | "--continue" | "-c" => Command::Interactive {
            selection: InteractiveSessionSelection::Latest,
        },
        "--resume" => {
            let selection = match arguments.next() {
                None => InteractiveSessionSelection::Latest,
                Some(target) => parse_interactive_resume_target(target)?,
            };
            Command::Interactive { selection }
        }
        "resume" => return parse_resume_command(arguments, true),
        "session" => {
            let target = arguments.next().ok_or(())?;
            if target == "resume" {
                return parse_resume_command(arguments, false);
            }
            let id = parse_explicit_session_id(target, SessionIdGrammar::Inspection)?;
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Session { id, json }
        }
        "sessions" => Command::Sessions {
            options: sessions::parse_options(arguments.by_ref())?,
        },
        "workspace" => Command::Workspace {
            options: workspace::parse_options(arguments.by_ref())?,
        },
        alias if alias.starts_with("--resume-") => Command::Interactive {
            selection: InteractiveSessionSelection::Exact(parse_explicit_session_id(
                OsString::from(&alias["--resume-".len()..]),
                SessionIdGrammar::ResumeExact,
            )?),
        },
        _ => return Err(()),
    };

    if arguments.next().is_some() {
        return Err(());
    }
    Ok(command)
}

fn parse_explicit_session_id(
    argument: OsString,
    grammar: SessionIdGrammar,
) -> Result<SessionId, ()> {
    let id = argument.into_string().map_err(|_| ())?;
    let id = match grammar {
        SessionIdGrammar::Inspection => id.as_str(),
        SessionIdGrammar::Resume | SessionIdGrammar::ResumeExact => trim_resume_target(&id),
    };
    let reserved = match grammar {
        SessionIdGrammar::Inspection => matches!(id, "last" | "--id" | "--json"),
        SessionIdGrammar::Resume => id == "last" || id.starts_with('-'),
        SessionIdGrammar::ResumeExact => id.starts_with('-'),
    };
    if reserved {
        return Err(());
    }
    SessionId::new(id).map_err(|_| ())
}

fn identity() -> String {
    format!(
        "machine-god {} (engine API {})\n",
        env!("CARGO_PKG_VERSION"),
        machine_god_native::supported_core_api_version()
    )
}

fn trim_resume_target(target: &str) -> &str {
    target.trim_matches([' ', '\t', '\r', '\n'])
}

fn parse_interactive_resume_target(target: OsString) -> Result<InteractiveSessionSelection, ()> {
    if target.to_str().map(trim_resume_target) == Some("last") {
        Ok(InteractiveSessionSelection::Latest)
    } else {
        parse_explicit_session_id(target, SessionIdGrammar::Resume)
            .map(InteractiveSessionSelection::Exact)
    }
}

fn parse_resume_command(
    mut arguments: impl Iterator<Item = OsString>,
    allow_prompt: bool,
) -> Result<Command, ()> {
    let Some(target) = arguments.next() else {
        return Ok(Command::Interactive {
            selection: InteractiveSessionSelection::Latest,
        });
    };
    if target == "--resume" {
        return if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--last"))
            && arguments.next().is_none()
        {
            Ok(Command::Interactive {
                selection: InteractiveSessionSelection::Latest,
            })
        } else {
            Err(())
        };
    }
    if target == "--id" {
        let id =
            parse_explicit_session_id(arguments.next().ok_or(())?, SessionIdGrammar::ResumeExact)?;
        return if arguments.next().is_none() {
            Ok(Command::Interactive {
                selection: InteractiveSessionSelection::Exact(id),
            })
        } else {
            Err(())
        };
    }
    if target.to_str().map(trim_resume_target) == Some("last") {
        return if arguments.next().is_none() {
            Ok(Command::Interactive {
                selection: InteractiveSessionSelection::Latest,
            })
        } else {
            Err(())
        };
    }
    let id = parse_explicit_session_id(target, SessionIdGrammar::Resume)?;
    let Some(first_prompt) = arguments.next() else {
        return Ok(Command::Interactive {
            selection: InteractiveSessionSelection::Exact(id),
        });
    };
    if !allow_prompt {
        return Err(());
    }
    Ok(Command::Resume {
        id,
        prompt: parse_prompt_arguments(std::iter::once(first_prompt).chain(arguments))?,
    })
}

fn help() -> String {
    format!(
        concat!(
            "machine-god {}\n",
            "Embeddable coding-agent engine\n",
            "\n",
            "Usage:\n",
            "  machine-god\n",
            "  machine-god [<interactive-resume-options>] --record\n",
            "  machine-god [--add-dir PATH | --add-dir=PATH]... [--no-additional-dirs] [ask ... | resume ... | <interactive-resume-options>]\n",
            "  machine-god help\n",
            "  machine-god ask [--] [<prompt...>]\n",
            "  machine-god background [last | <unsigned-decimal-u64>] [--json]\n",
            "  machine-god doctor [--json]\n",
            "  machine-god models [--json]\n",
            "  machine-god permissions [--json]\n",
            "  machine-god replay <tape> [--frames] [--json] [--golden <path>] [--frames-dir <path>]\n",
            "  machine-god resume [last | <id>]\n",
            "  machine-god resume --id <id>\n",
            "  machine-god resume --resume --last\n",
            "  machine-god resume <id> [--] <prompt...>\n",
            "  machine-god session resume [last | <id>]\n",
            "  machine-god session resume --id <id>\n",
            "  machine-god session <id> [--json]\n",
            "  machine-god sessions [--all] [--limit <1-100>] [--cursor <cursor>] [--json]\n",
            "  machine-god status [--json]\n",
            "  machine-god workspace [list | add <path> | remove <path> | clear] [--json]\n",
            "\n",
            "Commands:\n",
            "  help         Show this help\n",
            "  ask          Run one noninteractive prompt\n",
            "  background   Inspect persisted background history\n",
            "  doctor       Run local health and preflight checks\n",
            "  models       List available models\n",
            "  permissions  Show the permission mode and rules\n",
            "  replay       Replay a recorded terminal session\n",
            "  resume       Resume interactively or with one prompt\n",
            "  session      Inspect a saved session\n",
            "  sessions     List saved sessions\n",
            "  status       Show configuration and runtime information\n",
            "  workspace    Manage additional workspace directories\n",
            "\n",
            "Options:\n",
            "  -h, --help       Show this help\n",
            "  -V, --version    Show version\n",
            "  -r               Pick a session to resume\n",
            "  -c, --continue   Resume the latest workspace session\n",
            "  --resume-last    Resume the latest workspace session\n",
            "  --resume [last | <id>]  Resume latest or an exact session\n",
            "  --resume-<id>    Resume an exact session\n",
            "  --add-dir PATH  Add a launch-only workspace directory (repeatable, before command)\n",
            "  --no-additional-dirs  Suppress saved additional directories for this launch\n",
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

fn push_json_string(output: &mut String, value: &str) {
    write_json_string(output, value).expect("writing JSON to a String cannot fail");
}

fn write_json_string(output: &mut impl std::fmt::Write, value: &str) -> std::fmt::Result {
    output.write_char('"')?;
    write_json_string_content(output, value)?;
    output.write_char('"')
}

fn write_json_string_content(output: &mut impl std::fmt::Write, value: &str) -> std::fmt::Result {
    for character in value.chars() {
        match character {
            '"' => output.write_str("\\\"")?,
            '\\' => output.write_str("\\\\")?,
            '\u{08}' => output.write_str("\\b")?,
            '\u{0c}' => output.write_str("\\f")?,
            '\n' => output.write_str("\\n")?,
            '\r' => output.write_str("\\r")?,
            '\t' => output.write_str("\\t")?,
            '\u{00}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{061c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => {
                write!(output, "\\u{:04x}", character as u32)?;
            }
            _ => output.write_char(character)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
