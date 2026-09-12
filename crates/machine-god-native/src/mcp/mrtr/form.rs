use super::{
    Error, McpMrtrLimits, Result,
    bounds::{self, Object},
    strings,
};
use crate::mcp::schema::pattern::{Pattern, PatternError};
use serde_json::value::RawValue;
use std::{cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpFormFieldKind {
    String,
    Number,
    Integer,
    Boolean,
    SingleSelect,
    MultiSelect,
}
pub struct McpFormChoice {
    value: Box<str>,
    title: Box<str>,
}
impl McpFormChoice {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}
impl fmt::Debug for McpFormChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFormChoice { .. }")
    }
}
pub struct McpFormField {
    name: Box<str>,
    title: Option<Box<str>>,
    description: Option<Box<str>>,
    kind: McpFormFieldKind,
    required: bool,
    raw: Box<RawValue>,
    choices: Box<[McpFormChoice]>,
    default: Option<Box<RawValue>>,
    min: Option<usize>,
    max: Option<usize>,
    minimum: Option<Box<RawValue>>,
    maximum: Option<Box<RawValue>>,
    multiple: Option<Box<RawValue>>,
    format: Option<Box<str>>,
    pattern: Option<Box<str>>,
}
impl fmt::Debug for McpFormField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpFormField")
            .field("kind", &self.kind)
            .field("required", &self.required)
            .finish_non_exhaustive()
    }
}
impl McpFormField {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    #[must_use]
    pub const fn kind(&self) -> McpFormFieldKind {
        self.kind
    }
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn choices(&self) -> &[McpFormChoice] {
        &self.choices
    }
    #[must_use]
    pub fn default_json(&self) -> Option<&RawValue> {
        self.default.as_deref()
    }
    /// Validate a UI field without retaining or displaying its value.
    /// # Errors
    /// Invalid values, duplicate JSON keys and exhausted bounds fail.
    pub fn validate_json(&self, value: &RawValue, limits: McpMrtrLimits) -> Result<()> {
        bounds::admit(value, limits)?;
        self.validate_admitted(value, limits)
    }
    fn validate_admitted(&self, raw: &RawValue, limits: McpMrtrLimits) -> Result<()> {
        match self.kind {
            McpFormFieldKind::String => {
                let text = bounds::text(raw, limits.max_string_bytes)?;
                let length = text.chars().count();
                if self.min.is_some_and(|min| length < min)
                    || self.max.is_some_and(|max| length > max)
                {
                    return Err(Error::InvalidResponse);
                }
                if let Some(source) = &self.pattern {
                    let pattern = compile(source, limits)?;
                    if !pattern
                        .matches(&text, &mut 0, limits.max_pattern_steps)
                        .map_err(pattern_error)?
                    {
                        return Err(Error::InvalidResponse);
                    }
                }
                if self
                    .format
                    .as_deref()
                    .is_some_and(|format| !strings::format(format, &text))
                {
                    return Err(Error::InvalidResponse);
                }
            }
            McpFormFieldKind::Number | McpFormFieldKind::Integer => {
                let value = bounds::number(raw, limits)?;
                if self.kind == McpFormFieldKind::Integer && !value.integer() {
                    return Err(Error::InvalidResponse);
                }
                if let Some(minimum) = &self.minimum
                    && value.order(&bounds::number(minimum, limits)?) == Ordering::Less
                {
                    return Err(Error::InvalidResponse);
                }
                if let Some(maximum) = &self.maximum
                    && value.order(&bounds::number(maximum, limits)?) == Ordering::Greater
                {
                    return Err(Error::InvalidResponse);
                }
                if let Some(multiple) = &self.multiple
                    && !value
                        .multiple_of(&bounds::number(multiple, limits)?, limits.scalar_limits())
                        .map_err(|_| Error::Limit)?
                {
                    return Err(Error::InvalidResponse);
                }
            }
            McpFormFieldKind::Boolean => {
                if !matches!(raw.get(), "true" | "false") {
                    return Err(Error::InvalidResponse);
                }
            }
            McpFormFieldKind::SingleSelect => {
                let value = bounds::text(raw, limits.max_string_bytes)?;
                if !self.choices.iter().any(|choice| choice.value == value) {
                    return Err(Error::InvalidResponse);
                }
            }
            McpFormFieldKind::MultiSelect => {
                let values = bounds::array(raw)?;
                if values.len() > self.choices.len()
                    || self.min.is_some_and(|min| values.len() < min)
                    || self.max.is_some_and(|max| values.len() > max)
                {
                    return Err(Error::InvalidResponse);
                }
                let mut seen = Vec::with_capacity(values.len());
                for raw in values {
                    let value = bounds::text(raw, limits.max_string_bytes)?;
                    if !self.choices.iter().any(|choice| choice.value == value)
                        || seen.contains(&value)
                    {
                        return Err(Error::InvalidResponse);
                    }
                    seen.push(value);
                }
            }
        }
        Ok(())
    }
}

