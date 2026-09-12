//! Pure shared preflight/accounting checks, not native operation-authority doubles.

use super::*;
use crate::mcp::mrtr::{McpInputRequired, McpMrtrLimits};
use serde_json::json;

fn required(value: &serde_json::Value) -> McpInputRequired {
    let raw = RawValue::from_string(serde_json::to_string(value).unwrap()).unwrap();
    McpInputRequired::parse(&raw, McpMrtrLimits::default()).unwrap()
}

#[test]
fn preflight_rejects_empty_or_unsupported_sets_before_any_partial_prompt() {
    for value in [
        json!({"requestState":null}),
        json!({"inputRequests":{}}),
        json!({"inputRequests":{"roots":{"method":"roots/list"}}}),
        json!({"inputRequests":{
            "form":{"method":"elicitation/create","params":{
                "message":"Continue?","requestedSchema":{"type":"object","properties":{}}}},
            "roots":{"method":"roots/list"}
        }}),
    ] {
        let required = required(&value);
        assert!(!supported(&required, false));
        assert!(!supported(&required, true));
    }
}

#[test]
fn forms_and_urls_share_preflight_without_inventing_browser_availability() {
    let form = json!({"method":"elicitation/create","params":{
        "message":"Continue?","requestedSchema":{"type":"object","properties":{}}}});
    let url = json!({"method":"elicitation/create","params":{
        "mode":"url","message":"Open?","url":"https://example.com/continue"}});
    let only_form = required(&json!({"inputRequests":{"form":form.clone()}}));
    assert!(supported(&only_form, false));
    assert!(supported(&only_form, true));
    for value in [
        json!({"inputRequests":{"url":url.clone()}}),
        json!({"inputRequests":{"form":form,"url":url}}),
    ] {
        let required = required(&value);
        assert!(!supported(&required, false));
        assert!(supported(&required, true));
    }
}

#[test]
fn response_map_charge_includes_escaped_keys_and_rejects_overflow() {
    let answer = RawValue::from_string(r#"{"action":"cancel"}"#.into()).unwrap();
    let key = "quoted\\\"key\n";
    let mut bytes = 1;
    charge_response(&mut bytes, key, &answer).unwrap();
    let map = BTreeMap::from([(key, answer.as_ref())]);
    assert_eq!(bytes, serde_json::to_string(&map).unwrap().len());
    let remaining = 128 * 1024 - bytes;
    bytes += remaining;
    let original = bytes;
    assert!(charge_response(&mut bytes, "next", &answer).is_err());
    assert_eq!(bytes, original);
    bytes = usize::MAX;
    assert!(charge_response(&mut bytes, "next", &answer).is_err());
    assert_eq!(bytes, usize::MAX);
}

#[test]
fn exact_response_map_ceiling_is_admitted_without_an_extra_byte() {
    let key = "key";
    let overhead = 1 + serde_json::to_string(key).unwrap().len() + 2 + 2;
    let answer =
        RawValue::from_string(format!("\"{}\"", "x".repeat(128 * 1024 - overhead))).unwrap();
    let mut bytes = 1;
    charge_response(&mut bytes, key, &answer).unwrap();
    assert_eq!(bytes, 128 * 1024);
    let longer =
        RawValue::from_string(format!("\"{}\"", "x".repeat(128 * 1024 - overhead + 1))).unwrap();
    bytes = 1;
    assert!(charge_response(&mut bytes, key, &longer).is_err());
    assert_eq!(bytes, 1);
}

#[test]
fn dropping_prompt_guard_cancels_only_the_selected_token() {
    let selected = CancellationToken::new();
    let unrelated = CancellationToken::new();
    drop(PromptCancellation(selected.clone()));
    assert!(selected.is_cancelled());
    assert!(!unrelated.is_cancelled());
}
