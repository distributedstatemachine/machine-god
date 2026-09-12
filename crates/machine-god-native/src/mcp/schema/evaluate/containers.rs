use super::{Context, Dialect, Error, Marks, Node, Result, Tree, Violation, count};

impl Context<'_> {
    pub(super) fn array(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        let items = tree.array(instance).ok_or(Error::InvalidJson)?;
        if let Some(violation) = self.container_length(
            schema,
            items.len(),
            "minItems",
            "maxItems",
            Violation::MinItems,
            Violation::MaxItems,
        )? {
            return Ok(Some(violation));
        }
        if admitted
            .tree
            .field(schema, "uniqueItems")
            .and_then(|id| admitted.tree.boolean(id))
            == Some(true)
        {
            for (index, a) in items.iter().enumerate() {
                for b in &items[index + 1..] {
                    if self.equal(tree, *a, tree, *b, depth + 1)? {
                        return Ok(Some(Violation::UniqueItems));
                    }
                }
            }
        }
        let tuple = if admitted.dialect == Dialect::Draft202012 {
            admitted.tree.field(schema, "prefixItems")
        } else {
            admitted
                .tree
                .field(schema, "items")
                .filter(|id| admitted.tree.array(*id).is_some())
        };
        let prefix = tuple.and_then(|id| admitted.tree.array(id)).unwrap_or(&[]);
        let count = prefix.len().min(items.len());
        for (index, (child, item)) in prefix.iter().zip(items).enumerate() {
            if self
                .nested(*child, tree, *item, depth + 1)?
                .violation
                .is_some()
            {
                return Ok(Some(Violation::Type));
            }
            marks.items[index] = true;
        }
        let (remaining, violation) = if admitted.dialect == Dialect::Draft7 && tuple.is_some() {
            (
                admitted.tree.field(schema, "additionalItems"),
                Violation::AdditionalItems,
            )
        } else {
            (admitted.tree.field(schema, "items"), Violation::Type)
        };
        if let Some(child) = remaining {
            for (index, item) in items.iter().enumerate().skip(count) {
                if self
                    .nested(child, tree, *item, depth + 1)?
                    .violation
                    .is_some()
                {
                    return Ok(Some(violation));
                }
                marks.items[index] = true;
            }
        }
        if let Some(child) = admitted.tree.field(schema, "contains") {
            let mut matched = 0;
            for (index, item) in items.iter().enumerate() {
                if self
                    .nested(child, tree, *item, depth + 1)?
                    .violation
                    .is_none()
                {
                    matched += 1;
                    marks.items[index] = true;
                }
            }
            let (min, max) = if admitted.dialect == Dialect::Draft202012 {
                (
                    self.optional_count(schema, "minContains")?.unwrap_or(1),
                    self.optional_count(schema, "maxContains")?
                        .unwrap_or(usize::MAX),
                )
            } else {
                (1, usize::MAX)
            };
            if matched < min || matched > max {
                return Ok(Some(Violation::Contains));
            }
        }
        Ok(None)
    }
    fn optional_count(&self, schema: usize, key: &str) -> Result<Option<usize>> {
        self.admitted
            .tree
            .field(schema, key)
            .map(|id| count(&self.admitted.tree, id, self.admitted.limits))
            .transpose()
    }
    fn container_length(
        &self,
        schema: usize,
        length: usize,
        min: &str,
        max: &str,
        lower: Violation,
        upper: Violation,
    ) -> Result<Option<Violation>> {
        if self
            .optional_count(schema, min)?
            .is_some_and(|value| length < value)
        {
            return Ok(Some(lower));
        }
        if self
            .optional_count(schema, max)?
            .is_some_and(|value| length > value)
        {
            return Ok(Some(upper));
        }
        Ok(None)
    }
    pub(super) fn object(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        let object = tree.object(instance).ok_or(Error::InvalidJson)?;
        if let Some(violation) = self.container_length(
            schema,
            object.len(),
            "minProperties",
            "maxProperties",
            Violation::MinProperties,
            Violation::MaxProperties,
        )? {
            return Ok(Some(violation));
        }
        if let Some(required) = admitted.tree.field(schema, "required")
            && !self.required(tree, instance, required)?
        {
            return Ok(Some(Violation::Required));
        }
        let properties = admitted
            .tree
            .field(schema, "properties")
            .and_then(|id| admitted.tree.object(id));
        let patterns = admitted
            .tree
            .field(schema, "patternProperties")
            .and_then(|id| admitted.tree.object(id));
        for (index, (name, item)) in object.iter().enumerate() {
            let mut covered = false;
            if let Some(child) = properties.and_then(|properties| properties.get(name)) {
                if self
                    .nested(*child, tree, *item, depth + 1)?
                    .violation
                    .is_some()
                {
                    return Ok(Some(Violation::Properties));
                }
                covered = true;
            }
            if let Some(patterns) = patterns {
                for (pattern, child) in patterns {
                    if self.pattern(pattern, name)? {
                        if self
                            .nested(*child, tree, *item, depth + 1)?
                            .violation
                            .is_some()
                        {
                            return Ok(Some(Violation::PatternProperties));
                        }
                        covered = true;
                    }
                }
            }
            if !covered && let Some(child) = admitted.tree.field(schema, "additionalProperties") {
                if self
                    .nested(child, tree, *item, depth + 1)?
                    .violation
                    .is_some()
                {
                    return Ok(Some(Violation::AdditionalProperties));
                }
                covered = true;
            }
            marks.properties[index] |= covered;
            if let Some(child) = admitted.tree.field(schema, "propertyNames") {
                let name = Tree {
                    nodes: vec![Node::string(name.clone())],
                };
                if self.nested(child, &name, 0, depth + 1)?.violation.is_some() {
                    return Ok(Some(Violation::PropertyNames));
                }
            }
        }
        self.dependencies(schema, tree, instance, depth, marks)
    }
    fn required(&self, tree: &Tree, instance: usize, required: usize) -> Result<bool> {
        let object = tree.object(instance).ok_or(Error::InvalidJson)?;
        for child in self
            .admitted
            .tree
            .array(required)
            .ok_or(Error::InvalidSchema)?
        {
            if !object.contains_key(
                self.admitted
                    .tree
                    .string(*child)
                    .ok_or(Error::InvalidSchema)?,
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn dependencies(
        &mut self,
        schema: usize,
        tree: &Tree,
        instance: usize,
        depth: usize,
        marks: &mut Marks,
    ) -> Result<Option<Violation>> {
        let admitted = self.admitted;
        let object = tree.object(instance).ok_or(Error::InvalidJson)?;
        for (key, violation) in [
            ("dependentRequired", Violation::DependentRequired),
            ("dependentSchemas", Violation::DependentSchemas),
            ("dependencies", Violation::Dependencies),
        ] {
            if (key == "dependencies") != (admitted.dialect == Dialect::Draft7) {
                continue;
            }
            if let Some(dependencies) = admitted.tree.field(schema, key) {
                for (name, child) in admitted
                    .tree
                    .object(dependencies)
                    .ok_or(Error::InvalidSchema)?
                {
                    if !object.contains_key(name) {
                        continue;
                    }
                    let valid = if admitted.tree.array(*child).is_some() {
                        self.required(tree, instance, *child)?
                    } else {
                        self.collect(*child, tree, instance, depth + 1, marks)?
                    };
                    if !valid {
                        return Ok(Some(violation));
                    }
                }
            }
        }
        Ok(None)
    }
}
