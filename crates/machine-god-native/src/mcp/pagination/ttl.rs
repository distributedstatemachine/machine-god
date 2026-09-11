//! Exact decimal TTL conversion, without floating-point rounding or expansion.
use super::McpPaginationError;

pub(super) fn milliseconds(text: &str) -> Result<u64, McpPaginationError> {
    use McpPaginationError::InvalidResponse as invalid;
    if text.len() > 4096 || text.is_empty() {
        return Err(invalid);
    }
    // This lexeme has already been validated by serde as part of the envelope.
    // Reject non-numbers here instead of accepting a quoted numeric value.
    if !matches!(text.as_bytes()[0], b'-' | b'0'..=b'9') {
        return Err(invalid);
    }
    let (mantissa, exponent) =
        text.split_once(['e', 'E'])
            .map_or(Ok((text, 0i64)), |(mantissa, exponent)| {
                exponent
                    .parse::<i64>()
                    .map(|exponent| (mantissa, exponent))
                    .map_err(|_| invalid)
            })?;
    if !(-1_000_000..=1_000_000).contains(&exponent) {
        return Err(invalid);
    }
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let fraction = i64::try_from(fraction).map_err(|_| invalid)?;
    let mut digits = mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .skip_while(|byte| *byte == b'0')
        .peekable();
    if digits.peek().is_none() {
        return Ok(0);
    }
    let trailing = mantissa
        .bytes()
        .rev()
        .filter(|byte| *byte != b'.')
        .take_while(|byte| *byte == b'0')
        .count();
    let effective = exponent - fraction + i64::try_from(trailing).map_err(|_| invalid)?;
    if !(-1_000_000..=1_000_000).contains(&effective) {
        return Err(invalid);
    }
    // Pinned negative values (including fractional values) mean immediately stale.
    if negative {
        return Ok(0);
    }
    let significant = digits.clone().count() - trailing;
    let zeros = usize::try_from(effective).map_err(|_| invalid)?;
    if zeros > 20 || significant > 20 - zeros {
        return Err(invalid);
    }
    let mut number = 0u64;
    for digit in digits
        .take(significant)
        .chain(std::iter::repeat_n(b'0', zeros))
    {
        number = number
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or(invalid)?;
    }
    Ok(number)
}
