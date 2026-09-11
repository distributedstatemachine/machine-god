use std::collections::{BTreeMap, BTreeSet};

use super::json::{Node, Tree};
use super::number::Number;
use super::pattern::{Pattern, PatternError};
use super::resolver::Resolver;
use super::{
    McpSchemaAssessment as Assessment, McpSchemaDialect as Dialect, McpSchemaError as Error,
    McpSchemaLimits, Result,
};

pub(super) fn dialect(tree: &Tree, node: usize) -> Result<Dialect> {
    tree.field(node, "$schema")
        .map_or(Ok(Dialect::Draft202012), |value| {
            dialect_uri(tree.string(value).ok_or(Error::InvalidSchema)?)
        })
}
fn dialect_uri(uri: &str) -> Result<Dialect> {
    match uri.strip_suffix('#').unwrap_or(uri) {
        "https://json-schema.org/draft/2020-12/schema" => Ok(Dialect::Draft202012),
        "http://json-schema.org/draft-07/schema" => Ok(Dialect::Draft7),
        _ => Err(Error::UnsupportedDialect),
    }
}
pub(super) fn scan(
    tree: &Tree,
    resolver: &Resolver,
    dialect: Dialect,
    limits: McpSchemaLimits,
) -> Result<(Assessment, BTreeMap<String, Pattern>)> {
    let mut context = Scan {
        tree,
        dialect,
        limits,
        assessment: Assessment::LocallyEvaluable,
        patterns: BTreeMap::new(),
        cached_states: 0,
    };
    let mut pending = vec![(0, 0, 0)];
    let mut visited = BTreeSet::new();
    while let Some((node, resource, depth)) = pending.pop() {
        if !visited.insert(node) {
            continue;
        }
        if depth > limits.max_depth {
            return Err(Error::SchemaLimitExceeded);
        }
        context.schema(node)?;
        let resource = resolver.enter(tree, node, resource)?;
        for key in ["$ref", "$dynamicRef"] {
            if key == "$dynamicRef" && dialect == Dialect::Draft7 {
                continue;
            }
            if let Some(value) = tree.field(node, key) {
                let target = resolver.resolve(
                    tree,
                    resource,
                    tree.string(value).ok_or(Error::InvalidSchema)?,
                )?;
                pending.push((target.node, target.resource, depth + 1));
            }
        }
        for child in children(tree, node, dialect) {
            pending.push((child, resource, depth + 1));
        }
    }
    Ok((context.assessment, context.patterns))
}
struct Scan<'a> {
    tree: &'a Tree,
    dialect: Dialect,
    limits: McpSchemaLimits,
    assessment: Assessment,
    patterns: BTreeMap<String, Pattern>,
    cached_states: usize,
}
impl Scan<'_> {
    fn schema(&mut self, node: usize) -> Result<()> {
        if matches!(self.tree.nodes[node], Node::Bool(_)) {
            return Ok(());
        }
        let object = self.tree.object(node).ok_or(Error::InvalidSchema)?;
        if self.dialect == Dialect::Draft7
            && let Some(reference) = object.get("$ref")
        {
            self.tree.string(*reference).ok_or(Error::InvalidSchema)?;
            return Ok(());
        }
        for (keyword, value) in object {
            if known(self.dialect, keyword) {
                self.keyword(keyword, *value)?;
            }
        }
        Ok(())
    }
    fn keyword(&mut self, key: &str, value: usize) -> Result<()> {
        match key {
            "$schema" => {
                if dialect_uri(self.tree.string(value).ok_or(Error::InvalidSchema)?)?
                    != self.dialect
                {
                    return Err(Error::UnsupportedDialect);
                }
            }
            "$id" | "$anchor" | "$dynamicAnchor" | "$ref" | "$dynamicRef" | "$comment"
            | "title" | "description" | "format" | "contentEncoding" | "contentMediaType" => {
                self.tree.string(value).ok_or(Error::InvalidSchema)?;
            }
            "type" => {
                if let Some(name) = self.tree.string(value) {
                    if !type_name(name) {
                        return Err(Error::InvalidSchema);
                    }
                } else {
                    let values = unique_strings(self.tree, value)?;
                    if values.is_empty() || values.into_iter().any(|name| !type_name(name)) {
                        return Err(Error::InvalidSchema);
                    }
                }
            }
            "pattern" => self.pattern(self.tree.string(value).ok_or(Error::InvalidSchema)?)?,
            "enum" | "examples" => {
                self.tree.array(value).ok_or(Error::InvalidSchema)?;
            }
            "required" => {
                unique_strings(self.tree, value)?;
            }
            "multipleOf" | "maximum" | "minimum" | "exclusiveMaximum" | "exclusiveMinimum" => {
                let number = schema_number(self.tree, value, self.limits)?;
                if key == "multipleOf" && (number.negative || number.zero()) {
                    return Err(Error::InvalidSchema);
                }
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minContains" | "maxContains"
            | "minProperties" | "maxProperties" => {
                count(self.tree, value, self.limits)?;
            }
            "uniqueItems" | "deprecated" | "readOnly" | "writeOnly" => {
                self.tree.boolean(value).ok_or(Error::InvalidSchema)?;
            }
            "$vocabulary" => {
                for child in self
                    .tree
                    .object(value)
                    .ok_or(Error::InvalidSchema)?
                    .values()
                {
                    self.tree.boolean(*child).ok_or(Error::InvalidSchema)?;
                }
            }
            "patternProperties" => {
                for source in self.tree.object(value).ok_or(Error::InvalidSchema)?.keys() {
                    self.pattern(source)?;
                }
            }
            "dependentRequired" => {
                for child in self
                    .tree
                    .object(value)
                    .ok_or(Error::InvalidSchema)?
                    .values()
                {
                    unique_strings(self.tree, *child)?;
                }
            }
            "dependencies" => {
                for child in self
                    .tree
                    .object(value)
                    .ok_or(Error::InvalidSchema)?
                    .values()
                {
                    if self.tree.array(*child).is_some() {
                        unique_strings(self.tree, *child)?;
                    }
                }
            }
            key if map_keyword(self.dialect, key) => {
                self.tree.object(value).ok_or(Error::InvalidSchema)?;
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                let children = self.tree.array(value).ok_or(Error::InvalidSchema)?;
                if key != "prefixItems" && children.is_empty() {
                    return Err(Error::InvalidSchema);
                }
            }
            "items" if self.dialect == Dialect::Draft7 && self.tree.array(value).is_some() => {}
            key if single_keyword(self.dialect, key) => {
                if !matches!(self.tree.nodes[value], Node::Object(_) | Node::Bool(_)) {
                    return Err(Error::InvalidSchema);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pattern(&mut self, source: &str) -> Result<()> {
        if self.patterns.contains_key(source) {
            return Ok(());
        }
        match Pattern::compile(source, self.limits) {
            Ok(pattern) => {
                if self.cached_states + pattern.states() <= self.limits.max_pattern_states {
                    self.cached_states += pattern.states();
                    self.patterns.insert(source.into(), pattern);
                }
            }
            Err(PatternError::Unsupported) => self.assessment = Assessment::ServerAuthoritative,
            Err(PatternError::Limit) => return Err(Error::SchemaLimitExceeded),
        }
        Ok(())
    }
}
pub(super) fn schema_number(tree: &Tree, node: usize, limits: McpSchemaLimits) -> Result<Number> {
    Number::parse(
        tree.number(node).ok_or(Error::InvalidSchema)?,
        limits,
        Error::SchemaLimitExceeded,
    )
}
pub(super) fn count(tree: &Tree, node: usize, limits: McpSchemaLimits) -> Result<usize> {
    schema_number(tree, node, limits)?
        .nonnegative_usize()
        .ok_or(Error::InvalidSchema)
}
fn unique_strings(tree: &Tree, node: usize) -> Result<Vec<&str>> {
    let mut names = BTreeSet::new();
    for child in tree.array(node).ok_or(Error::InvalidSchema)? {
        if !names.insert(tree.string(*child).ok_or(Error::InvalidSchema)?) {
            return Err(Error::InvalidSchema);
        }
    }
    Ok(names.into_iter().collect())
}
pub(super) fn type_name(name: &str) -> bool {
    matches!(
        name,
        "null" | "boolean" | "object" | "array" | "number" | "string" | "integer"
    )
}
pub(super) fn known(dialect: Dialect, key: &str) -> bool {
    matches!(
        key,
        "$schema"
            | "$id"
            | "$ref"
            | "$comment"
            | "type"
            | "enum"
            | "const"
            | "multipleOf"
            | "maximum"
            | "exclusiveMaximum"
            | "minimum"
            | "exclusiveMinimum"
            | "maxLength"
            | "minLength"
            | "pattern"
            | "maxItems"
            | "minItems"
            | "uniqueItems"
            | "maxProperties"
            | "minProperties"
            | "required"
            | "items"
            | "contains"
            | "additionalProperties"
            | "properties"
            | "patternProperties"
            | "propertyNames"
            | "if"
            | "then"
            | "else"
            | "allOf"
            | "anyOf"
            | "oneOf"
            | "not"
            | "title"
            | "description"
            | "default"
            | "readOnly"
            | "writeOnly"
            | "examples"
            | "format"
            | "contentEncoding"
            | "contentMediaType"
    ) || match dialect {
        Dialect::Draft202012 => matches!(
            key,
            "$vocabulary"
                | "$dynamicRef"
                | "$anchor"
                | "$dynamicAnchor"
                | "$defs"
                | "minContains"
                | "maxContains"
                | "dependentRequired"
                | "dependentSchemas"
                | "prefixItems"
                | "unevaluatedItems"
                | "unevaluatedProperties"
                | "deprecated"
                | "contentSchema"
        ),
        Dialect::Draft7 => matches!(key, "definitions" | "additionalItems" | "dependencies"),
    }
}
fn map_keyword(dialect: Dialect, key: &str) -> bool {
    matches!(key, "properties" | "patternProperties")
        || match dialect {
            Dialect::Draft202012 => matches!(key, "$defs" | "dependentSchemas"),
            Dialect::Draft7 => matches!(key, "definitions" | "dependencies"),
        }
}
fn single_keyword(dialect: Dialect, key: &str) -> bool {
    matches!(
        key,
        "items"
            | "contains"
            | "additionalProperties"
            | "propertyNames"
            | "if"
            | "then"
            | "else"
            | "not"
    ) || match dialect {
        Dialect::Draft202012 => matches!(
            key,
            "unevaluatedItems" | "unevaluatedProperties" | "contentSchema"
        ),
        Dialect::Draft7 => key == "additionalItems",
    }
}
pub(super) fn children(tree: &Tree, node: usize, dialect: Dialect) -> Vec<usize> {
    let Some(object) = tree.object(node) else {
        return Vec::new();
    };
    if dialect == Dialect::Draft7 && object.contains_key("$ref") {
        return Vec::new();
    }
    let mut output = Vec::new();
    for (key, value) in object {
        if map_keyword(dialect, key) {
            if let Some(map) = tree.object(*value) {
                for child in map.values() {
                    if !(dialect == Dialect::Draft7
                        && key == "dependencies"
                        && tree.array(*child).is_some())
                    {
                        output.push(*child);
                    }
                }
            }
        } else if matches!(key.as_str(), "allOf" | "anyOf" | "oneOf")
            || dialect == Dialect::Draft202012 && key == "prefixItems"
            || dialect == Dialect::Draft7 && key == "items" && tree.array(*value).is_some()
        {
            if let Some(array) = tree.array(*value) {
                output.extend_from_slice(array);
            }
        } else if single_keyword(dialect, key) {
            output.push(*value);
        }
    }
    output
}
