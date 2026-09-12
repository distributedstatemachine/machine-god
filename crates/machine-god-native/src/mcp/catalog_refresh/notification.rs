use super::{
    McpCatalogKind, McpCatalogRefresh, McpRefreshError, McpRefreshGeneration, Result, Subscription,
};
use crate::mcp::{
    peer::McpPeerCapabilities,
    protocol::{RpcEnvelope, RpcKind},
};
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::Value;
use std::fmt;

/// Capability-derived filter data. It neither starts a listener nor subscribes.
pub struct McpSubscriptionFilters {
    tools: bool,
    resources: bool,
    prompts: bool,
    uris: Box<[Box<str>]>,
}
impl fmt::Debug for McpSubscriptionFilters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpSubscriptionFilters { <redacted> }")
    }
}
impl McpSubscriptionFilters {
    /// Selects advertised list changes and the owner's explicit resource URIs.
    /// # Errors
    /// Rejects unadvertised resource subscriptions, empty/duplicate URIs, more
    /// than 64 URIs or more than 64 KiB of aggregate URI text before copying.
    pub fn new(capabilities: McpPeerCapabilities, uris: &[&str]) -> Result<Self> {
        if uris.len() > 64 || (!uris.is_empty() && !capabilities.resources_subscribe()) {
            return Err(McpRefreshError::Invalid);
        }
        let mut bytes = 0usize;
        for (index, uri) in uris.iter().enumerate() {
            bytes = bytes
                .checked_add(uri.len())
                .ok_or(McpRefreshError::Exhausted)?;
            if uri.is_empty() || bytes > 64 * 1024 || uris[..index].contains(uri) {
                return Err(McpRefreshError::Invalid);
            }
        }
        Ok(Self {
            tools: capabilities.tools_list_changed(),
            resources: capabilities.resources_list_changed(),
            prompts: capabilities.prompts_list_changed(),
            uris: uris.iter().map(|uri| Box::<str>::from(*uri)).collect(),
        })
    }
    /// Whether this selection requests no list changes or resource updates.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.tools && !self.resources && !self.prompts && self.uris.is_empty()
    }
    fn accepts(&self, value: Option<&Value>) -> bool {
        let Some(fields) = value.and_then(Value::as_object) else {
            return false;
        };
        let count = usize::from(self.tools)
            + usize::from(self.resources)
            + usize::from(self.prompts)
            + usize::from(!self.uris.is_empty());
        if fields.len() != count {
            return false;
        }
        for (key, expected) in [
            ("toolsListChanged", self.tools),
            ("resourcesListChanged", self.resources),
            ("promptsListChanged", self.prompts),
        ] {
            match fields.get(key) {
                Some(Value::Bool(true)) if expected => {}
                None if !expected => {}
                _ => return false,
            }
        }
        match fields.get("resourceSubscriptions") {
            None => self.uris.is_empty(),
            Some(Value::Array(uris)) => {
                !self.uris.is_empty()
                    && uris.len() == self.uris.len()
                    && uris
                        .iter()
                        .zip(self.uris.iter())
                        .all(|(actual, expected)| actual.as_str() == Some(expected.as_ref()))
            }
            _ => false,
        }
    }
}
impl Serialize for McpSubscriptionFilters {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (key, enabled) in [
            ("toolsListChanged", self.tools),
            ("resourcesListChanged", self.resources),
            ("promptsListChanged", self.prompts),
        ] {
            if enabled {
                map.serialize_entry(key, &true)?;
            }
        }
        if !self.uris.is_empty() {
            map.serialize_entry("resourceSubscriptions", &self.uris)?;
        }
        map.end()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpRefreshNotification {
    Ignored,
    Acknowledged,
    Invalidated,
    AllResourceReads,
    CloseUnsupported,
    CloseCancelled,
}

const ALL: &[McpCatalogKind] = &[
    McpCatalogKind::Tools,
    McpCatalogKind::Resources,
    McpCatalogKind::ResourceTemplates,
    McpCatalogKind::Prompts,
];
const SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";

impl McpCatalogRefresh {
    /// The owner supplies the exact peer-reserved listen ID and retains this
    /// filter allocation through encoding and acknowledgement admission.
    /// # Errors
    /// Rejects foreign generations, closed state, empty filters, negative or
    /// reused IDs, and invalidation counter exhaustion during handoff.
    pub fn install_subscription(
        &mut self,
        generation: &McpRefreshGeneration,
        request_id: i64,
        filters: McpSubscriptionFilters,
    ) -> Result<()> {
        self.check(generation)?;
        if request_id < 0
            || filters.is_empty()
            || self
                .last_subscription_id
                .is_some_and(|previous| request_id <= previous)
        {
            return Err(McpRefreshError::Invalid);
        }
        if self.last_subscription_id.is_some() {
            self.invalidate(ALL, true)?;
        }
        self.last_subscription_id = Some(request_id);
        self.subscription = Some(Subscription {
            id: request_id,
            acknowledged: false,
            filters,
        });
        Ok(())
    }

