use super::*;
use serde_json::json;

mod composed;

fn command(text: &str) -> NativeAcpCommand {
    parse::classify_text(text).unwrap().unwrap()
}

#[test]
fn supported_native_grammar_rejects_unsupported_and_malformed_locals() {
    for text in [
        "/help",
        "/status",
        "/permissions",
        "/models",
        "/model provider/model",
        "/model effort high",
        "/model save",
        "/fast",
        "/compact",
        "/undo",
        "/allowlist view effective",
        "/skills list",
        "/mcp",
        "/mcp resource list server",
    ] {
        command(text);
    }
    for text in [
        "/clear",
        "/new",
        "/reset",
        "/resume",
        "/copy",
        "/credits",
        "/unknown",
        "/mcp auth server",
        "/mcp add a /bin/false",
        "/skills install example",
        "/allowlist add command x",
        "/permissions yolo",
        "/model save-default",
    ] {
        assert!(parse::classify_text(text).is_err(), "{text}");
    }
    for text in [
        "/undo extra",
        "/help\nextra",
        "/model\nsecret",
        "/skills list extra",
    ] {
        assert!(parse::classify_text(text).is_err());
    }
    for text in ["ordinary text", "/tmp/file.rs has an issue"] {
        assert!(parse::classify_text(text).unwrap().is_none());
    }
    assert!(
        parse::classify_text(&"x".repeat(1_000_000))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        parse::classify_text(&format!("/model {}", "x".repeat(65_536))).unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        parse::classify_text(&format!("{}/help", " ".repeat(65_536))).unwrap_err(),
        Error::Limit
    );
}

#[test]
fn resource_prompt_routing_does_not_read_or_change_canonical_text() {
    let prompt = crate::acp::prompt::decode_prompt_input(&json!({"prompt":[
        {"type":"text","text":"/status"}, {"type":"resource","resource":{"uri":"file:///missing/a"}}
    ]}))
    .unwrap();
    assert!(matches!(
        classify(&prompt).unwrap().unwrap().action,
        Action::Status
    ));
    assert_eq!(prompt.prompt().text, "/status");
    assert_eq!(prompt.resource_targets().len(), 1);
}

#[test]
fn output_preserves_exact_native_data_without_fake_tool_identity() {
    let principal = BackgroundOutputOwner::new(
        SessionId::new("one").unwrap(),
        machine_god_core::SessionIncarnationId::new("inc").unwrap(),
    );
    let data = crate::acp::protocol::decode_value(br#"{"a":-0,"b":1e400}"#).unwrap();
    let result = NativeAcpCommandResult::new(principal, NativeSlashCommand::Mcp, false, data);
    let update = result.update();
    assert_eq!(
        update["command_result"]["receipt"]["a"]
            .as_number()
            .unwrap()
            .as_str(),
        "-0"
    );
    assert_eq!(
        update["command_result"]["receipt"]["b"]
            .as_number()
            .unwrap()
            .as_str(),
        "1e400"
    );
    assert!(update.get("toolCallId").is_none());
    assert!(!result.cancelled());
    assert!(serde_json::to_vec(&update).unwrap().len() <= MAX_ACP_COMMAND_OUTPUT_BYTES);
    assert_eq!(format!("{result:?}"), "NativeAcpCommandResult(..)");
}

#[test]
fn oversized_output_is_explicit_omission_not_unbounded_clone() {
    let principal = BackgroundOutputOwner::new(
        SessionId::new("one").unwrap(),
        machine_god_core::SessionIncarnationId::new("inc").unwrap(),
    );
    let result = NativeAcpCommandResult::new(
        principal,
        NativeSlashCommand::Status,
        false,
        json!({"text":"\u{1}".repeat(MAX_ACP_COMMAND_OUTPUT_BYTES)}),
    );
    assert_eq!(
        result.update()["command_result"]["receipt"]["outputOmitted"],
        true
    );
    assert!(serde_json::to_vec(&result.update()).unwrap().len() < MAX_ACP_COMMAND_OUTPUT_BYTES);
}
