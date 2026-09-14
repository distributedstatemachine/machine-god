//! Production transport checks reuse the CLI's owned subprocess harness.

use super::*;
use serde_json::Value;

const INITIALIZE: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1}}\n";

fn command(directory: &TestDirectory) -> Command {
    let mut command = machine_god();
    command
        .arg("acp")
        .current_dir(directory.path())
        .env_clear()
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn messages(output: &Output) -> Vec<Value> {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(output.stdout.len() < 64 * 1024);
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn acp_initialize_and_eof_do_not_read_profile_or_acquire_session_authority() {
    let directory = TestDirectory::new("acp-initialize");
    let profile = directory.path().join("config/machine-god");
    fs::create_dir_all(&profile).unwrap();
    for file in ["config.json", "mcp.json", "mcp-credentials.json"] {
        fs::write(profile.join(file), b"invalid profile sentinel").unwrap();
    }
    let mut child = ScopedChild::spawn(&mut command(&directory));
    child.close_stdin_with(INITIALIZE);
    let output = child.wait_with_output(Duration::from_secs(10));
    let frames = messages(&output);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["id"], 1);
    assert_eq!(frames[0]["result"]["protocolVersion"], 1);
    assert!(!directory.path().join("state").exists());
    for file in ["config.json", "mcp.json", "mcp-credentials.json"] {
        assert_eq!(
            fs::read(profile.join(file)).unwrap(),
            b"invalid profile sentinel"
        );
    }
    assert_eq!(fs::read_dir(profile).unwrap().count(), 3);
}

/// One reader owns stdout, so a readable poll followed by a one-byte read cannot
/// lose readiness to another consumer. No detached reader outlives the child.
fn read_frame(child: &mut ScopedChild, deadline: Instant) -> Value {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let stdout = child.child_mut().stdout.as_ref().expect("piped stdout");
    let mut frame = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "ACP response deadline elapsed");
        let mut descriptors = [PollFd::new(stdout, PollFlags::IN)];
        match poll(
            &mut descriptors,
            Some(&Timespec::try_from(remaining).unwrap()),
        ) {
            Err(rustix::io::Errno::INTR) => continue,
            Ok(0) => panic!("ACP response deadline elapsed"),
            result => {
                result.unwrap();
                assert!(
                    descriptors[0]
                        .revents()
                        .intersects(PollFlags::IN | PollFlags::HUP),
                    "ACP stdout was not readable"
                );
            }
        }
        let mut byte = [0];
        match rustix::io::read(stdout, &mut byte) {
            Err(rustix::io::Errno::INTR) => continue,
            Ok(0) => panic!("ACP stdout closed before a complete response"),
            result => assert_eq!(result.unwrap(), 1),
        }
        if byte[0] == b'\n' {
            return serde_json::from_slice(&frame).expect("one JSON-RPC response frame");
        }
        assert!(frame.len() < 64 * 1024, "unexpected ACP response size");
        frame.push(byte[0]);
    }
}

fn exchange(child: &mut ScopedChild, request: &[u8], deadline: Instant) -> Value {
    // Requests are tiny and strictly sequential: the previous response proves
    // its input was consumed before the next write, keeping the pipe empty.
    assert!(request.len() <= 512);
    assert!(Instant::now() < deadline);
    std::io::Write::write_all(
        child.child_mut().stdin.as_mut().expect("piped stdin"),
        request,
    )
    .unwrap();
    read_frame(child, deadline)
}

#[test]
fn acp_persistent_listing_is_read_only_and_recovers_after_correlated_request_errors() {
    let directory = TestDirectory::new("acp-persistent-list");
    let profile = directory.path().join("config/machine-god");
    fs::create_dir_all(&profile).unwrap();
    let files = ["config.json", "mcp.json", "mcp-credentials.json"];
    let sentinel = b"invalid profile sentinel";
    for file in files {
        fs::write(profile.join(file), sentinel).unwrap();
    }
    let assert_untouched = || {
        assert!(!directory.path().join("state").exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(profile.parent().unwrap()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&profile).unwrap().count(), files.len());
        for file in files {
            assert_eq!(fs::read(profile.join(file)).unwrap(), sentinel);
        }
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut child = ScopedChild::spawn(&mut command(&directory));
    let initialized = exchange(&mut child, INITIALIZE, deadline);
    assert_eq!(initialized["jsonrpc"], "2.0");
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["protocolVersion"], 1);
    assert!(initialized.get("error").is_none());
    for (id, request) in [
        (
            "first-list",
            b"{\"jsonrpc\":\"2.0\",\"id\":\"first-list\",\"method\":\"session/list\"}\n".as_slice(),
        ),
        (
            "second-list",
            b"{\"jsonrpc\":\"2.0\",\"id\":\"second-list\",\"method\":\"session/list\",\"params\":{}}\n".as_slice(),
        ),
    ] {
        let response = exchange(&mut child, request, deadline);
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], id);
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["sessions"], serde_json::json!([]));
        assert!(response["result"].get("nextCursor").is_none());
        let metadata = &response["result"]["_meta"]["machineGod"];
        assert_eq!(metadata["scanComplete"], true);
        assert_eq!(metadata["resultsTruncated"], false);
        assert_eq!(metadata["skippedInvalid"], 0);
        assert_eq!(metadata["omittedWorkspace"], 0);
        assert_eq!(metadata["omittedUpdatedAt"], 0);
        assert_untouched();
        if id == "first-list" {
            for (error_id, code, request) in [
                (
                    "invalid-params",
                    -32602,
                    b"{\"jsonrpc\":\"2.0\",\"id\":\"invalid-params\",\"method\":\"session/list\",\"params\":{\"cwd\":false}}\n".as_slice(),
                ),
                (
                    "unsupported",
                    -32601,
                    b"{\"jsonrpc\":\"2.0\",\"id\":\"unsupported\",\"method\":\"session/not-supported\",\"params\":{}}\n".as_slice(),
                ),
            ] {
                let response = exchange(&mut child, request, deadline);
                assert_eq!(response["jsonrpc"], "2.0");
                assert_eq!(response["id"], error_id);
                assert_eq!(response["error"]["code"], code);
                assert!(response.get("result").is_none());
            }
        }
    }
    child.close_stdin_with(b"");
    let remaining = deadline.saturating_duration_since(Instant::now());
    assert!(!remaining.is_zero(), "ACP shutdown deadline elapsed");
    let output = child.wait_with_output(remaining);
    assert!(messages(&output).is_empty(), "unexpected trailing response");
    assert_untouched();
}

