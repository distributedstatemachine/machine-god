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
