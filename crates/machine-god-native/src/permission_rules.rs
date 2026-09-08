//! Pure saved exact-action rule values; decoding is neither confirmation nor authority.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, Write};

use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::session_store::{MAX_FILE_SESSION_BYTES, MAX_STORED_JSON_NODES};

/// Reserved schema-1 exact-action rules. Absence means an empty rule set.
pub const NATIVE_SESSION_PERMISSION_RULES_KEY: &str = "machine_god.session_permission_rules";
/// Pinned maximum number of saved rules.
pub const MAX_NATIVE_PERMISSION_RULES: usize = 1024;
/// Pinned UTF-8 byte bound for canonical and display identities independently.
pub const MAX_NATIVE_PERMISSION_IDENTITY_BYTES: usize = 4096;

/// Exact identity namespace; equal canonical bytes in different kinds are distinct.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePermissionRuleKind {
    Command,
    FileMutation,
    StructuredTool,
}

/// Saved decision, not an execution grant or proof of confirmation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePermissionRuleDecision {
    Allow,
    Deny,
}

/// Fixed errors do not retain rejected identities or metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativePermissionRuleError {
    InvalidIdentity,
    InvalidDisplayIdentity,
    Malformed,
    UnsupportedVersion,
    Stale,
    Full,
    GenerationExhausted,
    Limit,
}

type Error = NativePermissionRuleError;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentity => "permission rule identity is invalid",
            Self::InvalidDisplayIdentity => "permission rule display identity is invalid",
            Self::Malformed => "saved permission rules are malformed",
            Self::UnsupportedVersion => "saved permission rule version is unsupported",
            Self::Stale => "permission rule generation is stale",
            Self::Full => "saved permission rule capacity is exhausted",
            Self::GenerationExhausted => "permission rule generation is exhausted",
            Self::Limit => "saved permission rules exceed the session envelope",
        })
    }
}

impl std::error::Error for Error {}

/// Owned canonical identity; all three fields participate in equality.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativePermissionRuleKey {
    kind: NativePermissionRuleKind,
    #[serde(serialize_with = "serialize_digest")]
    digest: [u8; 32],
    canonical: String,
}

impl NativePermissionRuleKey {
    /// # Errors
    /// Rejects empty identities and identities exceeding the pinned byte bound.
    pub fn new(kind: NativePermissionRuleKind, canonical: &str) -> Result<Self, Error> {
        validate_identity(canonical, Error::InvalidIdentity)?;
        Ok(Self {
            kind,
            digest: Sha256::digest(canonical.as_bytes()).into(),
            canonical: canonical.to_owned(),
        })
    }

    #[must_use]
    pub const fn kind(&self) -> NativePermissionRuleKind {
        self.kind
    }
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }
}

impl fmt::Debug for NativePermissionRuleKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePermissionRuleKey")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// An ordered saved rule with a stable ID and per-rule optimistic generation.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativePermissionRule {
    id: u64,
    key: NativePermissionRuleKey,
    display_identity: String,
    decision: NativePermissionRuleDecision,
    generation: u64,
}

impl NativePermissionRule {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }
    #[must_use]
    pub const fn key(&self) -> &NativePermissionRuleKey {
        &self.key
    }
    #[must_use]
    pub fn display_identity(&self) -> &str {
        &self.display_identity
    }
    #[must_use]
    pub const fn decision(&self) -> NativePermissionRuleDecision {
        self.decision
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl fmt::Debug for NativePermissionRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePermissionRule")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

/// Bounded owned rule set. Changes return candidates and perform no persistence.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeSessionPermissionRules {
    schema_version: u8,
    next_generation: u64,
    rules: Vec<NativePermissionRule>,
}

impl Default for NativeSessionPermissionRules {
    fn default() -> Self {
        Self {
            schema_version: 1,
            next_generation: 1,
            rules: Vec::new(),
        }
    }
}

