use std::cmp::Ordering;

use super::admit::{count, schema_number};
use super::json::{Node, Tree};
use super::number::Number;
use super::pattern::{Pattern, PatternError};
use super::{
    Admitted, McpSchemaDialect as Dialect, McpSchemaError as Error,
    McpSchemaValidation as Validation, McpSchemaViolation as Violation, Result,
};

mod containers;

#[derive(Default)]
struct Marks {
    items: Vec<bool>,
    properties: Vec<bool>,
}
impl Marks {
    fn new(tree: &Tree, instance: usize) -> Self {
        Self {
            items: vec![false; tree.array(instance).map_or(0, <[usize]>::len)],
            properties: vec![
                false;
                tree.object(instance)
                    .map_or(0, std::collections::BTreeMap::len)
            ],
        }
    }
    fn merge(&mut self, other: &Self) {
        for (left, right) in self.items.iter_mut().zip(&other.items) {
            *left |= right;
        }
        for (left, right) in self.properties.iter_mut().zip(&other.properties) {
            *left |= right;
        }
    }
}
struct Outcome {
    violation: Option<Violation>,
    marks: Marks,
}
pub(super) fn validate(admitted: &Admitted, instance: &Tree) -> Result<Validation> {
    let mut context = Context {
        admitted,
        steps: 0,
        pattern_steps: 0,
        ref_hops: 0,
        scope: Vec::new(),
    };
    Ok(context
        .validate(0, instance, 0, 0, 0)?
        .violation
        .map_or(Validation::Valid, Validation::Invalid))
}
struct Context<'a> {
    admitted: &'a Admitted,
    steps: usize,
    pattern_steps: usize,
    ref_hops: usize,
    scope: Vec<usize>,
}
impl Context<'_> {
    fn step(&mut self, depth: usize) -> Result<()> {
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or(Error::InstanceLimitExceeded)?;
        if depth > self.admitted.limits.max_depth || self.steps > self.admitted.limits.max_steps {
            return Err(Error::InstanceLimitExceeded);
        }
        Ok(())
    }
    fn validate(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        inherited: usize,
    ) -> Result<Outcome> {
        self.step(depth)?;
        let mut marks = Marks::new(tree, instance);
        if let Some(value) = self.admitted.tree.boolean(schema) {
            return Ok(Outcome {
                violation: (!value).then_some(Violation::Not),
                marks,
            });
        }
        let resource = self
            .admitted
            .resolver
            .enter(&self.admitted.tree, schema, inherited)?;
        let push = self.scope.last() != Some(&resource);
        if push {
            self.scope.push(resource);
        }
        let result = self.object_schema(schema, tree, instance, depth, &mut marks);
        if push {
            self.scope.pop();
        }
        Ok(Outcome {
            violation: result?,
            marks,
        })
    }
    fn nested(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
    ) -> Result<Outcome> {
        self.validate(
            schema,
            tree,
            instance,
            depth,
            self.scope.last().copied().unwrap_or(0),
        )
    }
    fn collect(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<bool> {
        let result = self.nested(schema, tree, instance, depth)?;
        if result.violation.is_some() {
            return Ok(false);
        }
        marks.merge(&result.marks);
        Ok(true)
    }
    fn reference(
        &mut self,
        reference: &str,
        dynamic: bool,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<bool> {
        self.ref_hops += 1;
        if self.ref_hops > self.admitted.limits.max_ref_hops {
            return Err(Error::InstanceLimitExceeded);
        }
        let resource = self.scope.last().copied().unwrap_or(0);
        let mut target =
            self.admitted
                .resolver
                .resolve(&self.admitted.tree, resource, reference)?;
        if dynamic && let Some(name) = target.dynamic.as_deref() {
            for &resource in &self.scope {
                if let Some(node) = self.admitted.resolver.dynamic_anchor(resource, name) {
                    target.node = node;
                    target.resource = resource;
                    break;
                }
            }
        }
        let result = self.validate(target.node, tree, instance, depth + 1, target.resource);
        self.ref_hops -= 1;
        let result = result?;
        if result.violation.is_some() {
            return Ok(false);
        }
        marks.merge(&result.marks);
        Ok(true)
    }
    fn object_schema(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        let object = admitted.tree.object(schema).ok_or(Error::InvalidSchema)?;
        if let Some(reference) = object.get("$ref") {
            if !self.reference(
                admitted
                    .tree
                    .string(*reference)
                    .ok_or(Error::InvalidSchema)?,
                false,
                tree,
                instance,
                depth,
                marks,
            )? {
                return Ok(Some(Violation::Reference));
            }
            if admitted.dialect == Dialect::Draft7 {
                return Ok(None);
            }
        }
        if admitted.dialect == Dialect::Draft202012
            && let Some(reference) = object.get("$dynamicRef")
            && !self.reference(
                admitted
                    .tree
                    .string(*reference)
                    .ok_or(Error::InvalidSchema)?,
                true,
                tree,
                instance,
                depth,
                marks,
            )?
        {
            return Ok(Some(Violation::Reference));
        }
        if let Some(violation) = self.scalar(schema, tree, instance, depth)? {
            return Ok(Some(violation));
        }
        if tree.array(instance).is_some()
            && let Some(violation) = self.array(schema, tree, instance, depth, marks)?
        {
            return Ok(Some(violation));
        }
        if tree.object(instance).is_some()
            && let Some(violation) = self.object(schema, tree, instance, depth, marks)?
        {
            return Ok(Some(violation));
        }
        if let Some(violation) = self.combinators(schema, tree, instance, depth, marks)? {
            return Ok(Some(violation));
        }
        self.unevaluated(schema, tree, instance, depth, marks)
    }
    fn scalar(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        let object = admitted.tree.object(schema).ok_or(Error::InvalidSchema)?;
        if let Some(types) = object.get("type") {
            let matches = if let Some(name) = admitted.tree.string(*types) {
                self.type_matches(tree, instance, name)?
            } else {
                let mut found = false;
                for value in admitted.tree.array(*types).ok_or(Error::InvalidSchema)? {
                    found |= self.type_matches(
                        tree,
                        instance,
                        admitted.tree.string(*value).ok_or(Error::InvalidSchema)?,
                    )?;
                }
                found
            };
            if !matches {
                return Ok(Some(Violation::Type));
            }
        }
        if let Some(constant) = object.get("const")
            && !self.equal(tree, instance, &admitted.tree, *constant, depth + 1)?
        {
            return Ok(Some(Violation::Constant));
        }
        if let Some(enumeration) = object.get("enum") {
            let mut matched = false;
            for candidate in admitted
                .tree
                .array(*enumeration)
                .ok_or(Error::InvalidSchema)?
            {
                if self.equal(tree, instance, &admitted.tree, *candidate, depth + 1)? {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return Ok(Some(Violation::Enumeration));
            }
        }
        if let Some(text) = tree.number(instance) {
            let value = Number::parse(text, admitted.limits, Error::InstanceLimitExceeded)?;
            if let Some(violation) = self.numeric(schema, &value)? {
                return Ok(Some(violation));
            }
        }
        if let Some(value) = tree.string(instance) {
            let scalar_count = tree
                .string_scalar_count(instance)
                .ok_or(Error::InvalidJson)?;
            for (key, lower, violation) in [
                ("minLength", true, Violation::MinLength),
                ("maxLength", false, Violation::MaxLength),
            ] {
                if let Some(bound) = object.get(key) {
                    let bound = count(&admitted.tree, *bound, admitted.limits)?;
                    if if lower {
                        scalar_count < bound
                    } else {
                        scalar_count > bound
                    } {
                        return Ok(Some(violation));
                    }
                }
            }
            if let Some(pattern) = object.get("pattern")
                && !self.pattern(
                    admitted.tree.string(*pattern).ok_or(Error::InvalidSchema)?,
                    value,
                )?
            {
                return Ok(Some(Violation::Pattern));
            }
        }
        Ok(None)
    }
    fn numeric(&self, schema: usize, value: &Number) -> Result<Option<Violation>> {
        let tree = &self.admitted.tree;
        if let Some(divisor) = tree.field(schema, "multipleOf")
            && !value.multiple_of(
                &schema_number(tree, divisor, self.admitted.limits)?,
                self.admitted.limits,
            )?
        {
            return Ok(Some(Violation::MultipleOf));
        }
        for (key, forbidden, inclusive, violation) in [
            ("maximum", Ordering::Greater, false, Violation::Maximum),
            ("minimum", Ordering::Less, false, Violation::Minimum),
            (
                "exclusiveMaximum",
                Ordering::Greater,
                true,
                Violation::ExclusiveMaximum,
            ),
            (
                "exclusiveMinimum",
                Ordering::Less,
                true,
                Violation::ExclusiveMinimum,
            ),
        ] {
            if let Some(bound) = tree.field(schema, key) {
                let order = value.order(&schema_number(tree, bound, self.admitted.limits)?);
                if order == forbidden || inclusive && order == Ordering::Equal {
                    return Ok(Some(violation));
                }
            }
        }
        Ok(None)
    }
    fn type_matches(&self, tree: &Tree, instance: usize, name: &str) -> Result<bool> {
        Ok(match name {
            "null" => matches!(tree.nodes[instance], Node::Null),
            "boolean" => tree.boolean(instance).is_some(),
            "object" => tree.object(instance).is_some(),
            "array" => tree.array(instance).is_some(),
            "string" => tree.string(instance).is_some(),
            "number" | "integer" => {
                if let Some(text) = tree.number(instance) {
                    let number =
                        Number::parse(text, self.admitted.limits, Error::InstanceLimitExceeded)?;
                    name == "number" || number.integer()
                } else {
                    false
                }
            }
            _ => false,
        })
    }
    fn equal(
        &mut self,
        left: &Tree,
        a: usize,
        right: &Tree,
        b: usize,
        depth: usize,
    ) -> Result<bool> {
        self.step(depth)?;
        let limits = self.admitted.limits;
        let a_number = left
            .number(a)
            .map(|text| Number::parse(text, limits, Error::InstanceLimitExceeded))
            .transpose()?;
        let b_number = right
            .number(b)
            .map(|text| Number::parse(text, limits, Error::InstanceLimitExceeded))
            .transpose()?;
        if a_number.is_some() || b_number.is_some() {
            return Ok(
                matches!((a_number, b_number), (Some(a), Some(b)) if a.order(&b) == Ordering::Equal),
            );
        }
        Ok(match (&left.nodes[a], &right.nodes[b]) {
            (Node::Null, Node::Null) => true,
            (Node::Bool(a), Node::Bool(b)) => a == b,
            (Node::String { value: a, .. }, Node::String { value: b, .. }) => a == b,
            (Node::Array(a), Node::Array(b)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(b) {
                    if !self.equal(left, *a, right, *b, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            (Node::Object(a), Node::Object(b)) if a.len() == b.len() => {
                for (key, a) in a {
                    let Some(b) = b.get(key) else {
                        return Ok(false);
                    };
                    if !self.equal(left, *a, right, *b, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => false,
        })
    }
    fn pattern(&mut self, source: &str, text: &str) -> Result<bool> {
        let limits = self.admitted.limits;
        let local;
        let pattern = if let Some(pattern) = self.admitted.patterns.get(source) {
            pattern
        } else {
            local = Pattern::compile(source, limits).map_err(|_| Error::InvalidSchema)?;
            &local
        };
        pattern
            .matches(text, &mut self.pattern_steps, limits.max_pattern_steps)
            .map_err(|error| match error {
                PatternError::Unsupported => Error::InvalidSchema,
                PatternError::Limit => Error::InstanceLimitExceeded,
            })
    }
    fn combinators(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        for (key, violation) in [
            ("allOf", Violation::AllOf),
            ("anyOf", Violation::AnyOf),
            ("oneOf", Violation::OneOf),
        ] {
            if let Some(children) = admitted.tree.field(schema, key) {
                let mut matches = 0;
                for child in admitted.tree.array(children).ok_or(Error::InvalidSchema)? {
                    let valid = self.collect(*child, tree, instance, depth + 1, marks)?;
                    if key == "allOf" && !valid {
                        return Ok(Some(violation));
                    }
                    matches += usize::from(valid);
                }
                if key == "anyOf" && matches == 0 || key == "oneOf" && matches != 1 {
                    return Ok(Some(violation));
                }
            }
        }
        if let Some(child) = admitted.tree.field(schema, "not")
            && self
                .nested(child, tree, instance, depth + 1)?
                .violation
                .is_none()
        {
            return Ok(Some(Violation::Not));
        }
        if let Some(condition) = admitted.tree.field(schema, "if") {
            let valid = self.collect(condition, tree, instance, depth + 1, marks)?;
            if let Some(branch) = admitted
                .tree
                .field(schema, if valid { "then" } else { "else" })
                && !self.collect(branch, tree, instance, depth + 1, marks)?
            {
                return Ok(Some(Violation::Conditional));
            }
        }
        Ok(None)
    }
    fn unevaluated(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        if admitted.dialect != Dialect::Draft202012 {
            return Ok(None);
        }
        if let (Some(child), Some(items)) = (
            admitted.tree.field(schema, "unevaluatedItems"),
            tree.array(instance),
        ) {
            for (index, item) in items.iter().enumerate() {
                if !marks.items[index] {
                    if self
                        .nested(child, tree, *item, depth + 1)?
                        .violation
                        .is_some()
                    {
                        return Ok(Some(Violation::UnevaluatedItems));
                    }
                    marks.items[index] = true;
                }
            }
        }
        if let (Some(child), Some(properties)) = (
            admitted.tree.field(schema, "unevaluatedProperties"),
            tree.object(instance),
        ) {
            for (index, item) in properties.values().enumerate() {
                if !marks.properties[index] {
                    if self
                        .nested(child, tree, *item, depth + 1)?
                        .violation
                        .is_some()
                    {
                        return Ok(Some(Violation::UnevaluatedProperties));
                    }
                    marks.properties[index] = true;
                }
            }
        }
        Ok(None)
    }
}