#[test]
fn acp_empty_eof_is_clean_and_legacy_version_has_no_fallback() {
    let directory = TestDirectory::new("acp-eof-version");
    let mut child = ScopedChild::spawn(&mut command(&directory));
    child.close_stdin_with(b"");
    assert!(messages(&child.wait_with_output(Duration::from_secs(10))).is_empty());
    let mut child = ScopedChild::spawn(&mut command(&directory));
    child.close_stdin_with(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\",\"params\":{\"protocolVersion\":0}}\n");
    let frames = messages(&child.wait_with_output(Duration::from_secs(10)));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["id"], 2);
    assert!(frames[0].get("error").is_some());
    assert!(frames[0].get("result").is_none());
    assert!(!directory.path().join("state").exists());
}

#[test]
fn acp_preserves_the_shared_input_pipe_open_file_description_flags() {
    use std::io::Write as _;
    let directory = TestDirectory::new("acp-pipe-flags");
    let (reader, writer) = std::io::pipe().unwrap();
    let flags = rustix::fs::fcntl_getfl(&reader).unwrap();
    let inherited = reader.try_clone().unwrap();
    let mut selected = command(&directory);
    selected.stdin(Stdio::from(inherited));
    let child = ScopedChild::spawn(&mut selected);
    let mut writer = writer;
    writer.write_all(INITIALIZE).unwrap();
    drop(writer);
    let frames = messages(&child.wait_with_output(Duration::from_secs(10)));
    assert_eq!(frames.len(), 1);
    assert_eq!(rustix::fs::fcntl_getfl(&reader).unwrap(), flags);
}

#[test]
fn acp_blocked_stdout_and_backpressured_complete_requests_do_not_hide_pipe_disconnect() {
    use std::io::Write as _;
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let directory = TestDirectory::new("acp-blocked-disconnect");
    let (mut stdout, _undrained_peer) = UnixStream::pair().unwrap();
    rustix::net::sockopt::set_socket_send_buffer_size(&stdout, 1024).unwrap();
    // Fill this fixture-owned output endpoint before handing it to the child,
    // so saturation does not depend on platform socket-buffer minimums.
    stdout.set_nonblocking(true).unwrap();
    let mut filled = 0;
    loop {
        match stdout.write(&[0; 1024]) {
            Ok(0) => panic!("fixture output closed"),
            Ok(count) => {
                filled += count;
                assert!(filled <= 1024 * 1024, "fixture output did not saturate");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => panic!("fixture output unavailable: {error}"),
        }
    }
    stdout.set_nonblocking(false).unwrap();
    let (read, mut write) = std::io::pipe().unwrap();
    let original = rustix::fs::fcntl_getfl(&read).unwrap();
    let mut selected = command(&directory);
    selected
        .stdin(Stdio::from(read.try_clone().unwrap()))
        .stdout(Stdio::from(OwnedFd::from(stdout)));
    let child = ScopedChild::spawn(&mut selected);
    // The first response blocks, the second occupies the native reply lane,
    // and the third complete request remains with transport backpressure.
    // A single <= PIPE_BUF write is bounded even before child startup.
    let requests = INITIALIZE.repeat(3);
    assert!(requests.len() <= 4096);
    write.write_all(&requests).unwrap();
    drop(write);
    let output = child.wait_with_output(Duration::from_secs(10));
    assert_eq!(
        output.status.code(),
        Some(1),
        "blocked output must be a failure: {output:?}"
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(rustix::fs::fcntl_getfl(&read).unwrap(), original);
    assert!(!directory.path().join("state").exists());
}