    /// Inspects the existing admitted envelope without parsing JSON again.
    /// Outcomes request policy actions only; they grant no transport authority.
    /// # Errors
    /// Rejects foreign generations, closed state and invalidation exhaustion.
    pub fn observe(
        &mut self,
        generation: &McpRefreshGeneration,
        envelope: &RpcEnvelope,
    ) -> Result<McpRefreshNotification> {
        self.check(generation)?;
        if envelope.kind() != RpcKind::Notification {
            return Ok(McpRefreshNotification::Ignored);
        }
        let Some(subscription) = &mut self.subscription else {
            return Ok(McpRefreshNotification::Ignored);
        };
        let Some(params) = envelope.params().and_then(Value::as_object) else {
            return Ok(McpRefreshNotification::Ignored);
        };
        if envelope.method() == Some("notifications/cancelled") {
            if params.get("requestId").and_then(Value::as_i64) == Some(subscription.id) {
                self.subscription = None;
                return Ok(McpRefreshNotification::CloseCancelled);
            }
            return Ok(McpRefreshNotification::Ignored);
        }
        if params
            .get("_meta")
            .and_then(|meta| meta.get(SUBSCRIPTION_ID))
            .and_then(Value::as_i64)
            != Some(subscription.id)
        {
            return Ok(McpRefreshNotification::Ignored);
        }
        if envelope.method() == Some("notifications/subscriptions/acknowledged") {
            if subscription.acknowledged {
                return Ok(McpRefreshNotification::Ignored);
            }
            if !subscription.filters.accepts(params.get("notifications")) {
                self.subscription = None;
                return Ok(McpRefreshNotification::CloseUnsupported);
            }
            subscription.acknowledged = true;
            return Ok(McpRefreshNotification::Acknowledged);
        }
        if !subscription.acknowledged {
            return Ok(McpRefreshNotification::Ignored);
        }
        let filters = &subscription.filters;
        match envelope.method() {
            Some("notifications/tools/list_changed") if filters.tools => {
                self.invalidate(&[McpCatalogKind::Tools], false)?;
            }
            Some("notifications/resources/list_changed") if filters.resources => {
                self.invalidate(
                    &[McpCatalogKind::Resources, McpCatalogKind::ResourceTemplates],
                    true,
                )?;
                return Ok(McpRefreshNotification::AllResourceReads);
            }
            Some("notifications/prompts/list_changed") if filters.prompts => {
                self.invalidate(&[McpCatalogKind::Prompts], false)?;
            }
            Some("notifications/resources/updated") => {
                let uri = params.get("uri").and_then(Value::as_str);
                if !filters
                    .uris
                    .iter()
                    .any(|selected| Some(selected.as_ref()) == uri)
                {
                    return Ok(McpRefreshNotification::Ignored);
                }
                self.invalidate(&[], true)?;
                return Ok(McpRefreshNotification::AllResourceReads);
            }
            _ => return Ok(McpRefreshNotification::Ignored),
        }
        Ok(McpRefreshNotification::Invalidated)
    }
}
