use super::NativeInteractivePromptToken;

/// Structural presentation category; it carries no prompt or effect authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractivePromptKind {
    Permission,
    Question,
    Elicitation,
    UrlRecovery,
}

/// Payload-free row. The token exposes its exact owner without copying a form,
/// permission rationale, question, or authorization URL into a catalog.
#[derive(Debug)]
pub struct NativeInteractivePromptSummary {
    pub(super) token: NativeInteractivePromptToken,
    pub(super) kind: NativeInteractivePromptKind,
}
impl NativeInteractivePromptSummary {
    #[must_use]
    pub const fn token(&self) -> &NativeInteractivePromptToken {
        &self.token
    }
    #[must_use]
    pub const fn kind(&self) -> NativeInteractivePromptKind {
        self.kind
    }
}

/// Bounded unanswered-request projection in admission order. A continuation
/// cursor does not assert that previously shown rows are still present.
#[derive(Debug)]
pub struct NativeInteractivePromptPage {
    pub(super) entries: Vec<NativeInteractivePromptSummary>,
    pub(super) next: Option<NativeInteractivePromptToken>,
}
impl NativeInteractivePromptPage {
    #[must_use]
    pub fn entries(&self) -> &[NativeInteractivePromptSummary] {
        &self.entries
    }
    #[must_use]
    pub const fn next(&self) -> Option<&NativeInteractivePromptToken> {
        self.next.as_ref()
    }
}