impl fmt::Debug for NativeSessionPermissionRules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSessionPermissionRules")
            .field("next_generation", &self.next_generation)
            .field("rule_count", &self.rules.len())
            .finish_non_exhaustive()
    }
}

impl NativeSessionPermissionRules {
    #[must_use]
    pub const fn next_generation(&self) -> u64 {
        self.next_generation
    }
    #[must_use]
    pub fn rules(&self) -> &[NativePermissionRule] {
        &self.rules
    }
    #[must_use]
    pub fn rule_for_id(&self, id: u64) -> Option<&NativePermissionRule> {
        self.rules.iter().find(|rule| rule.id == id)
    }
    #[must_use]
    pub fn rule_for_key(&self, key: &NativePermissionRuleKey) -> Option<&NativePermissionRule> {
        self.rules.iter().find(|rule| rule.key == *key)
    }
    /// `None` is unresolved. The display string never participates in matching.
    #[must_use]
    pub fn decide(&self, key: &NativePermissionRuleKey) -> Option<NativePermissionRuleDecision> {
        self.rule_for_key(key).map(NativePermissionRule::decision)
    }

    /// Returns an owned insertion/replacement candidate without changing this set.
    /// An insertion requires no expected generation; replacement requires the
    /// exact current generation of that rule, not the set's next generation.
    ///
    /// # Errors
    /// Rejects invalid display identities, stale events, capacity, generation
    /// exhaustion, and candidates exceeding the native session envelope.
    pub fn apply_set(
        &self,
        key: &NativePermissionRuleKey,
        display_identity: &str,
        decision: NativePermissionRuleDecision,
        expected_generation: Option<u64>,
    ) -> Result<Self, Error> {
        validate_identity(display_identity, Error::InvalidDisplayIdentity)?;
        let next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        let index = self.rules.iter().position(|rule| rule.key == *key);
        if expected_generation != index.map(|index| self.rules[index].generation) {
            return Err(Error::Stale);
        }
        if index.is_none() && self.rules.len() == MAX_NATIVE_PERMISSION_RULES {
            return Err(Error::Full);
        }
        let mut views: Vec<_> = self.rules.iter().map(RuleView::from).collect();
        let replacement = RuleView {
            id: index.map_or(self.next_generation, |index| self.rules[index].id),
            key,
            display_identity,
            decision,
            generation: self.next_generation,
        };
        if let Some(index) = index {
            views[index] = replacement;
        } else {
            views.push(replacement);
        }
        let candidate = RulesView {
            schema_version: 1,
            next_generation,
            rules: views,
        };
        check_envelope(&candidate, candidate.rules.len())?;
        Ok(Self {
            schema_version: 1,
            next_generation,
            rules: candidate
                .rules
                .into_iter()
                .map(RuleView::into_owned)
                .collect(),
        })
    }

    /// Returns an owned candidate with the exact rule removed in stable order.
    ///
    /// # Errors
    /// Rejects unknown IDs, stale per-rule generations and generation exhaustion.
    pub fn apply_revoke(&self, id: u64, expected_generation: u64) -> Result<Self, Error> {
        let index = self
            .rules
            .iter()
            .position(|rule| rule.id == id)
            .ok_or(Error::Stale)?;
        if self.rules[index].generation != expected_generation {
            return Err(Error::Stale);
        }
        let next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        let candidate = RulesView {
            schema_version: 1,
            next_generation,
            rules: self
                .rules
                .iter()
                .enumerate()
                .filter(|(position, _)| *position != index)
                .map(|(_, rule)| RuleView::from(rule))
                .collect(),
        };
        check_envelope(&candidate, candidate.rules.len())?;
        Ok(Self {
            schema_version: 1,
            next_generation,
            rules: candidate
                .rules
                .into_iter()
                .map(RuleView::into_owned)
                .collect(),
        })
    }

