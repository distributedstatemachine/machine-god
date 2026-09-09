//! Pure, bounded workspace-scoped configured permissions. No path is opened.

use std::{collections::BTreeSet, fmt, path::Path};

use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::{
    CONFIG_SCHEMA_VERSION, MAX_CONFIG_BYTES, NativeConfiguredPermissionDecision,
    NativeConfiguredPermissionRule,
};
use super::{
    NativeConfig, NativeConfigError, NativeConfigErrorKind, NativeConfiguredPermissionRules,
    WirePermissionRule, decode_permission_rules,
};

// Matches the native session metadata path contract, without requiring that
// Unix-only module merely to parse configuration on other targets.
const MAX_WORKSPACE_BYTES: usize = 4096;

/// The persistent user list or the exact selected workspace's local list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConfiguredPermissionScope {
    User,
    Local,
}

/// Categories whose allow decisions are removed; ask and deny are preserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConfiguredPermissionReset {
    Commands,
    Tools,
    Urls,
    WebFetchDomains,
    All,
}

/// An explicit storage edit, not a slash parser, tool lookup or grant.
/// Categories and patterns must already be canonical (no surrounding whitespace).
#[derive(Clone, Eq, PartialEq)]
pub enum NativeConfiguredPermissionMutation {
    Add { permission: String, pattern: String },
    Remove { permission: String, pattern: String },
    Reset(NativeConfiguredPermissionReset),
}

impl fmt::Debug for NativeConfiguredPermissionMutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeConfiguredPermissionMutation { .. }")
    }
}

/// Pure edit result; only the store's confirmed receipt proves publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConfiguredPermissionMutationOutcome {
    Unchanged,
    Changed { removed_rules: usize },
}

/// Borrowed source observations. A present empty local list shadows user rules.
#[derive(Clone, Copy)]
pub struct NativeConfiguredPermissionSources<'a> {
    user: &'a NativeConfiguredPermissionRules,
    local: Option<&'a NativeConfiguredPermissionRules>,
}

impl fmt::Debug for NativeConfiguredPermissionSources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConfiguredPermissionSources")
            .field("user_shadowed_by_local", &self.local.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a> NativeConfiguredPermissionSources<'a> {
    #[must_use]
    pub const fn user(&self) -> &'a NativeConfiguredPermissionRules {
        self.user
    }
    #[must_use]
    pub const fn local(&self) -> Option<&'a NativeConfiguredPermissionRules> {
        self.local
    }
    #[must_use]
    pub fn effective(&self) -> &'a NativeConfiguredPermissionRules {
        self.local.unwrap_or(self.user)
    }
    #[must_use]
    pub const fn user_shadowed_by_local(&self) -> bool {
        self.local.is_some()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub(super) struct WorkspaceRules {
    workspace_hex: String,
    permission_rules: NativeConfiguredPermissionRules,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireWorkspaceRules {
    workspace_hex: String,
    permission_rules: Vec<WirePermissionRule>,
}

fn invalid() -> NativeConfigError {
    NativeConfigError::new(NativeConfigErrorKind::InvalidFormat)
}

pub(super) fn decode_workspaces(
    wire: Vec<WireWorkspaceRules>,
) -> Result<Vec<WorkspaceRules>, NativeConfigError> {
    let mut identities = BTreeSet::new();
    wire.into_iter()
        .map(|entry| {
            validate_hex(&entry.workspace_hex)?;
            if !identities.insert(entry.workspace_hex.clone()) {
                return Err(invalid());
            }
            Ok(WorkspaceRules {
                workspace_hex: entry.workspace_hex,
                permission_rules: decode_permission_rules(entry.permission_rules)?,
            })
        })
        .collect()
}

fn valid_workspace_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"/")
        && bytes.len() <= MAX_WORKSPACE_BYTES
        && !bytes.contains(&0)
        && (bytes == b"/"
            || bytes[1..]
                .split(|byte| *byte == b'/')
                .all(|part| !part.is_empty() && part != b"." && part != b".."))
}

fn validate_hex(encoded: &str) -> Result<(), NativeConfigError> {
    if encoded.len() > MAX_WORKSPACE_BYTES * 2 || !encoded.len().is_multiple_of(2) {
        return Err(invalid());
    }
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(invalid()),
    };
    let bytes = encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(nibble(pair[0])? * 16 + nibble(pair[1])?))
        .collect::<Result<Vec<_>, NativeConfigError>>()?;
    if valid_workspace_bytes(&bytes) {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn workspace_hex(workspace: &Path) -> Result<String, NativeConfigError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = workspace.as_os_str().as_encoded_bytes();
    if !valid_workspace_bytes(bytes) {
        return Err(invalid());
    }
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(encoded)
}

impl NativeConfig {
    /// Whether any local source exists, including an explicitly empty shadow.
    /// Hosts without permission composition must not silently discard these sources.
    #[must_use]
    pub fn has_workspace_permission_rules(&self) -> bool {
        !self.workspace_permission_rules.is_empty()
    }

