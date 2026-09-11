use super::{McpCatalogError as Error, McpCatalogKind, Result, fields, template};
use crate::mcp::schema::{McpSchema, McpSchemaLimits};
use serde_json::value::RawValue;
use std::{fmt, sync::Arc};

#[derive(Clone)]
pub enum McpDescriptor {
    Tool(McpToolDescriptor),
    Resource(McpResourceDescriptor),
    ResourceTemplate(McpResourceTemplateDescriptor),
    Prompt(McpPromptDescriptor),
}
impl fmt::Debug for McpDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpDescriptor { .. }")
    }
}
struct Common {
    raw: Box<RawValue>,
    name: Box<str>,
    title: Option<Box<str>>,
    description: Option<Box<str>>,
    mime_type: Option<Box<str>>,
    icons: Option<Box<RawValue>>,
    annotations: Option<Box<RawValue>>,
    metadata: Option<Box<RawValue>>,
}
struct Tool {
    common: Common,
    input: McpSchema,
    output: Option<McpSchema>,
}
struct Resource {
    common: Common,
    uri: Box<str>,
    size: Option<u64>,
}
struct Template {
    common: Common,
    uri: Box<str>,
}
struct Prompt {
    common: Common,
    arguments: Box<[McpPromptArgument]>,
}

macro_rules! descriptor {
    ($name:ident, $inner:ident) => {
        #[derive(Clone)]
        pub struct $name(Arc<$inner>);
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), " { .. }"))
            }
        }
        impl $name {
            #[must_use]
            pub fn raw_json(&self) -> &RawValue {
                &self.0.common.raw
            }
            #[must_use]
            pub fn name(&self) -> &str {
                &self.0.common.name
            }
            #[must_use]
            pub fn title(&self) -> Option<&str> {
                self.0.common.title.as_deref()
            }
            #[must_use]
            pub fn description(&self) -> Option<&str> {
                self.0.common.description.as_deref()
            }
            #[must_use]
            pub fn icons_json(&self) -> Option<&RawValue> {
                self.0.common.icons.as_deref()
            }
            #[must_use]
            pub fn annotations_json(&self) -> Option<&RawValue> {
                self.0.common.annotations.as_deref()
            }
            #[must_use]
            pub fn metadata_json(&self) -> Option<&RawValue> {
                self.0.common.metadata.as_deref()
            }
        }
    };
}
descriptor!(McpToolDescriptor, Tool);
descriptor!(McpResourceDescriptor, Resource);
descriptor!(McpResourceTemplateDescriptor, Template);
descriptor!(McpPromptDescriptor, Prompt);

impl McpToolDescriptor {
    #[must_use]
    pub fn input_schema(&self) -> &McpSchema {
        &self.0.input
    }
    #[must_use]
    pub fn output_schema(&self) -> Option<&McpSchema> {
        self.0.output.as_ref()
    }
    #[must_use]
    pub fn effective_description(&self) -> &str {
        self.description()
            .filter(|text| !text.is_empty())
            .unwrap_or("MCP tool")
    }
}
impl McpResourceDescriptor {
    #[must_use]
    pub fn uri(&self) -> &str {
        &self.0.uri
    }
    #[must_use]
    pub fn size(&self) -> Option<u64> {
        self.0.size
    }
    #[must_use]
    pub fn mime_type(&self) -> Option<&str> {
        self.0.common.mime_type.as_deref()
    }
}
impl McpResourceTemplateDescriptor {
    #[must_use]
    pub fn uri_template(&self) -> &str {
        &self.0.uri
    }
    #[must_use]
    pub fn mime_type(&self) -> Option<&str> {
        self.0.common.mime_type.as_deref()
    }
}
pub struct McpPromptArgument {
    name: Box<str>,
    description: Option<Box<str>>,
    required: bool,
}
impl fmt::Debug for McpPromptArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpPromptArgument { .. }")
    }
}
impl McpPromptArgument {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
}
impl McpPromptDescriptor {
    #[must_use]
    pub fn arguments(&self) -> &[McpPromptArgument] {
        &self.0.arguments
    }
    #[must_use]
    pub fn argument_named(&self, name: &str) -> Option<&McpPromptArgument> {
        self.0
            .arguments
            .iter()
            .find(|argument| argument.name() == name)
    }
}

