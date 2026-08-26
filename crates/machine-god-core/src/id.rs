use core::fmt;
use core::str::FromStr;
use serde::{Deserialize, Serialize};

const MAX_ID_BYTES: usize = 128;

/// Describes why a public identifier was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidId {
    kind: &'static str,
    reason: &'static str,
}

impl InvalidId {
    /// The identifier type that was being parsed.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// A stable, human-readable rejection reason.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for InvalidId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.kind, self.reason)
    }
}

impl std::error::Error for InvalidId {}

fn validate_identifier(value: &str, kind: &'static str) -> Result<(), InvalidId> {
    if value.is_empty() {
        return Err(InvalidId {
            kind,
            reason: "must not be empty",
        });
    }
    if value.len() > MAX_ID_BYTES {
        return Err(InvalidId {
            kind,
            reason: "must be at most 128 bytes",
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(InvalidId {
            kind,
            reason: "must contain only ASCII letters, digits, '-', '_', '.', or ':'",
        });
    }
    Ok(())
}

macro_rules! identifier {
    ($name:ident, $kind:literal) => {
        #[doc = concat!("A validated ", $kind, ".")]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Validates borrowed ", $kind, " text without taking ownership.")]
            ///
            /// # Errors
            ///
            /// Returns [`InvalidId`] when the value is empty, longer than 128
            /// bytes, or contains a character outside the portable ID alphabet.
            pub fn validate(value: &str) -> Result<(), InvalidId> {
                validate_identifier(value, $kind)
            }

            #[doc = concat!("Parses a ", $kind, ".")]
            ///
            /// # Errors
            ///
            /// Returns [`InvalidId`] when the value is empty, longer than 128
            /// bytes, or contains a character outside the portable ID alphabet.
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                Self::validate(&value)?;
                Ok(Self(value))
            }

            /// Returns the validated identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidId;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidId;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier!(SessionId, "session ID");
identifier!(SessionIncarnationId, "session incarnation ID");
identifier!(TurnId, "turn ID");
identifier!(ToolCallId, "tool-call ID");
identifier!(PermissionRequestId, "permission-request ID");
identifier!(ToolName, "tool name");

#[cfg(test)]
mod tests {
    use super::{SessionId, SessionIncarnationId, ToolCallId, ToolName};

    #[test]
    fn identifiers_reject_log_and_path_injection_characters() {
        for invalid in ["", "two words", "line\nbreak", "path/to", "café"] {
            assert!(SessionId::new(invalid).is_err(), "accepted {invalid:?}");
            assert!(
                SessionIncarnationId::new(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn deserialization_cannot_bypass_validation() {
        assert!(serde_json::from_str::<ToolName>(r#""not/a/tool""#).is_err());
        let tool = serde_json::from_str::<ToolName>(r#""read_file""#).unwrap();
        assert_eq!(tool.as_str(), "read_file");
    }

    #[test]
    fn borrowed_validation_matches_owned_construction() {
        for candidate in ["call-1", "read_file", "", "not/a/tool"] {
            assert_eq!(
                ToolCallId::validate(candidate).is_ok(),
                ToolCallId::new(candidate).is_ok()
            );
            assert_eq!(
                ToolName::validate(candidate).is_ok(),
                ToolName::new(candidate).is_ok()
            );
        }
    }
}
