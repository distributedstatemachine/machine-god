use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolOutput, ToolSpec,
};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::DEFAULT_RECORD_CAPACITY;

/// One ordered response from a scripted tool.
#[derive(Clone, Debug)]
pub enum ToolStep {
    Output(ToolOutput),
    Error(ToolError),
    /// Remain pending until the invocation's cancellation token is cancelled.
    Pending,
}

/// A tool invocation captured before its scripted behavior starts.
#[derive(Clone, Debug)]
pub struct RecordedToolInvocation {
    pub context: ToolContext,
    pub arguments: Value,
    pub cancellation: CancellationToken,
}

/// One ordered response from a scripted tool preparation.
#[derive(Clone, Debug)]
pub enum ToolPrepareStep {
    Prepared {
        capability: Capability,
        arguments: Value,
    },
    Error(ToolError),
}

/// A provider-requested call captured before scripted preparation starts.
#[derive(Clone, Debug)]
pub struct RecordedToolPreparation {
    pub call: ToolCall,
}

#[derive(Debug)]
struct ToolState {
    steps: VecDeque<ToolStep>,
    invocations: Vec<RecordedToolInvocation>,
}

#[derive(Debug)]
struct ToolInner {
    spec: ToolSpec,
    record_capacity: usize,
    state: Mutex<ToolState>,
}

/// A cloneable, strict tool double with bounded invocation recording.
#[derive(Clone, Debug)]
pub struct ScriptedTool {
    inner: Arc<ToolInner>,
}

#[derive(Debug)]
struct PreparedToolState {
    spec: ToolSpec,
    preparation_steps: VecDeque<ToolPrepareStep>,
    preparations: Vec<RecordedToolPreparation>,
    execution_steps: VecDeque<ToolStep>,
    invocations: Vec<RecordedToolInvocation>,
}

#[derive(Debug)]
struct PreparedToolInner {
    record_capacity: usize,
    state: Mutex<PreparedToolState>,
}

/// A cloneable strict tool double with independently scripted preparation and execution.
#[derive(Clone, Debug)]
pub struct ScriptedPreparedTool {
    inner: Arc<PreparedToolInner>,
}

impl ScriptedTool {
    /// Creates a tool with the default invocation-recording bound.
    #[must_use]
    pub fn new(spec: ToolSpec, steps: impl IntoIterator<Item = ToolStep>) -> Self {
        Self::with_record_capacity(spec, steps, DEFAULT_RECORD_CAPACITY)
    }

    /// Creates a tool with an explicit invocation-recording bound.
    #[must_use]
    pub fn with_record_capacity(
        spec: ToolSpec,
        steps: impl IntoIterator<Item = ToolStep>,
        record_capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(ToolInner {
                spec,
                record_capacity,
                state: Mutex::new(ToolState {
                    steps: steps.into_iter().collect(),
                    invocations: Vec::new(),
                }),
            }),
        }
    }

    /// Returns a consistent snapshot in execution-call order.
    #[must_use]
    pub fn invocations(&self) -> Vec<RecordedToolInvocation> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invocations
            .clone()
    }

    /// Returns the number of unconsumed strict steps.
    #[must_use]
    pub fn remaining_steps(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .steps
            .len()
    }

    fn record_and_select(&self, invocation: RecordedToolInvocation) -> Result<ToolStep, ToolError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.invocations.len() >= self.inner.record_capacity {
            return Err(tool_fixture_error(
                "testkit_record_capacity_exhausted",
                "scripted tool invocation recording capacity was exhausted",
            ));
        }
        state.invocations.push(invocation);
        let Some(step) = state.steps.pop_front() else {
            return Err(tool_fixture_error(
                "testkit_script_exhausted",
                "scripted tool was invoked after its script was exhausted",
            ));
        };
        Ok(step)
    }
}

impl ScriptedPreparedTool {
    /// Creates a prepared tool with the default per-phase recording bound.
    #[must_use]
    pub fn new(
        spec: ToolSpec,
        preparation_steps: impl IntoIterator<Item = ToolPrepareStep>,
        execution_steps: impl IntoIterator<Item = ToolStep>,
    ) -> Self {
        Self::with_record_capacity(
            spec,
            preparation_steps,
            execution_steps,
            DEFAULT_RECORD_CAPACITY,
        )
    }

