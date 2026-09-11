use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;

use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

use super::{McpSchemaError, McpSchemaLimits, Result};

#[derive(Debug)]
pub(super) enum Node {
    Null,
    Bool(bool),
    Number(Box<str>),
    String(String),
    Array(Vec<usize>),
    Object(BTreeMap<String, usize>),
}
pub(super) struct Tree {
    pub nodes: Vec<Node>,
}
impl Tree {
    pub fn parse(bytes: &[u8], limits: McpSchemaLimits, schema: bool) -> Result<Self> {
        let error = if schema {
            McpSchemaError::SchemaLimitExceeded
        } else {
            McpSchemaError::InstanceLimitExceeded
        };
        if bytes.len()
            > if schema {
                limits.max_schema_bytes
            } else {
                limits.max_instance_bytes
            }
        {
            return Err(error);
        }
        let raw: &RawValue =
            serde_json::from_slice(bytes).map_err(|_| McpSchemaError::InvalidJson)?;
        let mut tree = Self { nodes: Vec::new() };
        tree.push(raw, 0, limits, error)?;
        Ok(tree)
    }
    fn push(
        &mut self,
        raw: &RawValue,
        depth: usize,
        limits: McpSchemaLimits,
        error: McpSchemaError,
    ) -> Result<usize> {
        if depth > limits.max_depth || self.nodes.len() >= limits.max_nodes {
            return Err(error);
        }
        let id = self.nodes.len();
        self.nodes.push(Node::Null);
        let text = raw.get();
        let node = match text.as_bytes()[0] {
            b'n' => Node::Null,
            b't' => Node::Bool(true),
            b'f' => Node::Bool(false),
            b'"' => {
                Node::String(serde_json::from_str(text).map_err(|_| McpSchemaError::InvalidJson)?)
            }
            b'[' | b'{' => {
                let mut deserializer = serde_json::Deserializer::from_str(text);
                let exceeded = Cell::new(false);
                let value = ContainerSeed {
                    limit: limits.max_container_entries,
                    exceeded: &exceeded,
                }
                .deserialize(&mut deserializer)
                .map_err(|_| {
                    if exceeded.get() {
                        error
                    } else {
                        McpSchemaError::InvalidJson
                    }
                })?;
                match value {
                    Container::Array(values) => Node::Array(
                        values
                            .into_iter()
                            .map(|child| self.push(child, depth + 1, limits, error))
                            .collect::<Result<_>>()?,
                    ),
                    Container::Object(values) => Node::Object(
                        values
                            .into_iter()
                            .map(|(key, child)| {
                                Ok((key, self.push(child, depth + 1, limits, error)?))
                            })
                            .collect::<Result<_>>()?,
                    ),
                }
            }
            _ => {
                if text.len() > limits.max_number_bytes {
                    return Err(error);
                }
                Node::Number(text.into())
            }
        };
        self.nodes[id] = node;
        Ok(id)
    }
    pub fn field(&self, id: usize, key: &str) -> Option<usize> {
        self.object(id)?.get(key).copied()
    }
    pub fn object(&self, id: usize) -> Option<&BTreeMap<String, usize>> {
        if let Node::Object(value) = &self.nodes[id] {
            Some(value)
        } else {
            None
        }
    }
    pub fn array(&self, id: usize) -> Option<&[usize]> {
        if let Node::Array(value) = &self.nodes[id] {
            Some(value)
        } else {
            None
        }
    }
    pub fn string(&self, id: usize) -> Option<&str> {
        if let Node::String(value) = &self.nodes[id] {
            Some(value)
        } else {
            None
        }
    }
    pub fn boolean(&self, id: usize) -> Option<bool> {
        if let Node::Bool(value) = self.nodes[id] {
            Some(value)
        } else {
            None
        }
    }
    pub fn number(&self, id: usize) -> Option<&str> {
        if let Node::Number(value) = &self.nodes[id] {
            Some(value)
        } else {
            None
        }
    }
}

enum Container<'a> {
    Array(Vec<&'a RawValue>),
    Object(BTreeMap<String, &'a RawValue>),
}
struct ContainerSeed<'a> {
    limit: usize,
    exceeded: &'a Cell<bool>,
}
impl<'de> DeserializeSeed<'de> for ContainerSeed<'_> {
    type Value = Container<'de>;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for ContainerSeed<'_> {
    type Value = Container<'de>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON container")
    }
    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut sequence: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<&RawValue>()? {
            if values.len() >= self.limit {
                self.exceeded.set(true);
                return Err(A::Error::custom("container limit"));
            }
            values.push(value);
        }
        Ok(Container::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.len() >= self.limit {
                self.exceeded.set(true);
                return Err(A::Error::custom("container limit"));
            }
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate key"));
            }
            values.insert(key, map.next_value::<&RawValue>()?);
        }
        Ok(Container::Object(values))
    }
}
