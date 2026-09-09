//! Production interactive flow under an owned PTY and injected local Gateway.

mod gateway;
mod host;
mod resume;
mod support;

use gateway::Gateway;
use std::{fs, path::Path, sync::atomic::Ordering};
use support::{Fixture, Terminal, bounded_file};

#[test]
#[ignore = "owned subprocess fixture invoked by recording process scenarios"]
fn recording_process_child() {
    assert!(std::env::var_os("RECORDING_TEST_ROOT").is_some());
    let host = host::Host {
        required: std::env::var("RECORDING_TEST_REQUIRED").unwrap() == "1",
    };
    let selection = match std::env::var("RECORDING_TEST_SELECTION").as_deref() {
        Err(std::env::VarError::NotPresent) => crate::ask::InteractiveSessionSelection::Fresh,
        Ok("latest") => crate::ask::InteractiveSessionSelection::Latest,
        Ok("exact") => crate::ask::InteractiveSessionSelection::Exact(
            machine_god_core::SessionId::new(std::env::var("RECORDING_TEST_SESSION").unwrap())
                .unwrap(),
        ),
        other => panic!("invalid explicit subprocess selection: {other:?}"),
    };
    let code = crate::ask::run_interactive(
        &host,
        selection,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
        "machine-god: failed to write output\n",
    );
    std::process::exit(i32::from(code));
}

fn launch(fixture: &Fixture, gateway: &Gateway, required: bool) -> std::process::Command {
    let mut command = fixture.command();
    command
        .env("RECORDING_TEST_GATEWAY", gateway.address.to_string())
        .env("RECORDING_TEST_REQUIRED", if required { "1" } else { "0" });
    command
}

fn tapes(fixture: &Fixture) -> Vec<std::path::PathBuf> {
    let root = fixture.state.join("machine-god/recordings");
    if !root.exists() {
        return Vec::new();
    }
    let entries: Vec<_> = fs::read_dir(root).unwrap().take(9).collect();
    assert!(entries.len() < 9);
    entries
        .into_iter()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn sessions(fixture: &Fixture) -> Vec<std::path::PathBuf> {
    let root = fixture.state.join("machine-god");
    if !root.exists() {
        return Vec::new();
    }
    let entries: Vec<_> = fs::read_dir(root).unwrap().take(129).collect();
    assert!(entries.len() < 129);
    entries
        .into_iter()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .is_some_and(|name| name.starts_with("session-"))
                && path.extension() == Some(std::ffi::OsStr::new("json"))
        })
        .collect()
}

fn frames(path: &Path) -> Vec<(u8, Vec<u8>)> {
    let bytes = bounded_file(path);
    assert_eq!(&bytes[..5], b"FXTP\x01");
    assert_eq!(&bytes[5..9], &[80, 0, 24, 0]);
    let mut offset = 18 + usize::from(bytes[17]);
    let mut frames = Vec::new();
    while offset < bytes.len() {
        assert!(offset + 9 <= bytes.len(), "complete frame header");
        let kind = bytes[offset + 4];
        let len = u32::from_le_bytes(bytes[offset + 5..offset + 9].try_into().unwrap()) as usize;
        offset += 9;
        assert!(offset + len <= bytes.len(), "complete frame payload");
        frames.push((kind, bytes[offset..offset + len].to_vec()));
        offset += len;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let replay = runtime
        .block_on(machine_god_native::replay_terminal_tape(
            machine_god_native::TerminalTapeReplayRequest::new(
                path.to_owned(),
                false,
                true,
                None,
                None,
            ),
            machine_god_core::CancellationToken::new(),
        ))
        .unwrap();
    let summary: serde_json::Value = serde_json::from_slice(replay.stdout()).unwrap();
    assert_eq!(summary["frame_count"], frames.len());
    frames
}

fn recorded_output(frames: &[(u8, Vec<u8>)]) -> Vec<u8> {
    frames
        .iter()
        .filter(|(kind, _)| *kind == 1)
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect()
}

#[test]
fn automatic_recording_captures_gateway_answer_and_final_presentation_without_stdin() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let mut terminal = Terminal::spawn(&mut launch(&fixture, &gateway, true));
    terminal.wait_for(b"stdin excluded]");
    terminal.wait_for(b"> ");
    terminal.send(b"record this prompt\r");
    terminal.wait_for(b"local fixture answer");
    terminal.send(b"/quit\r");
    let (status, _) = terminal.finish();
    assert_eq!(status.code(), Some(0));
    assert_eq!(gateway.inference.load(Ordering::Acquire), 1);
    let tapes = tapes(&fixture);
    assert_eq!(tapes.len(), 1);
    let frames = frames(&tapes[0]);
    assert!(!frames.iter().any(|(kind, _)| *kind == 2));
    assert!(
        frames
            .iter()
            .any(|(kind, payload)| *kind == 5 && payload == b"machine-god:interactive")
    );
    let output = recorded_output(&frames);
    assert!(
        output
            .windows(b"local fixture answer".len())
            .any(|bytes| bytes == b"local fixture answer")
    );
    assert!(
        output
            .windows(b"session closed".len())
            .any(|bytes| bytes == b"session closed")
    );
    assert!(output.windows(8).any(|bytes| bytes == b"\x1b[?2004l"));
    assert_eq!(sessions(&fixture).len(), 1);
    gateway.finish();
}

