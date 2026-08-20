use machine_god_core::{
    BoxFuture, CancellationToken, Tool, ToolContext, ToolError, ToolErrorKind, ToolOutput, ToolSpec,
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

fn tool_fixture_error(code: &'static str, message: &'static str) -> ToolError {
    ToolError::new(ToolErrorKind::Other, code, message, false)
}
