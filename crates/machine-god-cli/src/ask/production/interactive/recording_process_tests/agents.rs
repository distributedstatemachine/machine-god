//! Managed navigation through physical input, output, process exit and restart.
use super::{Fixture, Gateway, Terminal, launch};
use std::sync::atomic::Ordering;

const NAME: &str = "pty-worker";
const PARENT: &[u8] = b"parent draft stays";

fn catalog(terminal: &mut Terminal, filter: &str, row: Option<&str>) {
    const HEADING: &str = "Agents & processes · clipped previews\r\n";
    const END: &str = "Arrows select/scroll · /create /configure /processes\r\n\r\x1b[2K> \x1b[3G";
    let heading = format!("{HEADING}Catalog({filter})\r\n");
    // Observe one complete, nonbusy frame, not a row/footer left by a pending
    // catalog. The final composer is physical output, not a flush ACK; the
    // driver's exact-frame deferred-selection tests cover that separate race.
    terminal.wait_for_output("complete nonbusy managed catalog", |output| {
        let Some(start) = output
            .windows(HEADING.len())
            .rposition(|window| window == HEADING.as_bytes())
        else {
            return false;
        };
        let frame = &output[start..];
        frame.starts_with(heading.as_bytes())
            && frame.ends_with(END.as_bytes())
            && row.is_none_or(|row| frame.windows(row.len()).any(|part| part == row.as_bytes()))
    });
}

fn open(terminal: &mut Terminal) {
    terminal.output.clear();
    terminal.send(b"\x18");
    catalog(terminal, "Current", None);
}

fn line(terminal: &mut Terminal, text: &str) {
    terminal.output.clear();
    terminal.send(text.as_bytes());
    // Observe the edited composer before submitting it. A same-chunk Enter
    // deliberately grants no ACK.
    // Slash commands also occur in the menu footer; observe the complete edited
    // composer, including its cursor placement, without mistaking it for an ACK.
    assert!(text.is_ascii() && text.len() < 76);
    let composer = format!("\r\x1b[2K> {text}\x1b[{}G", text.len() + 3);
    terminal.wait_for(composer.as_bytes());
    terminal.output.clear();
    terminal.send(b"\r");
}

fn create(terminal: &mut Terminal) {
    line(
        terminal,
        &format!("/create {{\"name\":\"{NAME}\",\"mode\":\"persistent\"}}"),
    );
    terminal.wait_for(b"Created");
    catalog(terminal, "Current", Some("pty-worker [Idle, g1]"));
    line(terminal, "/refresh");
    catalog(terminal, "Current", Some("pty-worker [Idle, g1]"));
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
    catalog(&mut terminal, "Current", Some("pty-worker [Idle, g1]"));
    line(&mut terminal, "/close");
    terminal.wait_for(b"ConfirmClose\r\n");
    terminal.wait_for(b"Close and archive this agent?");
    line(&mut terminal, "/confirm");
    terminal.wait_for(b"LifecycleChanged");
    terminal.wait_for(b"Agent(Status)\r\n");
    line(&mut terminal, "/archived");
    catalog(&mut terminal, "Archived", Some("pty-worker [Archived, g1]"));
    parent(&mut terminal);
    finish(terminal);

    let mut terminal = Terminal::spawn(&mut fixture.release_command());
    terminal.wait_for(b"> ");
    open(&mut terminal);
    line(&mut terminal, "/archived");
    catalog(&mut terminal, "Archived", Some("pty-worker [Archived, g1]"));
    line(&mut terminal, "/reopen");
    terminal.wait_for(b"LifecycleChanged");
    catalog(&mut terminal, "Archived", None);
    // Reopen advances generation; select it afresh instead of reusing an old
    // editor, observation or draft as authority for the replacement runtime.
    line(&mut terminal, "/current");
    catalog(&mut terminal, "Current", Some("pty-worker [Idle, g2]"));
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
    terminal.wait_for(b"History rows ");
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