pub struct McpFormSchema {
    raw: Box<RawValue>,
    fields: Box<[McpFormField]>,
}
impl fmt::Debug for McpFormSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpFormSchema")
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}
impl McpFormSchema {
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn fields(&self) -> &[McpFormField] {
        &self.fields
    }
    pub(super) fn parse_admitted(
        raw: &RawValue,
        limits: McpMrtrLimits,
        form_bounds: bounds::FormBounds,
    ) -> Result<Self> {
        let schema = bounds::object(raw)?;
        if schema.contains_key("title") || schema.contains_key("description") {
            return Err(Error::UnsupportedSchema);
        }
        if schema
            .get("additionalProperties")
            .is_some_and(|raw| raw.get() != "false")
        {
            return Err(Error::UnsupportedSchema);
        }
        if bounds::text(bounds::required(&schema, "type")?, limits.max_name_bytes)?.as_ref()
            != "object"
        {
            return Err(Error::InvalidSchema);
        }
        let properties_raw = bounds::required(&schema, "properties")?;
        let properties = bounds::object(properties_raw)?;
        if properties.len() > form_bounds.fields {
            return Err(Error::Limit);
        }
        let mut required = Vec::new();
        if let Some(raw) = schema.get("required") {
            let names = bounds::array(raw)?;
            if names.len() > form_bounds.fields {
                return Err(Error::Limit);
            }
            for raw in names {
                let name = bounds::text(raw, limits.max_name_bytes)?;
                if name.is_empty()
                    || !properties.contains_key(name.as_ref())
                    || required.contains(&name)
                {
                    return Err(Error::InvalidSchema);
                }
                required.push(name);
            }
        }
        let mut fields = Vec::with_capacity(properties.len());
        for (name, raw) in bounds::entries(properties_raw)? {
            if name.is_empty() {
                return Err(Error::InvalidSchema);
            }
            let required = required.iter().any(|value| value.as_ref() == name);
            fields.push(parse_field(
                name.into_boxed_str(),
                raw,
                required,
                limits,
                form_bounds,
            )?);
        }
        Ok(Self {
            raw: super::raw(raw),
            fields: fields.into_boxed_slice(),
        })
    }
    pub(super) fn validate_admitted(&self, raw: &RawValue, limits: McpMrtrLimits) -> Result<()> {
        let content = bounds::object(raw).map_err(|_| Error::InvalidResponse)?;
        if content.len() > self.fields.len() {
            return Err(Error::InvalidResponse);
        }
        for field in &self.fields {
            match content.get(field.name()) {
                Some(raw) => field.validate_admitted(raw, limits)?,
                None if field.required => return Err(Error::InvalidResponse),
                None => {}
            }
        }
        if content
            .keys()
            .any(|name| !self.fields.iter().any(|field| field.name() == name))
        {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }
}
fn parse_field(
    name: Box<str>,
    raw: &RawValue,
    required: bool,
    limits: McpMrtrLimits,
    form_bounds: bounds::FormBounds,
) -> Result<McpFormField> {
    let schema = bounds::object(raw)?;
    let title = bounds::optional_text(&schema, "title", form_bounds.label)?;
    let description = bounds::optional_text(&schema, "description", form_bounds.label)?;
    if strings::secret(&name) || title.as_deref().is_some_and(strings::secret) {
        return Err(Error::SecretField);
    }
    let kind =
        match bounds::text(bounds::required(&schema, "type")?, limits.max_name_bytes)?.as_ref() {
            "string" => McpFormFieldKind::String,
            "number" => McpFormFieldKind::Number,
            "integer" => McpFormFieldKind::Integer,
            "boolean" => McpFormFieldKind::Boolean,
            "array" => McpFormFieldKind::MultiSelect,
            _ => return Err(Error::UnsupportedSchema),
        };
    let mut field = McpFormField {
        name,
        title,
        description,
        kind,
        required,
        raw: super::raw(raw),
        choices: Box::new([]),
        default: None,
        min: None,
        max: None,
        minimum: None,
        maximum: None,
        multiple: None,
        format: None,
        pattern: None,
    };
    match kind {
        McpFormFieldKind::String => {
            string_constraints(&mut field, &schema, limits, form_bounds)?;
        }
        McpFormFieldKind::Number | McpFormFieldKind::Integer => {
            number_constraints(&mut field, &schema, limits)?;
        }
        McpFormFieldKind::MultiSelect => {
            multi_constraints(&mut field, &schema, limits, form_bounds)?;
        }
        _ => {}
    }
    if let Some(default) = schema.get("default") {
        if bounds::compact_len(default)? > limits.max_string_bytes {
            return Err(Error::Limit);
        }
        field.validate_admitted(default, limits)?;
        field.default = Some(super::raw(default));
    }
    Ok(field)
}
fn string_constraints(
    field: &mut McpFormField,
    schema: &Object<'_>,
    limits: McpMrtrLimits,
    form_bounds: bounds::FormBounds,
) -> Result<()> {
    (field.min, field.max) = range(schema, "minLength", "maxLength", limits.max_string_bytes)?;
    field.format = bounds::optional_text(schema, "format", limits.max_name_bytes)?;
    if field
        .format
        .as_deref()
        .is_some_and(|value| !matches!(value, "email" | "uri" | "date" | "date-time"))
    {
        return Err(Error::UnsupportedSchema);
    }
    field.pattern = bounds::optional_text(schema, "pattern", limits.max_pattern_bytes)?;
    if let Some(pattern) = &field.pattern {
        compile(pattern, limits)?;
    }
    if let Some(choices) = schema.get("oneOf") {
        field.choices = titled_choices(choices, limits, form_bounds)?;
        field.kind = McpFormFieldKind::SingleSelect;
    } else if let Some(choices) = schema.get("enum") {
        field.choices = enum_choices(
            choices,
            schema.get("enumNames").copied(),
            limits,
            form_bounds,
        )?;
        field.kind = McpFormFieldKind::SingleSelect;
    }
    Ok(())
}
fn number_constraints(
    field: &mut McpFormField,
    schema: &Object<'_>,
    limits: McpMrtrLimits,
) -> Result<()> {
    for name in ["minimum", "maximum", "multipleOf"] {
        if let Some(raw) = schema.get(name) {
            bounds::number(raw, limits)?;
        }
    }
    field.minimum = schema.get("minimum").map(|raw| super::raw(raw));
    field.maximum = schema.get("maximum").map(|raw| super::raw(raw));
    field.multiple = schema.get("multipleOf").map(|raw| super::raw(raw));
    if let (Some(min), Some(max)) = (&field.minimum, &field.maximum)
        && bounds::number(min, limits)?.order(&bounds::number(max, limits)?) == Ordering::Greater
    {
        return Err(Error::InvalidSchema);
    }
    if let Some(multiple) = &field.multiple {
        let value = bounds::number(multiple, limits)?;
        if value.negative || value.zero() {
            return Err(Error::InvalidSchema);
        }
    }
    Ok(())
}
fn multi_constraints(
    field: &mut McpFormField,
    schema: &Object<'_>,
    limits: McpMrtrLimits,
    form_bounds: bounds::FormBounds,
) -> Result<()> {
    (field.min, field.max) = range(schema, "minItems", "maxItems", form_bounds.options)?;
    let items = bounds::object(bounds::required(schema, "items")?)?;
    if let Some(choices) = items.get("anyOf") {
        if let Some(kind) = bounds::optional_text(&items, "type", limits.max_name_bytes)?
            && kind.as_ref() != "string"
        {
            return Err(Error::UnsupportedSchema);
        }
        field.choices = titled_choices(choices, limits, form_bounds)?;
    } else if let Some(choices) = items.get("enum") {
        if bounds::text(bounds::required(&items, "type")?, limits.max_name_bytes)?.as_ref()
            != "string"
        {
            return Err(Error::UnsupportedSchema);
        }
        field.choices = enum_choices(
            choices,
            items.get("enumNames").copied(),
            limits,
            form_bounds,
        )?;
    } else {
        return Err(Error::InvalidSchema);
    }
    Ok(())
}
fn range(
    schema: &Object<'_>,
    minimum: &str,
    maximum: &str,
    cap: usize,
) -> Result<(Option<usize>, Option<usize>)> {
    let parse = |key| -> Result<Option<usize>> {
        schema
            .get(key)
            .map(|raw| {
                // Pinned optionalUsize accepts integer lexemes, not 1.0 or 1e0.
                let value = raw.get().parse::<i64>().map_err(|_| Error::InvalidSchema)?;
                let value = usize::try_from(value).map_err(|_| Error::InvalidSchema)?;
                if value > cap {
                    return Err(Error::Limit);
                }
                Ok(value)
            })
            .transpose()
    };
    let min = parse(minimum)?;
    let max = parse(maximum)?;
    if min.zip(max).is_some_and(|(min, max)| min > max) {
        return Err(Error::InvalidSchema);
    }
    Ok((min, max))
}
fn titled_choices(
    raw: &RawValue,
    limits: McpMrtrLimits,
    form_bounds: bounds::FormBounds,
) -> Result<Box<[McpFormChoice]>> {
    let items = bounds::array(raw)?;
    if items.is_empty() || items.len() > form_bounds.options {
        return Err(Error::InvalidSchema);
    }
    let mut choices = Vec::with_capacity(items.len());
    for item in items {
        let fields = bounds::object(item)?;
        if fields.contains_key("description") {
            return Err(Error::UnsupportedSchema);
        }
        let value = bounds::text(bounds::required(&fields, "const")?, limits.max_string_bytes)?;
        let title = bounds::text(bounds::required(&fields, "title")?, form_bounds.label)?;
        push_choice(&mut choices, value, title)?;
    }
    Ok(choices.into_boxed_slice())
}
fn enum_choices(
    raw: &RawValue,
    names: Option<&RawValue>,
    limits: McpMrtrLimits,
    form_bounds: bounds::FormBounds,
) -> Result<Box<[McpFormChoice]>> {
    let values = bounds::array(raw)?;
    if values.is_empty() || values.len() > form_bounds.options {
        return Err(Error::InvalidSchema);
    }
    let names = names.map(bounds::array).transpose()?;
    if names
        .as_ref()
        .is_some_and(|names| names.len() != values.len())
    {
        return Err(Error::InvalidSchema);
    }
    let mut choices = Vec::with_capacity(values.len());
    for (index, raw) in values.iter().enumerate() {
        let value = bounds::text(raw, limits.max_string_bytes)?;
        let title = bounds::text(
            names.as_ref().map_or(*raw, |names| names[index]),
            form_bounds.label,
        )?;
        push_choice(&mut choices, value, title)?;
    }
    Ok(choices.into_boxed_slice())
}
fn push_choice(choices: &mut Vec<McpFormChoice>, value: Box<str>, title: Box<str>) -> Result<()> {
    if choices.iter().any(|choice| choice.value == value) {
        return Err(Error::InvalidSchema);
    }
    choices.push(McpFormChoice { value, title });
    Ok(())
}
fn compile(source: &str, limits: McpMrtrLimits) -> Result<Pattern> {
    Pattern::compile(source, limits.scalar_limits()).map_err(pattern_error)
}
fn pattern_error(error: PatternError) -> Error {
    match error {
        PatternError::Unsupported => Error::InvalidSchema,
        PatternError::Limit => Error::Limit,
    }
}
