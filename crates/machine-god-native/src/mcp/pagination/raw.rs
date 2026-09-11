use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::{McpCatalogKind, McpPaginationError, ttl};

pub(super) struct Page<'a> {
    pub items: Vec<&'a RawValue>,
    pub ttl: Option<u64>,
}

// Call only after wire admission bounded allocation/depth/nodes and rejected
// duplicate keys. Borrowed raw values preserve exact schema numbers/metadata;
// serde, not a second hand-written JSON scanner, locates their spans.
pub(super) fn parse(bytes: &[u8], kind: McpCatalogKind) -> Result<Page<'_>, McpPaginationError> {
    use McpPaginationError::InvalidResponse as invalid;
    let raw: Envelope<'_> = serde_json::from_slice(bytes).map_err(|_| invalid)?;
    let fields: BTreeMap<String, &RawValue> =
        serde_json::from_str(raw.result.get()).map_err(|_| invalid)?;
    let items = serde_json::from_str(fields.get(kind.field()).ok_or(invalid)?.get())
        .map_err(|_| invalid)?;
    let ttl = fields
        .get("ttlMs")
        .map(|raw| ttl::milliseconds(raw.get()))
        .transpose()?;
    Ok(Page { items, ttl })
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    result: &'a RawValue,
}
