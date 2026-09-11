use std::collections::BTreeMap;
use std::sync::Arc;

use super::admit::children;
use super::json::{Node, Tree};
use super::{McpSchemaDialect as Dialect, McpSchemaError as Error, McpSchemaLimits, Result};

mod uri;

pub(super) struct Resource {
    uri: Arc<str>,
    root: usize,
}
pub(super) struct Resolver {
    resources: Vec<Resource>,
    by_uri: BTreeMap<Arc<str>, usize>,
    by_node: BTreeMap<usize, usize>,
    anchors: BTreeMap<(usize, String), (usize, bool)>,
    dialect: Dialect,
    limits: McpSchemaLimits,
    retained_bytes: usize,
    byte_limit: usize,
}
pub(super) struct Target {
    pub node: usize,
    pub resource: usize,
    pub dynamic: Option<String>,
}
impl Resolver {
    pub fn new(
        tree: &Tree,
        dialect: Dialect,
        limits: McpSchemaLimits,
        remaining: usize,
    ) -> Result<Self> {
        let root_uri = schema_id(tree, 0, dialect).map_or_else(
            || Ok(String::from("fx-schema:/root")),
            |root_id| identifier("fx-schema:/root", root_id, dialect, limits),
        )?;
        let mut resolver = Self {
            resources: Vec::new(),
            by_uri: BTreeMap::new(),
            by_node: BTreeMap::new(),
            anchors: BTreeMap::new(),
            dialect,
            limits,
            retained_bytes: 0,
            byte_limit: limits.max_reference_bytes.min(remaining),
        };
        resolver.resource(root_uri, 0)?;
        resolver.index(tree, 0, 0, 0)?;
        Ok(resolver)
    }
    fn index(&mut self, tree: &Tree, node: usize, inherited: usize, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth {
            return Err(Error::SchemaLimitExceeded);
        }
        let Some(object) = tree.object(node) else {
            return Ok(());
        };
        if self.dialect == Dialect::Draft7 && object.contains_key("$ref") {
            return Ok(());
        }
        let mut resource = inherited;
        if node != 0
            && let Some(id) = schema_id(tree, node, self.dialect)
        {
            let absolute = identifier(
                &self.resources[inherited].uri,
                id,
                self.dialect,
                self.limits,
            )?;
            if !(self.dialect == Dialect::Draft7
                && plain_identifier(id)
                && absolute == self.resources[inherited].uri.as_ref())
            {
                if self.by_uri.contains_key(absolute.as_str()) {
                    return Err(Error::InvalidSchema);
                }
                resource = self.resource(absolute, node)?;
            }
        }
        match self.dialect {
            Dialect::Draft202012 => {
                for (key, dynamic) in [("$anchor", false), ("$dynamicAnchor", true)] {
                    if let Some(value) = object.get(key) {
                        let name = tree.string(*value).ok_or(Error::InvalidSchema)?;
                        self.anchor(resource, name, node, dynamic)?;
                    }
                }
            }
            Dialect::Draft7 => {
                if let Some(id) = schema_id(tree, node, self.dialect)
                    && let Some((_, name)) = id.split_once('#')
                    && !name.is_empty()
                {
                    self.anchor(resource, name, node, false)?;
                }
            }
        }
        for child in children(tree, node, self.dialect) {
            self.index(tree, child, resource, depth + 1)?;
        }
        Ok(())
    }
    fn anchor(&mut self, resource: usize, name: &str, node: usize, dynamic: bool) -> Result<()> {
        if !valid_anchor(name) {
            return Err(Error::InvalidSchema);
        }
        // Charge before copying the name or allocating the sparse index entry.
        let mut charge = super::accounting::entry::<((usize, String), (usize, bool))>();
        super::accounting::add(&mut charge, name.len(), usize::MAX)?;
        super::accounting::add(&mut self.retained_bytes, charge, self.byte_limit)?;
        if self
            .anchors
            .insert((resource, name.into()), (node, dynamic))
            .is_some()
        {
            return Err(Error::InvalidSchema);
        }
        Ok(())
    }
    fn resource(&mut self, absolute: String, node: usize) -> Result<usize> {
        use super::accounting::{add, entry};
        let mut charge = absolute.len();
        add(&mut charge, 2 * size_of::<usize>(), usize::MAX)?;
        // Four resource slots per entry conservatively cover Vec growth, even
        // its first minimum allocation. Both maps share the single URI text.
        add(&mut charge, 4 * size_of::<Resource>(), usize::MAX)?;
        add(&mut charge, entry::<(Arc<str>, usize)>(), usize::MAX)?;
        add(&mut charge, entry::<(usize, usize)>(), usize::MAX)?;
        add(&mut self.retained_bytes, charge, self.byte_limit)?;
        let absolute: Arc<str> = absolute.into();
        let resource = self.resources.len();
        self.resources.push(Resource {
            uri: Arc::clone(&absolute),
            root: node,
        });
        self.by_uri.insert(absolute, resource);
        self.by_node.insert(node, resource);
        Ok(resource)
    }
    pub fn retained_byte_charge(&self) -> usize {
        self.retained_bytes
    }
    pub fn enter(&self, tree: &Tree, node: usize, inherited: usize) -> Result<usize> {
        let Some(id) = schema_id(tree, node, self.dialect) else {
            return Ok(inherited);
        };
        if let Some(resource) = self.by_node.get(&node) {
            return Ok(*resource);
        }
        if self.dialect == Dialect::Draft7 && plain_identifier(id) {
            return Ok(inherited);
        }
        Err(Error::InvalidSchema)
    }
    pub fn resolve(&self, tree: &Tree, current: usize, reference: &str) -> Result<Target> {
        let absolute = uri::resolve(
            &self.resources[current].uri,
            reference,
            self.limits.max_schema_bytes,
        )?;
        let (document, fragment) = absolute
            .split_once('#')
            .map_or((absolute.as_str(), ""), |parts| parts);
        let resource = *self.by_uri.get(document).ok_or(Error::ExternalReference)?;
        let mut node = self.resources[resource].root;
        if fragment.is_empty() {
            return Ok(Target {
                node,
                resource,
                dynamic: None,
            });
        }
        if let Some(pointer) = fragment.strip_prefix('/') {
            for (depth, segment) in pointer.split('/').enumerate() {
                if depth >= self.limits.max_depth {
                    return Err(Error::SchemaLimitExceeded);
                }
                let percent = percent_decode(segment)?;
                let segment = pointer_segment(&percent)?;
                node = match &tree.nodes[node] {
                    Node::Object(object) => {
                        *object.get(&segment).ok_or(Error::UnresolvedReference)?
                    }
                    Node::Array(array) => {
                        if segment.is_empty()
                            || (segment.len() > 1 && segment.starts_with('0'))
                            || !segment.bytes().all(|byte| byte.is_ascii_digit())
                        {
                            return Err(Error::UnresolvedReference);
                        }
                        *array
                            .get(
                                segment
                                    .parse::<usize>()
                                    .map_err(|_| Error::UnresolvedReference)?,
                            )
                            .ok_or(Error::UnresolvedReference)?
                    }
                    _ => return Err(Error::UnresolvedReference),
                };
            }
            return Ok(Target {
                node,
                resource,
                dynamic: None,
            });
        }
        let name = percent_decode(fragment)?;
        let (node, dynamic) = self
            .anchors
            .get(&(resource, name.clone()))
            .ok_or(Error::UnresolvedReference)?;
        Ok(Target {
            node: *node,
            resource,
            dynamic: dynamic.then_some(name),
        })
    }
    pub fn dynamic_anchor(&self, resource: usize, name: &str) -> Option<usize> {
        let (node, dynamic) = self.anchors.get(&(resource, name.to_owned()))?;
        dynamic.then_some(*node)
    }
}
fn schema_id(tree: &Tree, node: usize, dialect: Dialect) -> Option<&str> {
    if dialect == Dialect::Draft7 && tree.field(node, "$ref").is_some() {
        return None;
    }
    tree.field(node, "$id").and_then(|value| tree.string(value))
}
fn plain_identifier(id: &str) -> bool {
    id.split_once('#').is_some_and(|(_, name)| !name.is_empty())
}
fn identifier(base: &str, id: &str, dialect: Dialect, limits: McpSchemaLimits) -> Result<String> {
    let absolute = uri::resolve(base, id, limits.max_schema_bytes)?;
    let Some((document, fragment)) = absolute.split_once('#') else {
        return Ok(absolute);
    };
    if !fragment.is_empty() && (dialect != Dialect::Draft7 || !valid_anchor(fragment)) {
        return Err(Error::InvalidSchema);
    }
    Ok(document.into())
}
fn valid_anchor(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}
fn percent_decode(text: &str) -> Result<String> {
    let mut result = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let a = char::from(bytes.next().ok_or(Error::UnresolvedReference)?)
                .to_digit(16)
                .ok_or(Error::UnresolvedReference)?;
            let b = char::from(bytes.next().ok_or(Error::UnresolvedReference)?)
                .to_digit(16)
                .ok_or(Error::UnresolvedReference)?;
            result.push(u8::try_from(a * 16 + b).map_err(|_| Error::UnresolvedReference)?);
        } else {
            result.push(byte);
        }
    }
    String::from_utf8(result).map_err(|_| Error::UnresolvedReference)
}
fn pointer_segment(text: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        result.push(if character == '~' {
            match chars.next() {
                Some('0') => '~',
                Some('1') => '/',
                _ => return Err(Error::UnresolvedReference),
            }
        } else {
            character
        });
    }
    Ok(result)
}
