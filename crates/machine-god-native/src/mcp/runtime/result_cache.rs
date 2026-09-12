//! Complete feature data only: no request reservation or continuation authority.
use super::{
    NativeMcpRuntimeError as Error, Result,
    catalog_state::{Charge, FeatureCacheBudget},
};
use crate::{
    McpFeatureAction as Action, McpFeatureRequest,
    mcp::{
        catalog::McpDescriptorCatalog,
        feature::{McpFeatureIdentity, McpFeatureOutcome, McpFeatureResponse},
    },
};
use std::{collections::VecDeque, sync::Arc, time::Instant};

const MAX_ENTRIES: usize = 64;
const MAX_KEY_BYTES: usize = 128 * 1024;

#[cfg(test)]
mod tests;

pub(super) struct FeatureResultCache {
    entries: VecDeque<Entry>,
    budget: Arc<FeatureCacheBudget>,
    serial: u64,
    last_seen: u64,
    closed: bool,
}
struct Entry {
    key: Box<str>,
    action: Action,
    invalidation: u64,
    expires: u64,
    response: McpFeatureResponse,
    _charge: Charge,
}
pub(super) struct Ticket {
    key: Box<str>,
    action: Action,
    invalidation: u64,
    serial: u64,
}
pub(super) enum Lookup {
    Hit(McpFeatureResponse),
    Fetch(Ticket),
}

/// Identity/schema admission is the same as exchange preparation, without
/// fabricating an RPC ID. Exact string arguments are already in a sorted map.
pub(super) fn key(
    request: &McpFeatureRequest,
    catalogs: &[McpDescriptorCatalog],
) -> crate::mcp::runtime::features::Result<Option<Box<str>>> {
    if !matches!(request.action(), Action::ResourceRead | Action::PromptGet) {
        return Ok(None);
    }
    let identity = McpFeatureIdentity::admit(request, catalogs)?;
    let descriptor = match &identity {
        McpFeatureIdentity::Resource(item) => item.raw_json(),
        McpFeatureIdentity::ResourceTemplate(item) => item.raw_json(),
        McpFeatureIdentity::Prompt(item) => item.raw_json(),
        McpFeatureIdentity::Catalog(_) => return Err(Error::Invalid.into()),
    };
    // A descriptor may be legal yet too large to retain as an optional key.
    // Bound encoding before allocation (JSON string escaping is at most 6x).
    let bytes = request.arguments().iter().try_fold(
        descriptor
            .get()
            .len()
            .saturating_add(request.identity().map_or(0, str::len)),
        |sum, (name, value)| {
            sum.checked_add(name.len())?
                .checked_add(value.len())?
                .checked_add(8)
        },
    );
    if bytes
        .and_then(|bytes| bytes.checked_mul(6))
        .and_then(|bytes| bytes.checked_add(128))
        .is_none_or(|bytes| bytes > MAX_KEY_BYTES)
    {
        return Ok(None);
    }
    serde_json::to_string(&(
        request.action().as_str(),
        request.identity(),
        request.arguments(),
        descriptor.get(),
    ))
    .map(|key| Some(key.into_boxed_str()))
    .map_err(|_| Error::Invalid.into())
}

impl FeatureResultCache {
    pub(super) fn new(budget: Arc<FeatureCacheBudget>) -> Self {
        Self {
            entries: VecDeque::new(),
            budget,
            serial: 0,
            last_seen: 0,
            closed: false,
        }
    }
    fn observe(&mut self, invalidation: (u64, u64), now: u64) -> Result<()> {
        if self.closed || now < self.last_seen {
            self.entries.clear();
            self.closed = true;
            return Err(Error::Invalid);
        }
        self.last_seen = now;
        self.entries.retain(|entry| {
            entry.invalidation == tag(entry.action, invalidation) && now < entry.expires
        });
        Ok(())
    }
    pub(super) fn begin(
        &mut self,
        key: Box<str>,
        action: Action,
        invalidation: (u64, u64),
        now: u64,
    ) -> Result<Lookup> {
        self.observe(invalidation, now)?;
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.key == key && now < entry.expires)
        {
            return Ok(Lookup::Hit(entry.response.clone()));
        }
        self.serial = self.serial.checked_add(1).ok_or_else(|| {
            self.closed = true;
            Error::Limit
        })?;
        Ok(Lookup::Fetch(Ticket {
            key,
            action,
            invalidation: tag(action, invalidation),
            serial: self.serial,
        }))
    }
    pub(super) fn finish(
        &mut self,
        ticket: Ticket,
        response: &McpFeatureResponse,
        invalidation: (u64, u64),
        epoch: Instant,
        now: u64,
    ) -> Result<()> {
        self.observe(invalidation, now)?;
        // Human input releases the lane: a later fetch or notification must not
        // let this old operation overwrite newer state on returning from consent.
        if ticket.serial != self.serial || ticket.invalidation != tag(ticket.action, invalidation) {
            return Ok(());
        }
        let ((Action::ResourceRead, McpFeatureOutcome::Resource { cache: hints, .. })
        | (Action::PromptGet, McpFeatureOutcome::Prompt { cache: hints, .. })) =
            (ticket.action, response.outcome())
        else {
            return Ok(());
        };
        let Some(received) = response
            .received_at()
            .and_then(|at| at.checked_duration_since(epoch))
            .and_then(|at| u64::try_from(at.as_millis()).ok())
        else {
            return Ok(());
        };
        let expires = received.saturating_add(hints.ttl_ms.unwrap_or(0));
        self.entries.retain(|entry| entry.key != ticket.key);
        if received > now || now >= expires {
            return Ok(());
        }
        let Some(bytes) = response
            .retained_byte_charge()
            .checked_add(ticket.key.len())
            .and_then(|bytes| bytes.checked_add(1024))
        else {
            return Ok(());
        };
        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
        }
        let Some(charge) = self.budget.reserve(bytes) else {
            return Ok(());
        };
        self.entries.push_back(Entry {
            key: ticket.key,
            action: ticket.action,
            invalidation: ticket.invalidation,
            expires,
            response: response.clone(),
            _charge: charge,
        });
        Ok(())
    }
}
fn tag(action: Action, invalidation: (u64, u64)) -> u64 {
    if action == Action::ResourceRead {
        invalidation.0
    } else {
        invalidation.1
    }
}
