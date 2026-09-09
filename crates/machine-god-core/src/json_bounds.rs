//! Shared JSON limits and cleanup mechanics, independent of caller error policy.
//!
//! Callers validate depth/node bounds before recursive serialization or cloning.
//! The iterative reader and destructor retain iterators, not every queued sibling.

use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};

struct JsonByteCounter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}

enum JsonChildren<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Values<'a>),
}

impl<'a> Iterator for JsonChildren<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

struct JsonFrame<'a> {
    container_depth: usize,
    children: JsonChildren<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsonLimitViolation {
    Depth,
    Nodes,
}

pub(crate) struct JsonValidationBudget {
    nodes: usize,
    max_nodes: usize,
    max_container_depth: usize,
}

impl JsonValidationBudget {
    pub(crate) fn nodes(&self) -> usize {
        self.nodes
    }

    pub(crate) fn new(limits: crate::EngineLimits) -> Self {
        Self {
            nodes: 0,
            max_nodes: limits.max_json_nodes.get(),
            max_container_depth: limits.max_json_depth.get(),
        }
    }

    pub(crate) fn validate(&mut self, root: &Value) -> Result<(), JsonLimitViolation> {
        let mut frames = Vec::<JsonFrame<'_>>::new();
        let mut current = Some((root, 0usize));

        loop {
            if let Some((value, parent_depth)) = current.take() {
                self.nodes = self.nodes.checked_add(1).ok_or(JsonLimitViolation::Nodes)?;
                if self.nodes > self.max_nodes {
                    return Err(JsonLimitViolation::Nodes);
                }

                let children = match value {
                    Value::Array(values) => Some(JsonChildren::Array(values.iter())),
                    Value::Object(values) => Some(JsonChildren::Object(values.values())),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
                };
                if let Some(children) = children {
                    let container_depth = parent_depth
                        .checked_add(1)
                        .ok_or(JsonLimitViolation::Depth)?;
                    if container_depth > self.max_container_depth {
                        return Err(JsonLimitViolation::Depth);
                    }
                    frames.push(JsonFrame {
                        container_depth,
                        children,
                    });
                }
            }

            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                if let Some(child) = frame.children.next() {
                    current = Some((child, frame.container_depth));
                    break;
                }
                frames.pop();
            }
        }
    }
}

pub(crate) fn validate_json_roots<'a>(
    roots: impl IntoIterator<Item = &'a Value>,
    limits: crate::EngineLimits,
) -> Result<(), JsonLimitViolation> {
    let mut budget = JsonValidationBudget::new(limits);
    for root in roots {
        budget.validate(root)?;
    }
    Ok(())
}

enum OwnedJsonChildren {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoValues),
}

impl Iterator for OwnedJsonChildren {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

/// Reclaims a JSON tree without recursive `Value::drop` calls.
pub(crate) fn drop_json_value_iterative(root: Value) {
    let mut frames = Vec::<OwnedJsonChildren>::new();
    let mut current = Some(root);

    loop {
        if let Some(value) = current.take() {
            match value {
                Value::Array(values) => frames.push(OwnedJsonChildren::Array(values.into_iter())),
                Value::Object(values) => {
                    frames.push(OwnedJsonChildren::Object(values.into_values()));
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }

        loop {
            let Some(frame) = frames.last_mut() else {
                return;
            };
            if let Some(child) = frame.next() {
                current = Some(child);
                break;
            }
            frames.pop();
        }
    }
}

impl Write for JsonByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(buffer.len())
            .ok_or_else(|| io::Error::other("serialized JSON byte count overflowed"))?;
        if next > self.limit {
            self.exceeded = true;
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn serialized_json_size_bounded<T: Serialize + ?Sized>(
    value: &T,
    limit: usize,
) -> Result<Option<usize>, serde_json::Error> {
    let mut counter = JsonByteCounter {
        bytes: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut counter, value);
    if counter.exceeded {
        Ok(None)
    } else {
        result?;
        Ok(Some(counter.bytes))
    }
}

#[cfg(test)]
mod tests;