#[test]
fn explicit_environment_recording_includes_raw_input_only_with_opt_in() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let path = fixture.path("explicit.fxtape");
    let mut command = launch(&fixture, &gateway, false);
    command.env("FX_RECORD", &path).env("FX_RECORD_INPUT", "ON");
    let mut terminal = Terminal::spawn(&mut command);
    terminal.wait_for(b"stdin included]");
    terminal.wait_for(b"> ");
    terminal.send(b"record this prompt\r");
    terminal.wait_for(b"local fixture answer");
    terminal.send(b"/quit\r");
    assert_eq!(terminal.finish().0.code(), Some(0));
    let frames = frames(&path);
    let input: Vec<_> = frames
        .iter()
        .filter(|(kind, _)| *kind == 2)
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect();
    assert_eq!(input, b"record this prompt\r/quit\r");
    assert!(tapes(&fixture).is_empty());
    gateway.finish();
}

#[test]
fn requested_existing_tape_is_fatal_before_session_creation_and_never_overwritten() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let path = fixture.path("existing.fxtape");
    fs::write(&path, b"existing contents").unwrap();
    let mut command = launch(&fixture, &gateway, true);
    command.env("FX_RECORD", &path);
    let (status, output) = Terminal::spawn(&mut command).finish();
    assert_eq!(status.code(), Some(1));
    assert!(
        output
            .windows(b"interactive request failed".len())
            .any(|bytes| bytes == b"interactive request failed")
    );
    assert_eq!(bounded_file(&path), b"existing contents");
    assert!(sessions(&fixture).is_empty());
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}

#[test]
fn optional_environment_failure_is_visible_and_does_not_claim_active_recording() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let path = fixture.path("existing.fxtape");
    fs::write(&path, b"existing contents").unwrap();
    let mut command = launch(&fixture, &gateway, false);
    command.env("FX_RECORD", &path);
    let mut terminal = Terminal::spawn(&mut command);
    terminal.wait_for(b"recording unavailable; continuing without a tape");
    terminal.wait_for(b"> ");
    terminal.send(b"/quit\r");
    let (status, output) = terminal.finish();
    assert_eq!(status.code(), Some(0));
    assert!(
        !output
            .windows(b"stdin excluded".len())
            .any(|bytes| bytes == b"stdin excluded")
    );
    assert_eq!(bounded_file(&path), b"existing contents");
    assert_eq!(sessions(&fixture).len(), 1);
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}

#[test]
fn sigint_preserves_exit_status_records_signal_and_closes_tape_and_input_helpers() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let mut terminal = Terminal::spawn(&mut launch(&fixture, &gateway, true));
    terminal.wait_for(b"stdin excluded]");
    terminal.wait_for(b"> ");
    terminal.child.signal();
    assert_eq!(terminal.finish().0.code(), Some(130));
    let tapes = tapes(&fixture);
    assert_eq!(tapes.len(), 1);
    let frames = frames(&tapes[0]);
    assert!(
        frames
            .iter()
            .any(|(kind, bytes)| *kind == 4 && bytes.is_empty())
    );
    assert!(
        recorded_output(&frames)
            .windows(8)
            .any(|bytes| bytes == b"\x1b[?2004l")
    );
    gateway.finish();
}

#[test]
fn disabled_recording_creates_no_tape_or_recording_directory() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let mut terminal = Terminal::spawn(&mut launch(&fixture, &gateway, false));
    terminal.wait_for(b"> ");
    terminal.send(b"/quit\r");
    assert_eq!(terminal.finish().0.code(), Some(0));
    assert!(!fixture.state.join("machine-god/recordings").exists());
    assert!(tapes(&fixture).is_empty());
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}
