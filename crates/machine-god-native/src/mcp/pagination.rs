//! Bounded, atomic assembly of untrusted MCP catalog pages.
//!
//! This stage validates correlation, pagination, identity uniqueness and cache
//! hints. Items retain exact JSON bytes; schema/descriptor admission and runtime
//! generation publication are separate steps. No candidate is executable.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::value::RawValue;

use super::protocol::{ProtocolVersion, RpcId, RpcKind, WireLimits, parse_envelope};

mod hints;
mod raw;
#[cfg(test)]
mod tests;
mod ttl;

/// A single catalog family. Families cannot be mixed within one assembly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCatalogKind {
    Tools,
    Resources,
    ResourceTemplates,
    Prompts,
}
impl McpCatalogKind {
    /// Pinned per-family descriptor cardinality, independent of byte budgets.
    #[must_use]
    pub const fn max_items(self) -> usize {
        match self {
            Self::Tools => 2048,
            Self::Resources | Self::ResourceTemplates | Self::Prompts => 4096,
        }
    }

    /// Corresponding read-only discovery method, not a user feature invocation.
    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::Tools => "tools/list",
            Self::Resources => "resources/list",
            Self::ResourceTemplates => "resources/templates/list",
            Self::Prompts => "prompts/list",
        }
    }
    const fn field(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Resources => "resources",
            Self::ResourceTemplates => "resourceTemplates",
            Self::Prompts => "prompts",
        }
    }
    const fn identity(self) -> (&'static str, usize) {
        match self {
            Self::Tools | Self::Prompts => ("name", 256),
            Self::Resources => ("uri", 64 * 1024),
            Self::ResourceTemplates => ("uriTemplate", 64 * 1024),
        }
    }
}

/// Server cache hints never grant permission or permit cross-owner publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCatalogCacheScope {
    Private,
    Public,
}

/// Positive caller-selected limits, bounded by the defaults below.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpCatalogLimits {
    pub max_pages: usize,
    pub max_items: usize,
    /// Cumulative received JSON bytes, including discarded envelope metadata.
    pub max_response_bytes: usize,
    /// Cumulative retained item JSON and identity bytes.
    pub max_item_bytes: usize,
    pub max_cursor_bytes: usize,
    /// Cumulative JSON tree charge across pages, not just one page's limit.
    pub max_nodes: usize,
}
impl Default for McpCatalogLimits {
    fn default() -> Self {
        Self {
            max_pages: 64,
            max_items: 4096,
            max_response_bytes: 16 * 1024 * 1024,
            max_item_bytes: 8 * 1024 * 1024,
            max_cursor_bytes: 4096,
            max_nodes: 262_144,
        }
    }
}
impl McpCatalogLimits {
    fn validate(self) -> Result<Self, McpPaginationError> {
        let cap = Self::default();
        for (value, maximum) in [
            (self.max_pages, cap.max_pages),
            (self.max_items, cap.max_items),
            (self.max_response_bytes, cap.max_response_bytes),
            (self.max_item_bytes, cap.max_item_bytes),
            (self.max_cursor_bytes, cap.max_cursor_bytes),
            (self.max_nodes, cap.max_nodes),
        ] {
            if value == 0 || value > maximum {
                return Err(McpPaginationError::Limit);
            }
        }
        Ok(self)
    }
}

/// Fixed, redacted failures. A failed builder is closed and releases its pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpPaginationError {
    InvalidResponse,
    ProtocolFailure,
    CursorMismatch,
    DuplicateCursor,
    DuplicateItem,
    InconsistentCacheScope,
    RegressingClock,
    Limit,
    Incomplete,
    Closed,
}
impl fmt::Display for McpPaginationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP catalog assembly rejected")
    }
}
impl std::error::Error for McpPaginationError {}

struct Assembly {
    items: BTreeMap<Box<str>, Box<RawValue>>,
    cursors: BTreeSet<Box<str>>,
    next: Option<Box<str>>,
    pages: usize,
    response_bytes: usize,
    item_bytes: usize,
    nodes: usize,
    first_received: u64,
    last_received: u64,
    expires: u64,
    scope: Option<McpCatalogCacheScope>,
}
impl Default for Assembly {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
            cursors: BTreeSet::new(),
            next: None,
            pages: 0,
            response_bytes: 0,
            item_bytes: 0,
            nodes: 0,
            first_received: 0,
            last_received: 0,
            expires: u64::MAX,
            scope: None,
        }
    }
}

