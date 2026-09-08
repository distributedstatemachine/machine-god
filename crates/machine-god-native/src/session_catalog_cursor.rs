//! Portable, inert continuation values for native session catalog observations.

use machine_god_core::SessionId;
use std::fmt;

pub const MAX_NATIVE_SESSION_CATALOG_CURSOR_BYTES: usize = 320;

/// Semantic ordering boundary, not an authenticated token or a store snapshot.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeSessionCatalogCursor {
    updated_at_ms: Option<i64>,
    id: SessionId,
}
impl NativeSessionCatalogCursor {
    #[must_use]
    pub const fn new(updated_at_ms: Option<i64>, id: SessionId) -> Self {
        Self { updated_at_ms, id }
    }

    /// Parses canonical `v1:<time>:<native-id>` or `v1:unknown:<native-id>`.
    /// The entire suffix is the native ID, including any native-valid colons.
    /// # Errors
    /// Rejects oversized input, unsupported versions, noncanonical signed
    /// integers, invalid IDs and missing fields without retaining input.
    pub fn parse(raw: &str) -> Result<Self, NativeSessionCatalogCursorError> {
        if raw.len() > MAX_NATIVE_SESSION_CATALOG_CURSOR_BYTES {
            return Err(NativeSessionCatalogCursorError);
        }
        let rest = raw
            .strip_prefix("v1:")
            .ok_or(NativeSessionCatalogCursorError)?;
        let (time, id) = rest
            .split_once(':')
            .ok_or(NativeSessionCatalogCursorError)?;
        SessionId::validate(id).map_err(|_| NativeSessionCatalogCursorError)?;
        let updated_at_ms = if time == "unknown" {
            None
        } else {
            let parsed = time
                .parse::<i64>()
                .map_err(|_| NativeSessionCatalogCursorError)?;
            if parsed.to_string() != time {
                return Err(NativeSessionCatalogCursorError);
            }
            Some(parsed)
        };
        Ok(Self::new(
            updated_at_ms,
            SessionId::new(id).map_err(|_| NativeSessionCatalogCursorError)?,
        ))
    }
    #[must_use]
    pub const fn updated_at_ms(&self) -> Option<i64> {
        self.updated_at_ms
    }
    #[must_use]
    pub const fn id(&self) -> &SessionId {
        &self.id
    }
}
impl fmt::Display for NativeSessionCatalogCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.updated_at_ms {
            Some(time) => write!(f, "v1:{time}:{}", self.id),
            None => write!(f, "v1:unknown:{}", self.id),
        }
    }
}
impl fmt::Debug for NativeSessionCatalogCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSessionCatalogCursor { .. }")
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSessionCatalogCursorError;
impl fmt::Display for NativeSessionCatalogCursorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native session catalog cursor is invalid")
    }
}
impl std::error::Error for NativeSessionCatalogCursorError {}
