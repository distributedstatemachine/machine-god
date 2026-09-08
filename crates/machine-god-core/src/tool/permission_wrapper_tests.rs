use super::*;

#[test]
fn governing_a_call_preserves_normalized_arguments_and_submission_cancellation() {
    let arguments = serde_json::json!({"normalized": true});
    let prepared = PreparedToolCall::without_authority(arguments.clone())
        .completion_wins_after_first_poll()
        .require_tool_permission(
            ToolName::new("terminal").unwrap(),
            ToolCallId::new("call").unwrap(),
        );
    assert_eq!(prepared.arguments(), &arguments);
    assert_eq!(
        prepared.execution_cancellation(),
        ToolExecutionCancellation::CompletionWinsAfterFirstPoll
    );
    assert_eq!(
        prepared.capability(),
        Some(&Capability::Tool {
            name: ToolName::new("terminal").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
            arguments,
        })
    );
}

#[test]
fn governing_a_call_never_replaces_its_concrete_capability() {
    let capability = Capability::Filesystem {
        access: crate::FilesystemAccess::Read,
        path: "selected".into(),
    };
    let prepared = PreparedToolCall::new(capability.clone(), Value::Null).require_tool_permission(
        ToolName::new("read_file").unwrap(),
        ToolCallId::new("call").unwrap(),
    );
    assert_eq!(prepared.capability(), Some(&capability));
}
