use machine_god_core::{
    BoxFuture, BuildError, CancellationToken, EngineBuilder, Tool, ToolContext, ToolError,
    ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct ChangingTool {
    captured_name: &'static str,
    spec_calls: Arc<AtomicUsize>,
}
impl ChangingTool {
    fn new(captured_name: &'static str) -> Self {
        Self {
            captured_name,
            spec_calls: Arc::default(),
        }
    }
}
impl Tool for ChangingTool {
    fn spec(&self) -> ToolSpec {
        let first = self.spec_calls.fetch_add(1, Ordering::SeqCst) == 0;
        ToolSpec {
            name: ToolName::new(if first { self.captured_name } else { "changed" }).unwrap(),
            description: "captured specification".into(),
            input_schema: json!({"type":"object","properties":{"argument":{"type":"string"}}}),
        }
    }

    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        panic!("registration-name observation must not execute a tool")
    }
}

#[test]
fn empty_names_are_exact_size_without_implying_build_validation() {
    let builder = EngineBuilder::new();
    let mut names = builder.registered_tool_names();
    assert_eq!(names.len(), 0);
    assert_eq!(names.size_hint(), (0, Some(0)));
    assert!(names.next().is_none());
    assert!(names.next().is_none());
    drop(names);
    assert!(matches!(builder.build(), Err(BuildError::MissingProvider)));
}

#[test]
fn owned_and_shared_registrations_have_sorted_captured_names_and_exact_lengths() {
    let builder = EngineBuilder::new()
        .tool(ChangingTool::new("zeta"))
        .shared_tool(Arc::new(ChangingTool::new("alpha")))
        .tool(ChangingTool::new("middle"));
    let mut names = builder.registered_tool_names();
    for (remaining, expected) in [(3, "alpha"), (2, "middle"), (1, "zeta")] {
        assert_eq!(names.len(), remaining);
        assert_eq!(names.size_hint(), (remaining, Some(remaining)));
        assert_eq!(names.next().unwrap().as_str(), expected);
    }
    assert_eq!(names.len(), 0);
    assert!(names.next().is_none());
}

#[test]
fn repeated_iterations_borrow_the_same_keys_without_calling_dynamic_tools() {
    let owned = ChangingTool::new("owned");
    let owned_calls = owned.spec_calls.clone();
    let shared = Arc::new(ChangingTool::new("shared"));
    let builder = EngineBuilder::new().tool(owned).shared_tool(shared.clone());
    let original: Vec<_> = builder.registered_tool_names().collect();
    for _ in 0..64 {
        for (before, observed) in original
            .iter()
            .copied()
            .zip(builder.registered_tool_names())
        {
            assert!(std::ptr::eq(before, observed));
        }
    }
    assert_eq!(
        original
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>(),
        ["owned", "shared"]
    );
    assert_eq!(owned_calls.load(Ordering::SeqCst), 1);
    assert_eq!(shared.spec_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn observing_duplicate_registration_names_does_not_clear_build_failure() {
    let builder = EngineBuilder::new()
        .tool(ChangingTool::new("duplicate"))
        .shared_tool(Arc::new(ChangingTool::new("duplicate")));
    assert_eq!(builder.registered_tool_names().len(), 1);
    assert_eq!(
        builder.registered_tool_names().next().unwrap().as_str(),
        "duplicate"
    );
    assert!(matches!(builder.build(), Err(BuildError::DuplicateTool(name)) if name == "duplicate"));
}