    /// Creates a prepared tool with an explicit per-phase recording bound.
    #[must_use]
    pub fn with_record_capacity(
        spec: ToolSpec,
        preparation_steps: impl IntoIterator<Item = ToolPrepareStep>,
        execution_steps: impl IntoIterator<Item = ToolStep>,
        record_capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(PreparedToolInner {
                record_capacity,
                state: Mutex::new(PreparedToolState {
                    spec,
                    preparation_steps: preparation_steps.into_iter().collect(),
                    preparations: Vec::new(),
                    execution_steps: execution_steps.into_iter().collect(),
                    invocations: Vec::new(),
                }),
            }),
        }
    }

    /// Returns a consistent snapshot in preparation-call order.
    #[must_use]
    pub fn preparations(&self) -> Vec<RecordedToolPreparation> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .preparations
            .clone()
    }

    /// Returns a consistent snapshot in execution-call order.
    #[must_use]
    pub fn invocations(&self) -> Vec<RecordedToolInvocation> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invocations
            .clone()
    }

    /// Returns the number of unconsumed preparation and execution steps.
    #[must_use]
    pub fn remaining_steps(&self) -> (usize, usize) {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.preparation_steps.len(), state.execution_steps.len())
    }

    fn record_and_prepare(
        &self,
        preparation: RecordedToolPreparation,
    ) -> Result<ToolPrepareStep, ToolError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.preparations.len() >= self.inner.record_capacity {
            return Err(tool_fixture_error(
                "testkit_record_capacity_exhausted",
                "scripted tool preparation recording capacity was exhausted",
            ));
        }
        state.preparations.push(preparation);
        let Some(step) = state.preparation_steps.pop_front() else {
            return Err(tool_fixture_error(
                "testkit_script_exhausted",
                "scripted tool was prepared after its preparation script was exhausted",
            ));
        };
        Ok(step)
    }

    fn record_and_select(&self, invocation: RecordedToolInvocation) -> Result<ToolStep, ToolError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.invocations.len() >= self.inner.record_capacity {
            return Err(tool_fixture_error(
                "testkit_record_capacity_exhausted",
                "scripted tool invocation recording capacity was exhausted",
            ));
        }
        state.invocations.push(invocation);
        let Some(step) = state.execution_steps.pop_front() else {
            return Err(tool_fixture_error(
                "testkit_script_exhausted",
                "scripted tool was invoked after its execution script was exhausted",
            ));
        };
        Ok(step)
    }
}

impl Tool for ScriptedTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec.clone()
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let step = self.record_and_select(RecordedToolInvocation {
            context,
            arguments,
            cancellation: cancellation.clone(),
        });
        Box::pin(async move {
            match step? {
                ToolStep::Output(output) => Ok(output),
                ToolStep::Error(error) => Err(error),
                ToolStep::Pending => {
                    cancellation.cancelled().await;
                    Err(ToolError::new(
                        ToolErrorKind::Cancelled,
                        "cancelled",
                        "scripted tool invocation was cancelled",
                        false,
                    ))
                }
            }
        })
    }
}

impl Tool for ScriptedPreparedTool {
    fn spec(&self) -> ToolSpec {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .spec
            .clone()
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        match self.record_and_prepare(RecordedToolPreparation { call })? {
            ToolPrepareStep::Prepared {
                capability,
                arguments,
            } => Ok(PreparedToolCall::new(capability, arguments)),
            ToolPrepareStep::Error(error) => Err(error),
        }
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let step = self.record_and_select(RecordedToolInvocation {
            context,
            arguments,
            cancellation: cancellation.clone(),
        });
        Box::pin(async move {
            match step? {
                ToolStep::Output(output) => Ok(output),
                ToolStep::Error(error) => Err(error),
                ToolStep::Pending => {
                    cancellation.cancelled().await;
                    Err(ToolError::new(
                        ToolErrorKind::Cancelled,
                        "cancelled",
                        "scripted tool invocation was cancelled",
                        false,
                    ))
                }
            }
        })
    }
}

fn tool_fixture_error(code: &'static str, message: &'static str) -> ToolError {
    ToolError::new(ToolErrorKind::Other, code, message, false)
}

#[cfg(test)]
mod tests {
    use super::{ScriptedPreparedTool, ToolPrepareStep, ToolStep};
    use machine_god_core::{
        CancellationToken, Capability, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId,
        ToolContext, ToolName, ToolOutput, ToolSpec, TurnId,
    };
    use serde_json::json;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: ToolName::new("poisoned-prepared-tool").unwrap(),
            description: "poison recovery fixture".to_owned(),
            input_schema: json!({"type": "object"}),
        }
    }

    fn call() -> ToolCall {
        ToolCall {
            id: ToolCallId::new("poisoned-prepare").unwrap(),
            name: spec().name,
            arguments: json!({"raw": true}),
        }
    }

    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("poisoned-tool-session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("poisoned-incarnation").unwrap(),
            turn_id: TurnId::new("turn-1").unwrap(),
            call_id: ToolCallId::new("poisoned-execution").unwrap(),
        }
    }

    #[test]
    fn prepared_tool_recovers_its_single_poisoned_state_lock() {
        let expected_spec = spec();
        let tool = ScriptedPreparedTool::new(
            expected_spec.clone(),
            [ToolPrepareStep::Prepared {
                capability: Capability::Custom {
                    name: "poison-recovered".to_owned(),
                    details: json!({}),
                },
                arguments: json!({"normalized": true}),
            }],
            [ToolStep::Output(ToolOutput::success("recovered"))],
        );
        let inner = tool.inner.clone();
        let poison = std::thread::spawn(move || {
            let _guard = inner.state.lock().unwrap();
            panic!("poison prepared tool state");
        })
        .join();
        assert!(poison.is_err());

        assert_eq!(tool.spec(), expected_spec);
        assert!(tool.prepare(call()).is_ok());
        assert!(
            futures_executor::block_on(tool.execute(
                context(),
                json!({"normalized": true}),
                CancellationToken::new(),
            ))
            .is_ok()
        );
        assert_eq!(tool.preparations().len(), 1);
        assert_eq!(tool.invocations().len(), 1);
        assert_eq!(tool.remaining_steps(), (0, 0));
    }
}
