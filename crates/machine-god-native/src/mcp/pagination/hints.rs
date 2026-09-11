use serde_json::{Map, Value};

use super::{Assembly, McpCatalogCacheScope, McpCatalogLimits, McpPaginationError};

pub(super) fn admit<'a>(
    result: &'a Map<String, Value>,
    state: &Assembly,
    limits: McpCatalogLimits,
) -> Result<(McpCatalogCacheScope, Option<&'a str>), McpPaginationError> {
    use McpPaginationError as E;
    let scope = match result.get("cacheScope").map(Value::as_str) {
        None | Some(Some("private")) => McpCatalogCacheScope::Private,
        Some(Some("public")) => McpCatalogCacheScope::Public,
        _ => return Err(E::InvalidResponse),
    };
    if state.scope.is_some_and(|old| old != scope) {
        return Err(E::InconsistentCacheScope);
    }
    let cursor = match result.get("nextCursor") {
        None => None,
        Some(value) => {
            let cursor = value.as_str().ok_or(E::InvalidResponse)?;
            if cursor.len() > limits.max_cursor_bytes {
                return Err(E::Limit);
            }
            if state.cursors.contains(cursor) {
                return Err(E::DuplicateCursor);
            }
            if state.pages + 1 == limits.max_pages {
                return Err(E::Limit);
            }
            Some(cursor)
        }
    };
    Ok((scope, cursor))
}
