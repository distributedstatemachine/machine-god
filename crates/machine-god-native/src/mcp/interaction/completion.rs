//! Owned client-URL completion, separate from an answer or browser handoff.

use super::{McpElicitationPromptError, McpElicitationPromptRequest};
use std::fmt;

/// Terminal observation of the operation that requested client URL input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpClientUrlOutcome {
    /// The final admitted operation response has been published. This does not
    /// assert that a remote tool's application-level result was successful.
    Completed,
    /// The operation returned unresolved input or a protocol failure.
    Unresolved,
    /// Cancellation, invalidation, failure or dropped ownership prevented a
    /// final published result. Never report this as successful completion.
    Abandoned,
}

/// Trusted host endpoint for one exact selected request. Registration must be
/// bounded, nonblocking and must not open a browser, submit a request, or grant
/// permission. The endpoint owns correlation with its actual outbound request,
/// including session/incarnation, operation and round; server URL identifiers
/// alone are not correlation authority.
pub trait McpClientUrlEndpoint: Send + Sync {
    /// # Errors
    /// Rejects unavailable/stale native custody or exhausted endpoint bounds.
    fn register(
        &self,
        request: &McpElicitationPromptRequest,
    ) -> Result<McpClientUrlCompletion, McpElicitationPromptError>;
}

/// One nonblocking terminal callback. Implementations enqueue into an explicitly
/// bounded host output lane, never perform I/O or block inside this callback.
/// A stale or never-submitted wire request must be invalidated, not relabelled
/// as belonging to a new session, operation or connection.
pub trait McpClientUrlCompletionObserver: Send {
    fn finish(self: Box<Self>, outcome: McpClientUrlOutcome);
}

/// Exactly-once terminal custody. Dropping without a final result abandons only
/// this registered request, including while a presenter future is suspended.
pub struct McpClientUrlCompletion {
    observer: Option<Box<dyn McpClientUrlCompletionObserver>>,
}
impl McpClientUrlCompletion {
    #[must_use]
    pub fn new(observer: Box<dyn McpClientUrlCompletionObserver>) -> Self {
        Self {
            observer: Some(observer),
        }
    }

    pub(crate) fn finish(mut self, outcome: McpClientUrlOutcome) {
        if let Some(observer) = self.observer.take() {
            observer.finish(outcome);
        }
    }
}
impl Drop for McpClientUrlCompletion {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.take() {
            observer.finish(McpClientUrlOutcome::Abandoned);
        }
    }
}
impl fmt::Debug for McpClientUrlCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpClientUrlCompletion { <redacted> }")
    }
}

/// Eight continuation rounds of at most 32 requests. Host endpoints must also
/// enforce their own independently bounded correlation/output allocations.
#[derive(Default)]
pub(crate) struct McpClientUrlCompletions(Vec<McpClientUrlCompletion>);
impl McpClientUrlCompletions {
    pub(crate) fn register(
        &self,
        endpoint: &dyn McpClientUrlEndpoint,
        request: &McpElicitationPromptRequest,
    ) -> Result<McpClientUrlCompletion, McpElicitationPromptError> {
        if self.0.len() >= 256 {
            return Err(McpElicitationPromptError::Limit);
        }
        endpoint.register(request)
    }

    pub(crate) fn retain(
        &mut self,
        completion: McpClientUrlCompletion,
    ) -> Result<(), McpElicitationPromptError> {
        if self.0.len() >= 256 {
            return Err(McpElicitationPromptError::Limit);
        }
        self.0.push(completion);
        Ok(())
    }

    pub(crate) fn finish(self, outcome: McpClientUrlOutcome) {
        for completion in self.0 {
            completion.finish(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Observer(Arc<Mutex<Vec<McpClientUrlOutcome>>>);
    impl McpClientUrlCompletionObserver for Observer {
        fn finish(self: Box<Self>, outcome: McpClientUrlOutcome) {
            self.0.lock().unwrap().push(outcome);
        }
    }

    #[test]
    fn completion_is_once_and_constructor_is_inert() {
        for outcome in [
            McpClientUrlOutcome::Completed,
            McpClientUrlOutcome::Unresolved,
        ] {
            let events = Arc::new(Mutex::new(Vec::new()));
            let completion = McpClientUrlCompletion::new(Box::new(Observer(events.clone())));
            assert!(events.lock().unwrap().is_empty());
            completion.finish(outcome);
            assert_eq!(*events.lock().unwrap(), [outcome]);
        }
    }

    #[test]
    fn dropping_an_operation_abandons_each_exact_registration() {
        let first = Arc::new(Mutex::new(Vec::new()));
        let other = Arc::new(Mutex::new(Vec::new()));
        let owned = McpClientUrlCompletions(vec![McpClientUrlCompletion::new(Box::new(Observer(
            first.clone(),
        )))]);
        let unrelated = McpClientUrlCompletion::new(Box::new(Observer(other.clone())));
        drop(owned);
        assert_eq!(*first.lock().unwrap(), [McpClientUrlOutcome::Abandoned]);
        assert!(other.lock().unwrap().is_empty());
        unrelated.finish(McpClientUrlOutcome::Completed);
        assert_eq!(*other.lock().unwrap(), [McpClientUrlOutcome::Completed]);
    }
}