    /// Reads only the reserved entry, leaving unrelated metadata untraversed.
    ///
    /// # Errors
    /// Rejects a malformed or oversized present entry; absence is empty.
    pub fn from_metadata(metadata: &BTreeMap<String, Value>) -> Result<Self, Error> {
        metadata
            .get(NATIVE_SESSION_PERMISSION_RULES_KEY)
            .map_or_else(|| Ok(Self::default()), Self::from_value)
    }

    /// Strictly validates a shallow schema before cloning any identity text.
    /// Digest encodings must be lowercase hexadecimal and match canonical bytes.
    ///
    /// # Errors
    /// Rejects unknown/missing fields, versions, types, duplicate IDs or exact
    /// keys, invalid generations and values exceeding pinned/session bounds.
    pub fn from_value(value: &Value) -> Result<Self, Error> {
        let object = object(value, 3)?;
        if number(object, "schema_version")? != 1 {
            return Err(Error::UnsupportedVersion);
        }
        let next_generation = number(object, "next_generation")?;
        if next_generation == 0 {
            return Err(Error::Malformed);
        }
        let rules = object
            .get("rules")
            .and_then(Value::as_array)
            .ok_or(Error::Malformed)?;
        if rules.len() > MAX_NATIVE_PERMISSION_RULES {
            return Err(Error::Full);
        }
        let mut parsed = Vec::with_capacity(rules.len());
        let mut ids = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for rule in rules {
            let rule = ParsedRule::parse(rule, next_generation)?;
            if !ids.insert(rule.id) || !keys.insert((rule.kind, rule.digest, rule.canonical)) {
                return Err(Error::Malformed);
            }
            parsed.push(rule);
        }
        check_envelope(value, rules.len())?;
        Ok(Self {
            schema_version: 1,
            next_generation,
            rules: parsed.into_iter().map(ParsedRule::into_owned).collect(),
        })
    }

    /// Returns only the validated bounded value, not a saved record or authority.
    #[must_use]
    pub fn to_value(&self) -> Value {
        // Every field is an infallibly serializable scalar or bounded collection.
        serde_json::json!(self)
    }
}

#[derive(Serialize)]
struct RulesView<'a> {
    schema_version: u8,
    next_generation: u64,
    rules: Vec<RuleView<'a>>,
}
#[derive(Serialize)]
struct RuleView<'a> {
    id: u64,
    key: &'a NativePermissionRuleKey,
    display_identity: &'a str,
    decision: NativePermissionRuleDecision,
    generation: u64,
}
impl<'a> From<&'a NativePermissionRule> for RuleView<'a> {
    fn from(rule: &'a NativePermissionRule) -> Self {
        Self {
            id: rule.id,
            key: &rule.key,
            display_identity: &rule.display_identity,
            decision: rule.decision,
            generation: rule.generation,
        }
    }
}
impl RuleView<'_> {
    fn into_owned(self) -> NativePermissionRule {
        NativePermissionRule {
            id: self.id,
            key: self.key.clone(),
            display_identity: self.display_identity.to_owned(),
            decision: self.decision,
            generation: self.generation,
        }
    }
}