/// One inert private assembly. There is no partial snapshot accessor.
pub struct McpCatalogBuilder {
    kind: McpCatalogKind,
    version: ProtocolVersion,
    limits: McpCatalogLimits,
    state: Option<Assembly>,
}
impl fmt::Debug for McpCatalogBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpCatalogBuilder")
            .field("kind", &self.kind)
            .field("closed", &self.state.is_none())
            .finish_non_exhaustive()
    }
}
impl McpCatalogBuilder {
    /// Starts a new bounded candidate without I/O, time or ambient authority.
    ///
    /// # Errors
    /// Rejects zero or excessive limits.
    pub fn new(
        kind: McpCatalogKind,
        version: ProtocolVersion,
        limits: McpCatalogLimits,
    ) -> Result<Self, McpPaginationError> {
        let mut limits = limits.validate()?;
        limits.max_items = limits.max_items.min(kind.max_items());
        Ok(Self {
            kind,
            version,
            limits,
            state: Some(Assembly::default()),
        })
    }

    /// Admits exactly the response to the supplied outstanding ID and cursor.
    /// Returns whether another page is required; an empty cursor still is one.
    /// `received_at_ms` is an explicit monotonic clock observation.
    ///
    /// # Errors
    /// Any failure closes the builder. Neither a failed nor an unfinished
    /// candidate can replace a previous runtime catalog.
    pub fn append_response(
        &mut self,
        bytes: &[u8],
        expected_id: &RpcId,
        requested_cursor: Option<&str>,
        received_at_ms: u64,
    ) -> Result<bool, McpPaginationError> {
        let mut state = self.state.take().ok_or(McpPaginationError::Closed)?;
        self.append(
            &mut state,
            bytes,
            expected_id,
            requested_cursor,
            received_at_ms,
        )?;
        let more = state.next.is_some();
        self.state = Some(state);
        Ok(more)
    }

