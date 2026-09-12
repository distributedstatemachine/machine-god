use super::*;
use crate::mcp::mrtr::{McpInputRequired, McpMrtrLimits};
use serde_json::value::RawValue;

fn input(state: &str) -> McpInputRequired {
    let raw = RawValue::from_string(format!(
        r#"{{"resultType":"input_required","inputRequests":{{"ask":{{"method":"elicitation/create","params":{{"message":"Choose","requestedSchema":{{"type":"object","properties":{{"n":{{"type":"number"}}}},"required":["n"]}}}}}}}}{state}}}"#
    )).unwrap();
    McpInputRequired::parse(&raw, McpMrtrLimits::default()).unwrap()
}
fn answers(input: &McpInputRequired) -> crate::mcp::mrtr::McpValidatedResponses {
    input
        .validate_responses(
            &RawValue::from_string(
                r#"{"ask":{"action":"accept","content":{"n":9007199254740993}}}"#.into(),
            )
            .unwrap(),
        )
        .unwrap()
}

#[test]
fn read_and_get_continue_original_identity_metadata_and_exact_raw_state() {
    for command in [
        "resource read srv test://fixed",
        r#"prompt get srv review {"topic":"9007199254740993"}"#,
    ] {
        let original = McpFeatureExchange::prepare(
            &request(command),
            "srv",
            &catalogs(),
            options(ProtocolVersion::Modern, 7)
                .with_progress_token(91)
                .with_elicitation(true, true),
            None,
            McpFeatureCodecLimits::default(),
        )
        .unwrap();
        let before = machine_god_core::json::from_str(original.wire_json().get()).unwrap();
        let input = input(
            r#", "requestState":{"amount":1e-99999,"signed":-0,"opaque":{"$serde_json::private::Number":"literal"}}"#,
        );
        let continued = original
            .continue_with(19, &input, &answers(&input))
            .unwrap();
        let wire = continued.wire_json().get();
        assert!(wire.contains("1e-99999"));
        assert!(wire.contains("9007199254740993"));
        assert!(wire.contains(r#""signed":-0"#));
        let after = machine_god_core::json::from_str(wire).unwrap();
        assert_eq!(after["id"], 19);
        assert_eq!(before["method"], after["method"]);
        assert_eq!(before["params"]["_meta"], after["params"]["_meta"]);
        for field in ["uri", "name", "arguments"] {
            assert_eq!(before["params"][field], after["params"][field]);
        }
        assert_eq!(continued.request().server(), "srv");
    }
}

#[test]
fn state_absence_and_explicit_null_remain_distinct() {
    for (state, expected) in [("", false), (r#", "requestState":null"#, true)] {
        let input = input(state);
        let continued = exchange("resource read srv test://fixed")
            .continue_with(8, &input, &answers(&input))
            .unwrap();
        let wire = machine_god_core::json::from_str(continued.wire_json().get()).unwrap();
        assert_eq!(
            wire["params"]
                .as_object()
                .unwrap()
                .contains_key("requestState"),
            expected
        );
        assert!(
            wire["params"]
                .as_object()
                .unwrap()
                .contains_key("inputResponses")
        );
    }
}

#[test]
fn continuation_rejects_non_read_get_actions_and_nonfresh_ids() {
    let input = input("");
    for command in [
        "resource list srv",
        "resource templates srv",
        "prompt list srv",
        "prompt complete srv review topic x",
        "resource complete srv test:///{id} id x",
    ] {
        assert!(
            exchange(command)
                .continue_with(8, &input, &answers(&input))
                .is_err()
        );
    }
    for id in [-1, 0, 6, 7] {
        assert!(
            exchange("resource read srv test://fixed")
                .continue_with(id, &input, &answers(&input))
                .is_err()
        );
    }
}