struct ParsedRule<'a> {
    id: u64,
    kind: NativePermissionRuleKind,
    digest: [u8; 32],
    canonical: &'a str,
    display_identity: &'a str,
    decision: NativePermissionRuleDecision,
    generation: u64,
}
impl<'a> ParsedRule<'a> {
    fn parse(value: &'a Value, next_generation: u64) -> Result<Self, Error> {
        let rule = object(value, 5)?;
        let id = number(rule, "id")?;
        let generation = number(rule, "generation")?;
        if id == 0 || id > generation || generation >= next_generation {
            return Err(Error::Malformed);
        }
        let key = object(rule.get("key").ok_or(Error::Malformed)?, 3)?;
        let kind = match string(key, "kind")? {
            "command" => NativePermissionRuleKind::Command,
            "file_mutation" => NativePermissionRuleKind::FileMutation,
            "structured_tool" => NativePermissionRuleKind::StructuredTool,
            _ => return Err(Error::Malformed),
        };
        let canonical = string(key, "canonical")?;
        validate_identity(canonical, Error::InvalidIdentity)?;
        let digest: [u8; 32] = Sha256::digest(canonical.as_bytes()).into();
        let encoded_digest = string(key, "digest")?;
        if encoded_digest.as_bytes() != digest_hex(&digest) {
            return Err(Error::Malformed);
        }
        let display_identity = string(rule, "display_identity")?;
        validate_identity(display_identity, Error::InvalidDisplayIdentity)?;
        let decision = match string(rule, "decision")? {
            "allow" => NativePermissionRuleDecision::Allow,
            "deny" => NativePermissionRuleDecision::Deny,
            _ => return Err(Error::Malformed),
        };
        Ok(Self {
            id,
            kind,
            digest,
            canonical,
            display_identity,
            decision,
            generation,
        })
    }
    fn into_owned(self) -> NativePermissionRule {
        NativePermissionRule {
            id: self.id,
            key: NativePermissionRuleKey {
                kind: self.kind,
                digest: self.digest,
                canonical: self.canonical.to_owned(),
            },
            display_identity: self.display_identity.to_owned(),
            decision: self.decision,
            generation: self.generation,
        }
    }
}

fn validate_identity(value: &str, error: Error) -> Result<(), Error> {
    if value.is_empty() || value.len() > MAX_NATIVE_PERMISSION_IDENTITY_BYTES {
        Err(error)
    } else {
        Ok(())
    }
}
fn object(value: &Value, fields: usize) -> Result<&serde_json::Map<String, Value>, Error> {
    value
        .as_object()
        .filter(|object| object.len() == fields)
        .ok_or(Error::Malformed)
}
fn number(object: &serde_json::Map<String, Value>, name: &str) -> Result<u64, Error> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .ok_or(Error::Malformed)
}
fn string<'a>(object: &'a serde_json::Map<String, Value>, name: &str) -> Result<&'a str, Error> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or(Error::Malformed)
}
fn digest_hex(digest: &[u8; 32]) -> [u8; 64] {
    let mut encoded = [0; 64];
    let hex = b"0123456789abcdef";
    for (index, byte) in digest.iter().enumerate() {
        encoded[index * 2] = hex[usize::from(byte >> 4)];
        encoded[index * 2 + 1] = hex[usize::from(byte & 15)];
    }
    encoded
}
fn serialize_digest<S: Serializer>(digest: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
    let bytes = digest_hex(digest);
    let text = std::str::from_utf8(&bytes).map_err(serde::ser::Error::custom)?;
    serializer.serialize_str(text)
}
fn check_envelope(value: &impl Serialize, rules: usize) -> Result<(), Error> {
    // Root object, two numbers, array; each rule adds its object, five direct
    // fields (including the key object), and that key object's three scalars.
    if rules
        .checked_mul(9)
        .and_then(|nodes| nodes.checked_add(4))
        .is_none_or(|nodes| nodes > MAX_STORED_JSON_NODES)
    {
        return Err(Error::Limit);
    }
    serde_json::to_writer(ByteBudget(0), value).map_err(|_| Error::Limit)
}
struct ByteBudget(usize);
impl Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_FILE_SESSION_BYTES)
            .ok_or_else(|| io::Error::other("permission rule byte limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colliding_digests_do_not_replace_canonical_identity() {
        let first =
            NativePermissionRuleKey::new(NativePermissionRuleKind::Command, "first").unwrap();
        let mut collision =
            NativePermissionRuleKey::new(NativePermissionRuleKind::Command, "second").unwrap();
        collision.digest = first.digest;
        assert_ne!(first, collision);
        let rules = NativeSessionPermissionRules::default()
            .apply_set(&first, "display", NativePermissionRuleDecision::Allow, None)
            .unwrap();
        assert_eq!(rules.decide(&collision), None);
    }
}
