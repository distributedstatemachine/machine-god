//! Pinned bounded matching over already admitted RFC 6570-subset templates.
use super::{Error, Result};
use crate::mcp::catalog::McpResourceTemplateDescriptor;

/// Shared across every candidate template in one concrete-URI lookup.
#[derive(Debug)]
pub struct McpTemplateMatchBudget {
    remaining: usize,
}
impl McpTemplateMatchBudget {
    /// # Errors
    /// Rejects zero or a bound above the pinned one-million-step ceiling.
    pub fn new(steps: usize) -> Result<Self> {
        if steps == 0 || steps > 1024 * 1024 {
            return Err(Error::InvalidLimits);
        }
        Ok(Self { remaining: steps })
    }
    fn consume(&mut self, steps: usize) -> Result<()> {
        let Some(remaining) = self.remaining.checked_sub(steps) else {
            self.remaining = 0;
            return Err(Error::Limit);
        };
        self.remaining = remaining;
        Ok(())
    }
}
struct Expression<'a> {
    open: usize,
    close: usize,
    operator: u8,
    variable: &'a [u8],
}

/// Matches only an admitted template. Percent escapes are matched literally;
/// no URI normalization, filesystem lookup or network authority is introduced.
///
/// # Errors
/// Exhausted work budgets remain errors, never ordinary no-match results.
pub fn matches_resource_template(
    template: &McpResourceTemplateDescriptor,
    uri: &str,
    budget: &mut McpTemplateMatchBudget,
) -> Result<bool> {
    if uri.len() > 64 * 1024 {
        return Ok(false);
    }
    let source = template.uri_template().as_bytes();
    budget.consume(source.len().checked_mul(2).ok_or(Error::Limit)?)?;
    let mut expressions = Vec::new();
    let mut cursor = 0;
    while let Some(open) = source[cursor..].iter().position(|byte| *byte == b'{') {
        let open = open + cursor;
        let close = source[open..]
            .iter()
            .position(|byte| *byte == b'}')
            .ok_or(Error::InvalidRequest)?
            + open;
        let body = &source[open + 1..close];
        let (operator, variable) = if b"+#./;?&".contains(&body[0]) {
            (body[0], &body[1..])
        } else {
            (0, body)
        };
        if expressions.len() == 64 {
            return Err(Error::Limit);
        }
        expressions.push(Expression {
            open,
            close,
            operator,
            variable,
        });
        cursor = close + 1;
    }
    match_from(source, &expressions, uri.as_bytes(), 0, 0, budget)
}
fn equal(left: &[u8], right: &[u8], budget: &mut McpTemplateMatchBudget) -> Result<bool> {
    if left.len() != right.len() {
        return Ok(false);
    }
    budget.consume(left.len())?;
    Ok(left == right)
}
fn match_from(
    source: &[u8],
    expressions: &[Expression<'_>],
    uri: &[u8],
    expression_index: usize,
    uri_index: usize,
    budget: &mut McpTemplateMatchBudget,
) -> Result<bool> {
    let literal_start = if expression_index == 0 {
        0
    } else {
        expressions[expression_index - 1].close + 1
    };
    let Some(expression) = expressions.get(expression_index) else {
        return equal(&source[literal_start..], &uri[uri_index..], budget);
    };
    let literal = &source[literal_start..expression.open];
    if literal.len() > uri.len() - uri_index
        || !equal(literal, &uri[uri_index..uri_index + literal.len()], budget)?
    {
        return Ok(false);
    }
    let value_start = uri_index + literal.len();
    let next_end = expressions
        .get(expression_index + 1)
        .map_or(source.len(), |next| next.open);
    let next_literal = &source[expression.close + 1..next_end];
    if expression_index + 1 == expressions.len() && next_literal.is_empty() {
        return expression_matches(expression, &uri[value_start..], budget);
    }
    if next_literal.is_empty() {
        return Ok(false);
    }
    let mut search = value_start;
    while search < uri.len() {
        let Some(offset) = uri[search..]
            .iter()
            .position(|byte| *byte == next_literal[0])
        else {
            budget.consume(uri.len() - search)?;
            return Ok(false);
        };
        budget.consume(offset + 1)?;
        let end = search + offset;
        if next_literal.len() > uri.len() - end {
            return Ok(false);
        }
        if equal(next_literal, &uri[end..end + next_literal.len()], budget)?
            && expression_matches(expression, &uri[value_start..end], budget)?
            && match_from(source, expressions, uri, expression_index + 1, end, budget)?
        {
            return Ok(true);
        }
        search = end + 1;
    }
    Ok(false)
}
fn expression_matches(
    expression: &Expression<'_>,
    value: &[u8],
    budget: &mut McpTemplateMatchBudget,
) -> Result<bool> {
    match expression.operator {
        0 => expanded(value, false, budget),
        b'+' => expanded(value, true, budget),
        prefix @ (b'#' | b'.' | b'/') => {
            if value.is_empty() {
                return Ok(true);
            }
            if value[0] != prefix {
                return Ok(false);
            }
            expanded(&value[1..], prefix == b'#', budget)
        }
        prefix @ (b';' | b'?' | b'&') => {
            if value.is_empty() {
                return Ok(true);
            }
            if value[0] != prefix
                || value.len() < expression.variable.len() + 1
                || !equal(
                    &value[1..=expression.variable.len()],
                    expression.variable,
                    budget,
                )?
            {
                return Ok(false);
            }
            let rest = &value[expression.variable.len() + 1..];
            if rest.is_empty() {
                return Ok(prefix == b';');
            }
            if rest[0] != b'=' {
                return Ok(false);
            }
            expanded(&rest[1..], false, budget)
        }
        _ => Err(Error::InvalidRequest),
    }
}
fn expanded(value: &[u8], reserved: bool, budget: &mut McpTemplateMatchBudget) -> Result<bool> {
    let mut cursor = 0;
    while let Some(byte) = value.get(cursor) {
        if *byte == b'%' {
            budget.consume(3)?;
            if value
                .get(cursor + 1)
                .is_none_or(|byte| !byte.is_ascii_hexdigit())
                || value
                    .get(cursor + 2)
                    .is_none_or(|byte| !byte.is_ascii_hexdigit())
            {
                return Ok(false);
            }
            cursor += 3;
        } else {
            budget.consume(1)?;
            if !(byte.is_ascii_alphanumeric()
                || b"-._~".contains(byte)
                || reserved && b":/?#[]@!$&'()*+,;=".contains(byte))
            {
                return Ok(false);
            }
            cursor += 1;
        }
    }
    Ok(true)
}
