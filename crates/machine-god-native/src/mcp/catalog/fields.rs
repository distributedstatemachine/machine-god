//! Borrow raw spans only after the pagination layer's strict wire admission.
use super::{McpCatalogError as Error, Result};
use crate::mcp::schema::{McpSchema, McpSchemaLimits, McpSchemaValidation};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub(crate) type Object<'a> = BTreeMap<String, &'a RawValue>;
pub(crate) fn object(raw: &RawValue) -> Result<Object<'_>> {
    serde_json::from_str(raw.get()).map_err(|_| Error::InvalidDescriptor)
}
pub(crate) fn array(raw: &RawValue) -> Result<Vec<&RawValue>> {
    serde_json::from_str(raw.get()).map_err(|_| Error::InvalidDescriptor)
}
pub(crate) fn boolean(raw: &RawValue) -> Result<bool> {
    serde_json::from_str(raw.get()).map_err(|_| Error::InvalidDescriptor)
}
fn text(raw: &RawValue, maximum: usize, required: bool) -> Result<Box<str>> {
    let text: String = serde_json::from_str(raw.get()).map_err(|_| Error::InvalidDescriptor)?;
    if text.len() > maximum {
        return Err(Error::Limit);
    }
    if required && text.is_empty() {
        return Err(Error::InvalidDescriptor);
    }
    Ok(text.into_boxed_str())
}
pub(crate) fn required(object: &Object<'_>, key: &str, maximum: usize) -> Result<Box<str>> {
    text(
        object.get(key).ok_or(Error::InvalidDescriptor)?,
        maximum,
        true,
    )
}
pub(crate) fn optional(object: &Object<'_>, key: &str, maximum: usize) -> Result<Option<Box<str>>> {
    object
        .get(key)
        .map(|raw| text(raw, maximum, false))
        .transpose()
}
pub(crate) fn metadata(
    raw: &RawValue,
    maximum_depth: usize,
    require_object: bool,
) -> Result<Box<RawValue>> {
    if raw.get().len() > 128 * 1024 {
        return Err(Error::Limit);
    }
    if require_object && !raw.get().starts_with('{') {
        return Err(Error::InvalidDescriptor);
    }
    depth(raw, 0, maximum_depth)?;
    Ok(raw.to_owned())
}
fn depth(raw: &RawValue, current: usize, maximum: usize) -> Result<()> {
    if current > maximum {
        return Err(Error::Limit);
    }
    match raw.get().as_bytes()[0] {
        b'{' => {
            for child in object(raw)?.values() {
                depth(child, current + 1, maximum)?;
            }
        }
        b'[' => {
            for child in array(raw)? {
                depth(child, current + 1, maximum)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn icons(raw: &RawValue, source_limit: usize) -> Result<()> {
    let icons = array(raw)?;
    if icons.len() > 16 {
        return Err(Error::Limit);
    }
    for raw in icons {
        let icon = object(raw)?;
        required(&icon, "src", source_limit)?;
        optional(&icon, "mimeType", 4096)?;
        if let Some(raw) = icon.get("sizes") {
            let sizes = array(raw)?;
            if sizes.len() > 16 {
                return Err(Error::Limit);
            }
            for raw in sizes {
                text(raw, 4096, false)?;
            }
        }
        if let Some(theme) = optional(&icon, "theme", 5)?
            && !matches!(theme.as_ref(), "light" | "dark")
        {
            return Err(Error::InvalidDescriptor);
        }
    }
    Ok(())
}
pub(crate) fn annotations(raw: &RawValue, tool: bool) -> Result<()> {
    let fields = object(raw)?;
    if tool {
        optional(&fields, "title", 4096)?;
        for name in [
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            if let Some(raw) = fields.get(name) {
                boolean(raw)?;
            }
        }
    } else {
        optional(&fields, "lastModified", 4096)?;
        if let Some(raw) = fields.get("audience") {
            let audience = array(raw)?;
            if audience.len() > 2 {
                return Err(Error::InvalidDescriptor);
            }
            for raw in audience {
                if !matches!(text(raw, 9, true)?.as_ref(), "user" | "assistant") {
                    return Err(Error::InvalidDescriptor);
                }
            }
        }
        if let Some(raw) = fields.get("priority") {
            let schema = McpSchema::parse(
                br#"{"type":"number","minimum":0,"maximum":1}"#,
                McpSchemaLimits::default(),
            )
            .map_err(Error::Schema)?;
            if schema
                .validate_json(raw.get().as_bytes())
                .map_err(Error::Schema)?
                != McpSchemaValidation::Valid
            {
                return Err(Error::InvalidDescriptor);
            }
        }
    }
    Ok(())
}

/// Complete, already JSON-validated numeric lexeme; no expansion or float.
pub(crate) fn size(text: &str) -> Result<u64> {
    if text.is_empty() || text.len() > 4096 || !matches!(text.as_bytes()[0], b'-' | b'0'..=b'9') {
        return Err(Error::InvalidDescriptor);
    }
    let (mantissa, exponent) =
        text.split_once(['e', 'E'])
            .map_or(Ok((text, 0_i64)), |(mantissa, exponent)| {
                exponent
                    .parse::<i64>()
                    .map(|exponent| (mantissa, exponent))
                    .map_err(|_| Error::InvalidDescriptor)
            })?;
    if !(-1_000_000..=1_000_000).contains(&exponent) {
        return Err(Error::Limit);
    }
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let fraction = mantissa.split_once('.').map_or(0, |(_, tail)| tail.len());
    let mut digits = mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .skip_while(|byte| *byte == b'0')
        .peekable();
    if digits.peek().is_none() {
        return Ok(0);
    }
    if negative {
        return Err(Error::InvalidDescriptor);
    }
    let trailing = mantissa
        .bytes()
        .rev()
        .filter(|byte| *byte != b'.')
        .take_while(|byte| *byte == b'0')
        .count();
    let effective = exponent - i64::try_from(fraction).map_err(|_| Error::Limit)?
        + i64::try_from(trailing).map_err(|_| Error::Limit)?;
    if !(-1_000_000..=1_000_000).contains(&effective) {
        return Err(Error::Limit);
    }
    let significant = digits.clone().count() - trailing;
    let zeros = usize::try_from(effective).map_err(|_| Error::InvalidDescriptor)?;
    if zeros > 20 || significant > 20 - zeros {
        return Err(Error::InvalidDescriptor);
    }
    let mut value = 0_u64;
    for digit in digits
        .take(significant)
        .chain(std::iter::repeat_n(b'0', zeros))
    {
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or(Error::InvalidDescriptor)?;
    }
    Ok(value)
}