pub(super) fn parse(kind: McpCatalogKind, identity: &str, raw: &RawValue) -> Result<McpDescriptor> {
    let object = fields::object(raw)?;
    let common = common(kind, raw, &object)?;
    match kind {
        McpCatalogKind::Tools => {
            if common.name.as_ref() != identity {
                return Err(Error::InvalidDescriptor);
            }
            let input = schema(object.get("inputSchema").ok_or(Error::InvalidDescriptor)?)?;
            input.require_object_root().map_err(Error::Schema)?;
            let output = object
                .get("outputSchema")
                .map(|raw| schema(raw))
                .transpose()?;
            Ok(McpDescriptor::Tool(McpToolDescriptor(Arc::new(Tool {
                common,
                input,
                output,
            }))))
        }
        McpCatalogKind::Resources => {
            let uri = fields::required(&object, "uri", 64 * 1024)?;
            if uri.as_ref() != identity {
                return Err(Error::InvalidDescriptor);
            }
            let size = object
                .get("size")
                .map(|value| fields::size(value.get()))
                .transpose()?;
            Ok(McpDescriptor::Resource(McpResourceDescriptor(Arc::new(
                Resource { common, uri, size },
            ))))
        }
        McpCatalogKind::ResourceTemplates => {
            let uri = fields::required(&object, "uriTemplate", 64 * 1024)?;
            if uri.as_ref() != identity || !template::valid(&uri) {
                return Err(Error::InvalidDescriptor);
            }
            // The producer validates size on templates, although it does not expose it.
            if let Some(size) = object.get("size") {
                fields::size(size.get())?;
            }
            Ok(McpDescriptor::ResourceTemplate(
                McpResourceTemplateDescriptor(Arc::new(Template { common, uri })),
            ))
        }
        McpCatalogKind::Prompts => {
            if common.name.as_ref() != identity {
                return Err(Error::InvalidDescriptor);
            }
            let mut arguments = Vec::new();
            let mut names = std::collections::BTreeSet::new();
            if let Some(values) = object.get("arguments") {
                let values = fields::array(values)?;
                if values.len() > 128 {
                    return Err(Error::Limit);
                }
                for value in values {
                    let object = fields::object(value)?;
                    let name = fields::required(&object, "name", 256)?;
                    if !names.insert(name.clone()) {
                        return Err(Error::InvalidDescriptor);
                    }
                    let description = fields::optional(&object, "description", 64 * 1024)?;
                    let required = object
                        .get("required")
                        .map_or(Ok(false), |raw| fields::boolean(raw))?;
                    arguments.push(McpPromptArgument {
                        name,
                        description,
                        required,
                    });
                }
            }
            Ok(McpDescriptor::Prompt(McpPromptDescriptor(Arc::new(
                Prompt {
                    common,
                    arguments: arguments.into_boxed_slice(),
                },
            ))))
        }
    }
}
fn schema(raw: &RawValue) -> Result<McpSchema> {
    McpSchema::parse(raw.get().as_bytes(), McpSchemaLimits::default()).map_err(Error::Schema)
}
fn common(kind: McpCatalogKind, raw: &RawValue, object: &fields::Object<'_>) -> Result<Common> {
    let tool = kind == McpCatalogKind::Tools;
    let prompt = kind == McpCatalogKind::Prompts;
    let name = fields::required(object, "name", 256)?;
    let title = fields::optional(object, "title", 4096)?;
    let description = fields::optional(object, "description", 64 * 1024)?;
    let mime_type = if tool || prompt {
        None
    } else {
        fields::optional(object, "mimeType", 4096)?
    };
    let depth = if tool { 64 } else { 32 };
    let icons = object
        .get("icons")
        .map(|raw| {
            fields::icons(raw, if tool { 1024 * 1024 } else { 64 * 1024 })?;
            fields::metadata(raw, depth, false)
        })
        .transpose()?;
    let annotations = if prompt {
        None
    } else {
        object
            .get("annotations")
            .map(|raw| {
                fields::annotations(raw, tool)?;
                fields::metadata(raw, depth, true)
            })
            .transpose()?
    };
    let metadata = object
        .get("_meta")
        .map(|raw| fields::metadata(raw, depth, true))
        .transpose()?;
    Ok(Common {
        raw: raw.to_owned(),
        name,
        title,
        description,
        mime_type,
        icons,
        annotations,
        metadata,
    })
}
