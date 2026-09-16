//! Managed navigation through physical input, output, process exit and restart.
use super::{Fixture, Gateway, Terminal, launch};
use std::sync::atomic::Ordering;

const NAME: &str = "pty-worker";
const PARENT: &[u8] = b"parent draft stays";

fn open(terminal: &mut Terminal) {
    terminal.output.clear();
    terminal.send(b"\x18");
    terminal.wait_for(b"Catalog(Current)\r\n");
    terminal.wait_for(b"Arrows select/scroll");
}

fn line(terminal: &mut Terminal, text: &str) {
    terminal.output.clear();
    terminal.send(text.as_bytes());
    // The edited composer must be physically presented before Enter can use
    // its new native frame. A same-chunk Enter deliberately grants no ACK.
    terminal.wait_for(text.as_bytes());
    terminal.output.clear();
    terminal.send(b"\r");
}

fn create(terminal: &mut Terminal) {
    line(
        terminal,
        &format!("/create {{\"name\":\"{NAME}\",\"mode\":\"persistent\"}}"),
    );
    terminal.wait_for(b"Created");
    line(terminal, "/refresh");
    terminal.wait_for(b"pty-worker [Idle, g1]");
}

fn parent(terminal: &mut Terminal) {
    terminal.output.clear();
    terminal.send(b"\x18");
    terminal.wait_for(b"> ");
}

fn finish(mut terminal: Terminal) {
    terminal.output.clear();
    terminal.send(b"/quit\r");
    assert_eq!(terminal.finish().0.code(), Some(0));
}

#[test]
fn release_cli_managed_create_archive_and_reopen_survive_process_restart() {
    let fixture = Fixture::new();
    let mut command = fixture.release_command();
    assert_eq!(
        command.get_args().count(),
        0,
        "actual production entrypoint"
    );
    let mut terminal = Terminal::spawn(&mut command);
    terminal.wait_for(b"> ");
    terminal.output.clear();
    terminal.send(PARENT);
    terminal.wait_for(PARENT);
    open(&mut terminal);
    create(&mut terminal);
    parent(&mut terminal);
    terminal.wait_for(PARENT);
    terminal.output.clear();
    terminal.send(b"\x03");
    terminal.wait_for(b"> ");
    finish(terminal);

    // A new real CLI must recover the durable catalog without replaying work.
    let mut terminal = Terminal::spawn(&mut fixture.release_command());
    terminal.wait_for(b"> ");
    open(&mut terminal);
    terminal.wait_for(b"pty-worker [Idle, g1]");
    line(&mut terminal, "/close");
    terminal.wait_for(b"Close and archive this agent?");
    line(&mut terminal, "/confirm");
    terminal.wait_for(b"LifecycleChanged");
    terminal.wait_for(b"Archived");
    parent(&mut terminal);
    finish(terminal);

    let mut terminal = Terminal::spawn(&mut fixture.release_command());
    terminal.wait_for(b"> ");
    open(&mut terminal);
    line(&mut terminal, "/archived");
    terminal.wait_for(b"pty-worker [Archived, g1]");
    line(&mut terminal, "/reopen");
    terminal.wait_for(b"LifecycleChanged");
    // Reopen advances generation; select it afresh instead of reusing an old
    // editor, observation or draft as authority for the replacement runtime.
    line(&mut terminal, "/current");
    terminal.wait_for(b"pty-worker [Idle, g2]");
    parent(&mut terminal);
    finish(terminal);
}

fn user_text(request: &serde_json::Value) -> Vec<&str> {
    request["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"][0]["text"].as_str().unwrap())
        .collect()
}

#[test]
fn managed_pty_child_turn_preserves_the_parent_draft_and_independent_context() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let mut terminal = Terminal::spawn(&mut launch(&fixture, &gateway, false));
    terminal.wait_for(b"> ");
    terminal.output.clear();
    terminal.send(PARENT);
    terminal.wait_for(PARENT);
    open(&mut terminal);
    create(&mut terminal);
    terminal.output.clear();
    terminal.send(b"\r");
    terminal.wait_for(b"canonical conversation");
    line(&mut terminal, "child prompt only");
    terminal.wait_for(b"local fixture answer");
    assert_eq!(gateway.inference.load(Ordering::Acquire), 1);
    let requests = gateway.requests();
    assert_eq!(user_text(&requests[0]), ["child prompt only"]);
    assert!(!requests[0].to_string().contains("parent draft stays"));

    parent(&mut terminal);
    terminal.wait_for(PARENT);
    terminal.output.clear();
    terminal.send(b"\r");
    terminal.wait_for(b"local fixture answer");
    finish(terminal);
    let requests = gateway.requests();
    assert_eq!(requests.len(), 2, "no automatic idle-parent model turn");
    assert_eq!(user_text(&requests[1]), ["parent draft stays"]);
    gateway.finish();
}
