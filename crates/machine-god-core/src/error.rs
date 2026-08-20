use core::fmt;

macro_rules! component_error {
    ($name:ident, $kind:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            /// Stable machine-readable category.
            pub kind: $kind,
            /// Stable component-defined error code.
            pub code: String,
            /// Human-readable diagnostic without secrets.
            pub message: String,
            /// Whether retrying the same operation may succeed.
            pub retryable: bool,
        }

        impl $name {
            /// Creates a structured component error.
            #[must_use]
            pub fn new(
                kind: $kind,
                code: impl Into<String>,
                message: impl Into<String>,
                retryable: bool,
            ) -> Self {
                Self {
                    kind,
                    code: code.into(),
                    message: message.into(),
                    retryable,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}: {}", self.code, self.message)
            }
        }

        impl std::error::Error for $name {}
    };
}

/// Stable provider failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderErrorKind {
    Authentication,
    RateLimited,
    InvalidRequest,
    Unavailable,
    Transport,
    Protocol,
    Cancelled,
    Other,
}

/// Stable tool failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ToolErrorKind {
    InvalidInput,
    PermissionDenied,
    Unavailable,
    Execution,
    Cancelled,
    Other,
}

/// Stable session-store failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SessionStoreErrorKind {
    NotFound,
    Conflict,
    Corrupt,
    Unavailable,
    Other,
}

component_error!(
    ProviderError,
    ProviderErrorKind,
    "A structured model-provider failure."
);
component_error!(ToolError, ToolErrorKind, "A structured tool failure.");
component_error!(
    SessionStoreError,
    SessionStoreErrorKind,
    "A structured session-store failure."
);

/// A permission handler failed to produce a decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionError {
    pub code: String,
    pub message: String,
}

impl PermissionError {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PermissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PermissionError {}

/// An observer failed to accept an engine event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventSinkError {
    pub code: String,
    pub message: String,
}

impl EventSinkError {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for EventSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EventSinkError {}

/// Engine construction failed before any session could start.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuildError {
    MissingProvider,
    MissingSessionStore,
    MissingPermissionHandler,
    DuplicateTool(String),
    ToolCatalogTooLarge,
    ToolCatalogJsonDepthExceeded,
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProvider => formatter.write_str("a model provider is required"),
            Self::MissingSessionStore => formatter.write_str("a session store is required"),
            Self::MissingPermissionHandler => {
                formatter.write_str("a permission handler is required")
            }
            Self::DuplicateTool(name) => write!(formatter, "duplicate tool registration: {name}"),
            Self::ToolCatalogTooLarge => {
                formatter.write_str("serialized tool catalog exceeds the configured byte limit")
            }
            Self::ToolCatalogJsonDepthExceeded => {
                formatter.write_str("tool input schema exceeds the configured JSON depth limit")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// A public engine operation failed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EngineError {
    SessionBusy,
    Provider(ProviderError),
    Store(SessionStoreError),
    Permission(PermissionError),
    EventSink(EventSinkError),
    Protocol(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionBusy => formatter.write_str("the session already has an active turn"),
            Self::Provider(error) => write!(formatter, "provider failed: {error}"),
            Self::Store(error) => write!(formatter, "session store failed: {error}"),
            Self::Permission(error) => write!(formatter, "permission handler failed: {error}"),
            Self::EventSink(error) => write!(formatter, "event sink failed: {error}"),
            Self::Protocol(message) => write!(formatter, "protocol violation: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<ProviderError> for EngineError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl From<SessionStoreError> for EngineError {
    fn from(error: SessionStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<PermissionError> for EngineError {
    fn from(error: PermissionError) -> Self {
        Self::Permission(error)
    }
}

impl From<EventSinkError> for EngineError {
    fn from(error: EventSinkError) -> Self {
        Self::EventSink(error)
    }
}