    /// Resolves already-normalized absolute workspace bytes without filesystem I/O.
    /// # Errors
    /// Rejects invalid or oversized workspace labels; no CWD fallback is inferred.
    pub fn permission_sources(
        &self,
        workspace: &Path,
    ) -> Result<NativeConfiguredPermissionSources<'_>, NativeConfigError> {
        let key = workspace_hex(workspace)?;
        Ok(NativeConfiguredPermissionSources {
            user: &self.permission_rules,
            local: self
                .workspace_permission_rules
                .iter()
                .find(|entry| entry.workspace_hex == key)
                .map(|entry| &entry.permission_rules),
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn with_permission_mutation(
        &self,
        workspace: &Path,
        scope: NativeConfiguredPermissionScope,
        mutation: &NativeConfiguredPermissionMutation,
    ) -> Result<(Self, NativeConfiguredPermissionMutationOutcome), NativeConfigError> {
        let key = workspace_hex(workspace)?;
        validate_mutation(mutation)?;
        let index = self
            .workspace_permission_rules
            .iter()
            .position(|entry| entry.workspace_hex == key);
        let original = match scope {
            NativeConfiguredPermissionScope::User => Some(&self.permission_rules),
            NativeConfiguredPermissionScope::Local => {
                index.map(|i| &self.workspace_permission_rules[i].permission_rules)
            }
        };
        let mut rules = original.map_or_else(Vec::new, |rules| rules.rules().to_vec());
        let outcome = apply(&mut rules, mutation)?;
        let mut candidate = self.clone();
        if outcome == NativeConfiguredPermissionMutationOutcome::Unchanged {
            return Ok((candidate, outcome));
        }
        let remove_scope =
            matches!(mutation, NativeConfiguredPermissionMutation::Reset(_)) && rules.is_empty();
        let rules = NativeConfiguredPermissionRules::new(rules)
            .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::TooLarge))?;
        match scope {
            NativeConfiguredPermissionScope::User => candidate.permission_rules = rules,
            NativeConfiguredPermissionScope::Local => match index {
                Some(index) if remove_scope => {
                    candidate.workspace_permission_rules.remove(index);
                }
                Some(index) => candidate.workspace_permission_rules[index].permission_rules = rules,
                None => candidate.workspace_permission_rules.push(WorkspaceRules {
                    workspace_hex: key,
                    permission_rules: rules,
                }),
            },
        }
        candidate.schema_version = CONFIG_SCHEMA_VERSION;
        // Validate the complete encoded envelope, not each list independently.
        candidate.serialize_current()?;
        Ok((candidate, outcome))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_mutation(
    mutation: &NativeConfiguredPermissionMutation,
) -> Result<(), NativeConfigError> {
    match mutation {
        NativeConfiguredPermissionMutation::Add {
            permission,
            pattern,
        }
        | NativeConfiguredPermissionMutation::Remove {
            permission,
            pattern,
        } => {
            if permission.len() > MAX_CONFIG_BYTES || pattern.len() > MAX_CONFIG_BYTES {
                return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
            }
            if permission.is_empty()
                || permission.trim_matches([' ', '\t', '\r', '\n']) != permission
                || pattern.trim_matches([' ', '\t', '\r', '\n']) != pattern
            {
                return Err(invalid());
            }
            NativeConfiguredPermissionRule::new(
                permission,
                pattern,
                NativeConfiguredPermissionDecision::Allow,
            )
            .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::TooLarge))?;
        }
        NativeConfiguredPermissionMutation::Reset(_) => {}
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn apply(
    rules: &mut Vec<NativeConfiguredPermissionRule>,
    mutation: &NativeConfiguredPermissionMutation,
) -> Result<NativeConfiguredPermissionMutationOutcome, NativeConfigError> {
    use NativeConfiguredPermissionDecision::Allow;
    use NativeConfiguredPermissionMutation::{Add, Remove, Reset};
    use NativeConfiguredPermissionMutationOutcome::{Changed, Unchanged};
    match mutation {
        Add {
            permission,
            pattern,
        } => {
            let replacement = NativeConfiguredPermissionRule::new(permission, pattern, Allow)
                .map_err(|_| invalid())?;
            if let Some(rule) = rules
                .iter_mut()
                .rev()
                .find(|rule| rule.permission() == permission && rule.pattern() == pattern)
            {
                if rule.decision() == Allow {
                    return Ok(Unchanged);
                }
                *rule = replacement;
            } else {
                rules.push(replacement);
            }
            Ok(Changed { removed_rules: 0 })
        }
        Remove {
            permission,
            pattern,
        } => {
            let before = rules.len();
            rules.retain(|rule| rule.permission() != permission || rule.pattern() != pattern);
            Ok(removed(before - rules.len()))
        }
        Reset(scope) => {
            let before = rules.len();
            rules.retain(|rule| {
                rule.decision() != Allow || !reset_matches(*scope, rule.permission())
            });
            Ok(removed(before - rules.len()))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn removed(count: usize) -> NativeConfiguredPermissionMutationOutcome {
    if count == 0 {
        NativeConfiguredPermissionMutationOutcome::Unchanged
    } else {
        NativeConfiguredPermissionMutationOutcome::Changed {
            removed_rules: count,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reset_matches(scope: NativeConfiguredPermissionReset, permission: &str) -> bool {
    let url = matches!(permission, "url" | "open_url" | "browser_navigate");
    match scope {
        NativeConfiguredPermissionReset::All => true,
        NativeConfiguredPermissionReset::Commands => permission == "bash",
        NativeConfiguredPermissionReset::Urls => url,
        NativeConfiguredPermissionReset::WebFetchDomains => permission == "web_fetch",
        NativeConfiguredPermissionReset::Tools => {
            !url && !matches!(permission, "bash" | "web_fetch" | "*")
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "config_permissions/tests.rs"]
mod tests;