    /// Opaque next cursor. `None` means initial request or completed assembly;
    /// use the return value from `append_response` to distinguish them.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.state.as_ref().and_then(|state| state.next.as_deref())
    }

    /// Consumes a complete candidate, sorting by exact remote identity.
    /// This is raw data, not schema admission or a live runtime publication.
    ///
    /// # Errors
    /// Rejects no pages, an outstanding cursor, or any prior error.
    pub fn finish(self) -> Result<McpRawCatalog, McpPaginationError> {
        let state = self.state.ok_or(McpPaginationError::Closed)?;
        if state.pages == 0 || state.next.is_some() {
            return Err(McpPaginationError::Incomplete);
        }
        Ok(McpRawCatalog {
            kind: self.kind,
            version: self.version,
            items: state.items,
            fetched_at_ms: state.first_received,
            expires_at_ms: state.expires,
            cache_scope: state.scope.ok_or(McpPaginationError::Incomplete)?,
        })
    }

    fn append(
        &self,
        state: &mut Assembly,
        bytes: &[u8],
        expected_id: &RpcId,
        requested_cursor: Option<&str>,
        received: u64,
    ) -> Result<(), McpPaginationError> {
        use McpPaginationError as E;
        if state.pages > 0 && state.next.is_none() {
            return Err(E::Closed);
        }
        if state.next.as_deref() != requested_cursor {
            return Err(E::CursorMismatch);
        }
        if state.pages >= self.limits.max_pages
            || bytes.len() > self.limits.max_response_bytes - state.response_bytes
            || state.nodes == self.limits.max_nodes
        {
            return Err(E::Limit);
        }
        if state.pages > 0 && received < state.last_received {
            return Err(E::RegressingClock);
        }
        let envelope = parse_envelope(
            bytes,
            WireLimits {
                max_frame_bytes: self.limits.max_response_bytes - state.response_bytes,
                // Pinned common feature envelopes count root depth as zero;
                // the wire visitor counts it as one. Tools retain the schema
                // transport ceiling instead of the common feature ceiling.
                max_depth: if self.kind == McpCatalogKind::Tools {
                    64
                } else {
                    33
                },
                max_nodes: self.limits.max_nodes - state.nodes,
            },
        )
        .map_err(|_| E::InvalidResponse)?;
        envelope
            .correlate(expected_id, false)
            .map_err(|_| E::InvalidResponse)?;
        if envelope.kind() == RpcKind::Error {
            return Err(E::ProtocolFailure);
        }
        let result = envelope
            .result()
            .and_then(serde_json::Value::as_object)
            .ok_or(E::InvalidResponse)?;
        if result
            .get("resultType")
            .is_some_and(|value| value.as_str() != Some("complete"))
        {
            return Err(E::InvalidResponse);
        }
        let (scope, cursor) = hints::admit(result, state, self.limits)?;
        let items = result
            .get(self.kind.field())
            .and_then(serde_json::Value::as_array)
            .ok_or(E::InvalidResponse)?;
        if items.len() > self.limits.max_items - state.items.len() {
            return Err(E::Limit);
        }
        let raw = raw::parse(bytes, self.kind)?;
        if raw.items.len() != items.len() {
            return Err(E::InvalidResponse);
        }
        let ttl = raw
            .ttl
            .unwrap_or(if self.version == ProtocolVersion::Modern {
                0
            } else {
                u64::MAX
            });
        let (identity_field, max_identity) = self.kind.identity();
        for (item, raw) in items.iter().zip(raw.items) {
            let identity = item
                .as_object()
                .and_then(|object| object.get(identity_field))
                .and_then(serde_json::Value::as_str)
                .ok_or(E::InvalidResponse)?;
            if identity.is_empty() || identity.len() > max_identity {
                return Err(E::InvalidResponse);
            }
            if state.items.contains_key(identity) {
                return Err(E::DuplicateItem);
            }
            let added = identity.len() + raw.get().len();
            if added > self.limits.max_item_bytes - state.item_bytes {
                return Err(E::Limit);
            }
            state.items.insert(identity.into(), raw.to_owned());
            state.item_bytes += added;
        }
        state.nodes += count_nodes(envelope.value());
        state.response_bytes += bytes.len();
        state.pages += 1;
        if state.pages == 1 {
            state.first_received = received;
        }
        state.last_received = received;
        state.expires = state.expires.min(received.saturating_add(ttl));
        state.scope = Some(scope);
        state.next = cursor.map(Into::into);
        if let Some(cursor) = &state.next {
            state.cursors.insert(cursor.clone());
        }
        Ok(())
    }
}

// The admitted tree is already at most depth 64 and a finite node count.
fn count_nodes(value: &serde_json::Value) -> usize {
    1 + match value {
        serde_json::Value::Array(items) => items.iter().map(count_nodes).sum(),
        serde_json::Value::Object(items) => {
            items.len() + items.values().map(count_nodes).sum::<usize>()
        }
        _ => 0,
    }
}

/// Complete immutable untrusted catalog data. Publication must separately bind
/// server/configuration/authentication, validate every item and recheck the live
/// connection generation. Raw JSON has no executable or permission authority.
pub struct McpRawCatalog {
    kind: McpCatalogKind,
    version: ProtocolVersion,
    items: BTreeMap<Box<str>, Box<RawValue>>,
    fetched_at_ms: u64,
    expires_at_ms: u64,
    cache_scope: McpCatalogCacheScope,
}
impl fmt::Debug for McpRawCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpRawCatalog")
            .field("kind", &self.kind)
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}
impl McpRawCatalog {
    #[must_use]
    pub const fn kind(&self) -> McpCatalogKind {
        self.kind
    }
    #[must_use]
    pub const fn version(&self) -> ProtocolVersion {
        self.version
    }
    #[must_use]
    pub const fn fetched_at_ms(&self) -> u64 {
        self.fetched_at_ms
    }
    #[must_use]
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
    #[must_use]
    pub const fn cache_scope(&self) -> McpCatalogCacheScope {
        self.cache_scope
    }
    /// Exact remote identity and original untrusted JSON, in identity order.
    #[must_use]
    pub fn items(&self) -> impl ExactSizeIterator<Item = (&str, &RawValue)> {
        self.items
            .iter()
            .map(|(identity, item)| (identity.as_ref(), item.as_ref()))
    }
}
