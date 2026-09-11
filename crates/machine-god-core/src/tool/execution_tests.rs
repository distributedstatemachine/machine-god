use super::*;

struct NeverExecute;
impl Tool for NeverExecute {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("unused").unwrap(),
            description: "unused".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        panic!("extracting an output cannot execute a registration")
    }
}

#[test]
fn consuming_complete_output_preserves_allocation_and_discards_turn_effects() {
    for registration in [false, true] {
        let output = ToolOutput::success(String::from("complete output"));
        let pointer = output.content.as_str().unwrap().as_ptr();
        let tool = Arc::new(TurnToolRegistration::new(NeverExecute));
        let weak = Arc::downgrade(&tool);
        let execution = if registration {
            ToolExecution::with_next_round_tool(output, tool)
        } else {
            drop(tool);
            ToolExecution::with_persisted_output(output, ToolOutput::success("archived reference"))
        };
        let complete = execution.into_output();
        assert_eq!(complete.content.as_str().unwrap().as_ptr(), pointer);
        assert!(weak.upgrade().is_none());
        assert_eq!(complete.content, "complete output");
    }
}
