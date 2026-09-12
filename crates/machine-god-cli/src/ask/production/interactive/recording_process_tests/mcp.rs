//! Exact test-child CLI flow with real release helpers, PTY and selected profile.
//! The local Gateway is explicitly injected; no production endpoint is replaced.
use super::{
    Fixture, Gateway, Terminal, bounded_file, frames, launch, recorded_output, sessions, tapes,
};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::Ordering};

fn start(fixture: &Fixture, gateway: &Gateway) -> Terminal {
    let directory = fixture.path("configuration/machine-god");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("mcp.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({"mcp":{"fixture":{
            "type":"http","url":format!("http://{}/mcp", gateway.address), "required":true,
            "startup_timeout_ms":5000,"operation_timeout_ms":5000
        }}}))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut command = launch(fixture, gateway, true);
    command.env("RECORDING_TEST_MCP", "1");
    let mut terminal = Terminal::spawn(&mut command);
    terminal.wait_for(b"stdin excluded]");
    terminal.wait_for(b"> ");
    assert_eq!(methods(gateway), ["server/discover", "tools/list"]);
    terminal.output.clear();
    terminal
}

fn command(terminal: &mut Terminal, text: &str) {
    terminal.output.clear();
    terminal.send(format!("{text}\r").as_bytes());
}

fn methods(gateway: &Gateway) -> Vec<String> {
    gateway
        .mcp_requests()
        .iter()
        .map(|request| request["method"].as_str().unwrap().to_owned())
        .collect()
}

fn pending(terminal: &mut Terminal) {
    command(terminal, "/mcp resource read fixture test://confirm");
    terminal.wait_for(b"MCP process confirmation");
    terminal.wait_for(b"Human feature: resource_read");
    terminal.wait_for(b"/cancel cancels the current operation.\n> ");
}

fn assert_no_command_replay(fixture: &Fixture, gateway: &Gateway, before: &[Value]) {
    let selected = tapes(fixture);
    assert_eq!(selected.len(), 1);
    let frames = frames(&selected[0]); // Actual native replay of a finalized tape.
    assert!(!frames.iter().any(|(kind, _)| *kind == 2));
    let output = recorded_output(&frames);
    assert!(
        output
            .windows(b"MCP process confirmation".len())
            .any(|bytes| bytes == b"MCP process confirmation")
    );
    assert_eq!(gateway.mcp_requests(), before);
}

#[test]
fn mcp_commands_cancel_and_session_switch_record_observations_without_replay() {
    let fixture = Fixture::new();
    let gateway = Gateway::new_with_mcp();
    let mut terminal = start(&fixture, &gateway);
    command(&mut terminal, "/mcp list");
    terminal.wait_for(b"fixture: http; enabled; required\n> ");
    command(&mut terminal, "/mcp resource list fixture");
    terminal.wait_for(b"MCP confirmation resource");
    terminal.wait_for(b"[end of retained MCP observation]\n> ");
    command(&mut terminal, "/mcp resource read fixture test://fixed");
    terminal.wait_for(b"MCP process resource content");
    terminal.wait_for(b"[end of retained MCP observation]\n> ");
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);

    pending(&mut terminal);
    command(&mut terminal, "/cancel");
    terminal.wait_for(b"MCP cancellation acknowledged");
    terminal.wait_for(b"[end of retained MCP observation]\n> ");
    let requests = gateway.mcp_requests();
    let resumed = requests.last().unwrap();
    assert_eq!(
        resumed["params"]["inputResponses"],
        json!({"confirm":{"action":"cancel"}})
    );
    assert!(resumed["id"].as_i64().unwrap() > requests[requests.len() - 2]["id"].as_i64().unwrap());

    // A genuine ordinary turn makes this session eligible for the resume picker.
    command(&mut terminal, "MCP session marker");
    terminal.wait_for(b"[turn completed]\n> ");
    let saved = sessions(&fixture);
    assert_eq!(saved.len(), 1);
    let envelope: Value = serde_json::from_slice(&bounded_file(&saved[0])).unwrap();
    let id = envelope["record"]["id"].as_str().unwrap();
    command(&mut terminal, "/new");
    terminal.wait_for(b": adopted]");
    terminal.wait_for(b"]\n> ");
    assert_eq!(gateway.mcp_requests(), requests);
    command(&mut terminal, "/resume");
    // The ordinary turn makes the sole previous session resumable; it does not
    // rename it. The exact adopted identity is checked through /status below.
    terminal.wait_for("> Untitled session · workspace · ".as_bytes());
    terminal.wait_for(b"1 turns");
    terminal.wait_for(b"Esc cancel");
    terminal.output.clear();
    terminal.send(b"\r");
    terminal.wait_for(b": adopted]");
    terminal.wait_for(b"]\n> ");
    command(&mut terminal, "/status");
    terminal.wait_for(format!("[session] {id}\n").as_bytes());
    terminal.wait_for(b" requested_fast=false\n> \n");
    assert_eq!(
        gateway.mcp_requests(),
        requests,
        "session history is data, not MCP dispatch"
    );
    command(&mut terminal, "/quit");
    assert_eq!(terminal.finish().0.code(), Some(0));
    assert_eq!(gateway.inference.load(Ordering::Acquire), 1);
    assert_no_command_replay(&fixture, &gateway, &requests);
    let persisted: Value = serde_json::from_slice(&bounded_file(&saved[0])).unwrap();
    assert_eq!(
        persisted["record"]["messages"],
        envelope["record"]["messages"]
    );
    gateway.finish();
}

#[test]
fn mcp_pending_human_sigint_settles_real_helpers_and_recording_without_resume() {
    let fixture = Fixture::new();
    let gateway = Gateway::new_with_mcp();
    let mut terminal = start(&fixture, &gateway);
    pending(&mut terminal);
    let before = gateway.mcp_requests();
    terminal.child.signal();
    let (status, physical) = terminal.finish();
    assert_eq!(status.code(), Some(130));
    assert!(physical.windows(8).any(|bytes| bytes == b"\x1b[?2004l"));
    assert_eq!(
        gateway.mcp_requests(),
        before,
        "whole-owner shutdown must not send a modal answer"
    );
    assert_no_command_replay(&fixture, &gateway, &before);
    let selected = tapes(&fixture);
    let frames = frames(&selected[0]);
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
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}

#[test]
fn mcp_pending_human_physical_pty_hangup_reaps_without_fabricating_cancel_reply() {
    let fixture = Fixture::new();
    let gateway = Gateway::new_with_mcp();
    let mut terminal = start(&fixture, &gateway);
    pending(&mut terminal);
    let before = gateway.mcp_requests();
    let status = terminal.hangup();
    assert_eq!(
        status.code(),
        Some(1),
        "hangup is input/output failure, not clean EOF"
    );
    assert_eq!(gateway.mcp_requests(), before);
    assert_no_command_replay(&fixture, &gateway, &before);
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}
