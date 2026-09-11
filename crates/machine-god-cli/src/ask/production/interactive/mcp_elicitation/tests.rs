use super::*;
use machine_god_native::mcp::{mrtr::McpElicitationAction, protocol::ProtocolVersion};

fn request(params: &str) -> McpElicitationRequest {
    McpElicitationRequest::parse(
        &RawValue::from_string(params.into()).unwrap(),
        ProtocolVersion::Modern,
        McpMrtrLimits::default(),
    )
    .unwrap()
}

fn form(schema: &str) -> McpElicitationRequest {
    request(&format!(
        r#"{{"message":"Please fill this form","requestedSchema":{schema}}}"#
    ))
}

fn checked_response(
    request: &McpElicitationRequest,
    response: NativeInteractivePromptResponse,
) -> (McpElicitationAction, Box<RawValue>) {
    let NativeInteractivePromptResponse::Elicitation(response) = response else {
        panic!()
    };
    request.validate_response(response.raw_json()).unwrap()
}

#[test]
fn every_form_kind_keeps_exact_values_until_explicit_final_confirmation() {
    let request = form(
        r#"{"type":"object","properties":{"name":{"type":"string"},"ratio":{"type":"number"},"age":{"type":"integer"},"enabled":{"type":"boolean"},"color":{"type":"string","enum":["red","blue"]},"tags":{"type":"array","items":{"type":"string","enum":["a","b"]}}},"required":["name","ratio","age","enabled","color","tags"]}"#,
    );
    let mut modal = ElicitationModal::default();
    assert!(modal.answer(&request, "yes").is_err());
    for line in [
        "/next",
        "text /cancel-input",
        "9007199254740993.00000001",
        "9007199254740993",
        "false",
        "2",
        "2,1",
    ] {
        let epoch = modal.epoch();
        assert!(modal.answer(&request, line).unwrap().is_none());
        assert_eq!(modal.epoch(), epoch + 1);
    }
    let text = String::from_utf8(
        modal
            .render(&request, "actual-server", "actual-tool")
            .unwrap(),
    )
    .unwrap();
    assert!(text.contains("Review the complete answers"));
    assert!(text.contains("9007199254740993.00000001"));
    let response = modal.answer(&request, "yes").unwrap().unwrap();
    let (action, raw) = checked_response(&request, response);
    assert_eq!(action, McpElicitationAction::Accept);
    assert!(raw.get().contains("9007199254740993.00000001"));
    assert!(raw.get().contains(r#""name":"/cancel-input""#));
    assert!(raw.get().contains(r#""tags":["b","a"]"#));
}

#[test]
fn invalid_fields_do_not_advance_and_defaults_and_optional_omissions_are_explicit() {
    let request = form(
        r#"{"type":"object","properties":{"count":{"type":"integer","minimum":2,"default":3},"optional":{"type":"string"},"lines":{"type":"string"}},"required":["count","lines"]}"#,
    );
    let mut modal = ElicitationModal::default();
    modal.answer(&request, "/next").unwrap();
    let epoch = modal.epoch();
    for line in ["1", "2.5", "/skip", "", "NaN", "{}"] {
        assert!(modal.answer(&request, line).is_err());
        assert_eq!(modal.epoch(), epoch);
    }
    for line in ["/default", "/skip", r#"json "a\nb\u0000c""#] {
        assert!(modal.answer(&request, line).unwrap().is_none());
    }
    let (_, raw) = checked_response(&request, modal.answer(&request, "y").unwrap().unwrap());
    assert!(raw.get().contains(r#""count":3"#));
    assert!(!raw.get().contains("optional"));
    assert!(raw.get().contains(r"a\nb\u0000c"));
}

#[test]
fn cancellation_and_decline_are_distinct_from_url_approval() {
    let request =
        request(r#"{"mode":"url","message":"Authorize","url":"https://example.test/connect"}"#);
    for (line, expected) in [
        ("/decline", McpElicitationAction::Decline),
        ("/cancel-input", McpElicitationAction::Cancel),
        ("y", McpElicitationAction::Accept),
    ] {
        let mut modal = ElicitationModal::default();
        let raw = modal.answer(&request, line).unwrap().unwrap();
        let (action, raw) = checked_response(&request, raw);
        assert_eq!(action, expected);
        assert!(!raw.get().contains("content"));
    }
    assert!(
        ElicitationModal::default()
            .answer(&request, "open it")
            .is_err()
    );
}

#[test]
fn source_pages_are_unicode_safe_bounded_and_require_reading_before_approval() {
    // Independently admitted message and URL span more than one source page.
    let message = "x\u{1b}\u{202e}é".repeat(1150);
    let url = format!("https://example.test/{}", "x".repeat(1000));
    let params = serde_json::json!({"mode":"url","message":message,"url":url});
    let request = request(&params.to_string());
    let mut modal = ElicitationModal::default();
    let first = modal
        .render(&request, "actual\u{1b}server", "actual\u{202e}tool")
        .unwrap();
    assert!(first.len() <= super::super::MAX_PRESENTATION_OUTPUT_BYTES);
    let first = String::from_utf8(first).unwrap();
    assert!(!first.contains('\u{1b}'));
    assert!(!first.contains('\u{202e}'));
    assert!(first.contains("actual\\u001bserver"));
    assert!(first.contains("/next to continue reading"));
    assert!(modal.answer(&request, "y").is_err());
    modal.answer(&request, "/next").unwrap();
    let epoch = modal.epoch();
    modal.answer(&request, "/back").unwrap();
    assert_eq!(modal.epoch(), epoch + 1);
    assert!(modal.answer(&request, "y").is_err());
    modal.answer(&request, "/next").unwrap();
    assert!(modal.answer(&request, "y").unwrap().is_some());
}

#[test]
fn selections_reject_duplicates_out_of_range_and_scalar_coercion() {
    let request = form(
        r#"{"type":"object","properties":{"colors":{"type":"array","items":{"type":"string","enum":["red","blue"]},"minItems":1}}}"#,
    );
    let mut modal = ElicitationModal::default();
    modal.answer(&request, "/next").unwrap();
    for line in ["1,1", "0", "3", "-1", "1.0", "", "1,2,1"] {
        assert!(modal.answer(&request, line).is_err());
    }
    assert!(modal.answer(&request, "1,2").unwrap().is_none());
}

#[test]
fn response_retention_is_bounded_without_partial_field_acceptance() {
    let request = form(
        r#"{"type":"object","properties":{"first":{"type":"string"},"second":{"type":"string"}}}"#,
    );
    let mut modal = ElicitationModal::default();
    modal.answer(&request, "/next").unwrap();
    assert!(
        modal
            .answer(&request, &"x".repeat(64 * 1024))
            .unwrap()
            .is_none()
    );
    let epoch = modal.epoch();
    assert!(modal.answer(&request, &"x".repeat(64 * 1024)).is_err());
    assert_eq!(modal.epoch(), epoch);
    assert_eq!(modal.answers.len(), 1);
    assert!(modal.answer(&request, "small").unwrap().is_none());
}

#[test]
fn nested_forms_are_not_limited_to_the_question_tools_four_fields() {
    use machine_god_native::mcp::mrtr::{McpInputRequestPayload, McpInputRequired};
    let fields = (0..256)
        .map(|n| format!(r#""field{n}":{{"type":"string"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let params = format!(
        r#"{{"message":"Many fields","requestedSchema":{{"type":"object","properties":{{{fields}}}}}}}"#
    );
    let raw = RawValue::from_string(format!(
        r#"{{"inputRequests":{{"form":{{"method":"elicitation/create","params":{params}}}}}}}"#
    ))
    .unwrap();
    let required = McpInputRequired::parse(&raw, McpMrtrLimits::default()).unwrap();
    let McpInputRequestPayload::Elicitation(request) = required.requests()[0].payload() else {
        panic!()
    };
    let mut modal = ElicitationModal::default();
    modal.answer(request, "/next").unwrap();
    for _ in 0..256 {
        assert!(modal.render(request, "server", "tool").is_ok());
        assert!(modal.answer(request, "/skip").unwrap().is_none());
    }
    let response = modal.answer(request, "y").unwrap().unwrap();
    let (_, raw) = checked_response(request, response);
    assert_eq!(raw.get(), r#"{"action":"accept","content":{}}"#);
}
