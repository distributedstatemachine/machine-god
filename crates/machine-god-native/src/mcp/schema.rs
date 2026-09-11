//! Bounded, lossless MCP JSON Schema admission and instance validation.

use std::fmt;
use std::sync::Arc;

mod admit;
mod evaluate;
mod json;
mod number;
mod pattern;
mod resolver;
#[cfg(test)]
mod tests;

/// Finite bounds. Callers may lower defaults, never enlarge hard limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpSchemaLimits {
    pub max_schema_bytes: usize,
    pub max_instance_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_steps: usize,
    pub max_ref_hops: usize,
    pub max_container_entries: usize,
    pub max_number_bytes: usize,
    pub max_number_exponent_abs: i64,
    pub max_number_expanded_digits: usize,
    pub max_pattern_states: usize,
    pub max_pattern_repeat: usize,
    pub max_pattern_steps: usize,
}
impl Default for McpSchemaLimits {
    fn default() -> Self {
        Self {
            max_schema_bytes: 256 * 1024,
            max_instance_bytes: 1024 * 1024,
            max_depth: 64,
            max_nodes: 4096,
            max_steps: 100_000,
            max_ref_hops: 256,
            max_container_entries: 4096,
            max_number_bytes: 4096,
            max_number_exponent_abs: 1_000_000,
            max_number_expanded_digits: 8192,
            max_pattern_states: 2048,
            max_pattern_repeat: 1024,
            max_pattern_steps: 200_000,
        }
    }
}
impl McpSchemaLimits {
    fn validate(self) -> Result<Self> {
        let cap = Self::default();
        for (value, maximum) in [
            (self.max_schema_bytes, cap.max_schema_bytes),
            (self.max_instance_bytes, cap.max_instance_bytes),
            (self.max_depth, cap.max_depth),
            (self.max_nodes, cap.max_nodes),
            (self.max_steps, cap.max_steps),
            (self.max_ref_hops, cap.max_ref_hops),
            (self.max_container_entries, cap.max_container_entries),
            (self.max_number_bytes, cap.max_number_bytes),
            (
                self.max_number_expanded_digits,
                cap.max_number_expanded_digits,
            ),
            (self.max_pattern_states, cap.max_pattern_states),
            (self.max_pattern_repeat, cap.max_pattern_repeat),
            (self.max_pattern_steps, cap.max_pattern_steps),
        ] {
            if value == 0 || value > maximum {
                return Err(McpSchemaError::InvalidLimits);
            }
        }
        if self.max_number_exponent_abs <= 0
            || self.max_number_exponent_abs > cap.max_number_exponent_abs
        {
            return Err(McpSchemaError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSchemaDialect {
    Draft202012,
    Draft7,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSchemaAssessment {
    LocallyEvaluable,
    ServerAuthoritative,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSchemaValidation {
    Valid,
    Invalid(McpSchemaViolation),
    ServerAuthoritative,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSchemaViolation {
    Type,
    Constant,
    Enumeration,
    MultipleOf,
    Maximum,
    ExclusiveMaximum,
    Minimum,
    ExclusiveMinimum,
    MinLength,
    MaxLength,
    Pattern,
    MinItems,
    MaxItems,
    UniqueItems,
    AdditionalItems,
    Contains,
    UnevaluatedItems,
    MinProperties,
    MaxProperties,
    Required,
    DependentRequired,
    Properties,
    PatternProperties,
    AdditionalProperties,
    PropertyNames,
    DependentSchemas,
    Dependencies,
    UnevaluatedProperties,
    AllOf,
    AnyOf,
    OneOf,
    Not,
    Conditional,
    Reference,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSchemaError {
    InvalidLimits,
    InvalidSchema,
    UnsupportedDialect,
    ExternalReference,
    UnresolvedReference,
    SchemaLimitExceeded,
    InstanceLimitExceeded,
    InvalidJson,
}
impl fmt::Display for McpSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP JSON Schema validation failed")
    }
}
impl std::error::Error for McpSchemaError {}
type Result<T> = std::result::Result<T, McpSchemaError>;

/// Immutable admitted schema data. Cloning shares bounds, compiled patterns and
/// local reference indexes; it never grants permission or performs reference I/O.
#[derive(Clone)]
pub struct McpSchema(Arc<Admitted>);
struct Admitted {
    raw: Box<str>,
    tree: json::Tree,
    resolver: resolver::Resolver,
    patterns: std::collections::BTreeMap<String, pattern::Pattern>,
    dialect: McpSchemaDialect,
    assessment: McpSchemaAssessment,
    limits: McpSchemaLimits,
}
impl fmt::Debug for McpSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpSchema")
            .field("assessment", &self.0.assessment)
            .finish_non_exhaustive()
    }
}
impl McpSchema {
    /// Admits complete schema shape, references and bounded local pattern grammar.
    ///
    /// # Errors
    /// Hard schema errors dominate server-authoritative pattern assessment.
    pub fn parse(bytes: &[u8], limits: McpSchemaLimits) -> Result<Self> {
        let limits = limits.validate()?;
        let tree = json::Tree::parse(bytes, limits, true)?;
        let dialect = admit::dialect(&tree, 0)?;
        let resolver = resolver::Resolver::new(&tree, dialect, limits)?;
        let (assessment, patterns) = admit::scan(&tree, &resolver, dialect, limits)?;
        let raw = std::str::from_utf8(bytes)
            .map_err(|_| McpSchemaError::InvalidJson)?
            .into();
        Ok(Self(Arc::new(Admitted {
            raw,
            tree,
            resolver,
            patterns,
            dialect,
            assessment,
            limits,
        })))
    }
    /// Requires the exact MCP input-schema root spelling `"type":"object"`.
    ///
    /// # Errors
    /// Rejects boolean schemas, unions and indirect/reference-only object roots.
    pub fn require_object_root(&self) -> Result<()> {
        if self
            .0
            .tree
            .field(0, "type")
            .and_then(|id| self.0.tree.string(id))
            != Some("object")
        {
            return Err(McpSchemaError::InvalidSchema);
        }
        Ok(())
    }
    #[must_use]
    pub fn raw_json(&self) -> &str {
        &self.0.raw
    }
    #[must_use]
    pub fn assessment(&self) -> McpSchemaAssessment {
        self.0.assessment
    }
    #[must_use]
    pub fn dialect(&self) -> McpSchemaDialect {
        self.0.dialect
    }
    /// Checks bounded JSON first, then the admitted schema. A delegated schema
    /// delegates the whole instance, never selectively applies sibling keywords.
    ///
    /// # Errors
    /// Rejects malformed/over-budget instances and exhausted evaluation budgets.
    pub fn validate_json(&self, bytes: &[u8]) -> Result<McpSchemaValidation> {
        let instance = json::Tree::parse(bytes, self.0.limits, false)?;
        if self.0.assessment == McpSchemaAssessment::ServerAuthoritative {
            return Ok(McpSchemaValidation::ServerAuthoritative);
        }
        evaluate::validate(&self.0, &instance)
    }
}
