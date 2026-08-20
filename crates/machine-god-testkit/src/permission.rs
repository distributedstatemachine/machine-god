use machine_god_core::{
    BoxFuture, PermissionDecision, PermissionError, PermissionHandler, PermissionRequest,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::DEFAULT_RECORD_CAPACITY;

/// One ordered permission-policy response.
#[derive(Clone, Debug)]
pub enum PermissionStep {
    Decision(PermissionDecision),
    Error(PermissionError),
    Pending,
}

#[derive(Debug)]
struct PermissionState {
    steps: VecDeque<PermissionStep>,
    requests: Vec<PermissionRequest>,
}

#[derive(Debug)]
struct PermissionInner {
    record_capacity: usize,
    state: Mutex<PermissionState>,
}

/// A cloneable, strict permission handler that records every handled request.
#[derive(Clone, Debug)]
pub struct ScriptedPermissionHandler {
    inner: Arc<PermissionInner>,
}

impl ScriptedPermissionHandler {
    /// Creates a handler with the default request-recording bound.
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = PermissionStep>) -> Self {
        Self::with_record_capacity(steps, DEFAULT_RECORD_CAPACITY)
    }

    /// Creates a handler with an explicit request-recording bound.
    #[must_use]
    pub fn with_record_capacity(
        steps: impl IntoIterator<Item = PermissionStep>,
        record_capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(PermissionInner {
                record_capacity,
                state: Mutex::new(PermissionState {
                    steps: steps.into_iter().collect(),
                    requests: Vec::new(),
                }),
            }),
        }
    }

    /// Returns a consistent snapshot in authorization-call order.
    #[must_use]
    pub fn requests(&self) -> Vec<PermissionRequest> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .requests
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

    fn record_and_select(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionStep, PermissionError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.requests.len() >= self.inner.record_capacity {
            return Err(permission_fixture_error(
                "testkit_record_capacity_exhausted",
                "scripted permission request recording capacity was exhausted",
            ));
        }
        state.requests.push(request);
        let Some(step) = state.steps.pop_front() else {
            return Err(permission_fixture_error(
                "testkit_script_exhausted",
                "scripted permission handler received a request after its script was exhausted",
            ));
        };
        Ok(step)
    }
}

impl PermissionHandler for ScriptedPermissionHandler {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        let step = self.record_and_select(request);
        Box::pin(async move {
            match step? {
                PermissionStep::Decision(decision) => Ok(decision),
                PermissionStep::Error(error) => Err(error),
                PermissionStep::Pending => core::future::pending().await,
            }
        })
    }
}

fn permission_fixture_error(code: &'static str, message: &'static str) -> PermissionError {
    PermissionError::new(code, message)
}
