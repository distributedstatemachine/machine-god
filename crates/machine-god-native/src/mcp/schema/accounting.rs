//! Conservative retained allocation charges, not allocator telemetry.
use super::{McpSchemaError as Error, Result};

pub(super) fn add(total: &mut usize, bytes: usize, limit: usize) -> Result<()> {
    let next = total.checked_add(bytes).ok_or(Error::SchemaLimitExceeded)?;
    if next > limit {
        return Err(Error::SchemaLimitExceeded);
    }
    *total = next;
    Ok(())
}

pub(super) fn array<T>(count: usize) -> Result<usize> {
    count
        .checked_mul(size_of::<T>())
        .ok_or(Error::SchemaLimitExceeded)
}

// Charge a sparsely occupied B-tree node for every entry: room for sixteen
// key/value slots and links plus fixed bookkeeping. Deliberately overcounts
// shared nodes; the accounting contract is independent of occupancy.
pub(super) fn entry<T>() -> usize {
    128 + 16 * (size_of::<T>() + size_of::<usize>())
}
