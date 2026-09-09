//! Read-only configured-pattern discovery, never a live authorization snapshot.

use std::{fmt, path::Path};

use crate::{
    ConfigOrigin, LoadedNativeConfig, NativeConfiguredPermissionDecision,
    NativeConfiguredPermissionRules, NativeConfiguredPermissionScope, PermissionMode,
};

/// A configured row. Inert malformed web-fetch patterns are deliberately omitted.
#[derive(Clone, Eq, PartialEq)]
pub struct NativePermissionInspectionRule {
    permission: String,
    pattern: Option<String>,
    decision: NativeConfiguredPermissionDecision,
}

impl NativePermissionInspectionRule {
    #[must_use]
    pub fn permission(&self) -> &str {
        &self.permission
    }

    #[must_use]
    pub fn pattern(&self) -> Option<&str> {
        self.pattern.as_deref()
    }

    #[must_use]
    pub const fn decision(&self) -> NativeConfiguredPermissionDecision {
        self.decision
    }

    /// Only malformed exact `web_fetch` domain rules are classified as inert.
    /// Other rows are configured patterns, not claims about a live tool registry.
    #[must_use]
    pub const fn inert(&self) -> bool {
        self.pattern.is_none()
    }
}

impl fmt::Debug for NativePermissionInspectionRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionInspectionRule { .. }")
    }
}

/// Bounded projections from one validated configuration; retains no other settings.
/// Saved exact-action rules and runtime grants are not observed by this report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePermissionInspection {
    origin: ConfigOrigin,
    mode: PermissionMode,
    user: Vec<NativePermissionInspectionRule>,
    local: Option<Vec<NativePermissionInspectionRule>>,
}

impl NativePermissionInspection {
    #[must_use]
    pub const fn origin(&self) -> ConfigOrigin {
        self.origin
    }

    #[must_use]
    pub const fn permission_mode(&self) -> PermissionMode {
        self.mode
    }

    #[must_use]
    pub const fn effective_scope(&self) -> NativeConfiguredPermissionScope {
        if self.local.is_some() {
            NativeConfiguredPermissionScope::Local
        } else {
            NativeConfiguredPermissionScope::User
        }
    }

    #[must_use]
    pub fn user_rules(&self) -> &[NativePermissionInspectionRule] {
        &self.user
    }

    /// `Some([])` is an observed empty local shadow, not an absent local source.
    #[must_use]
    pub fn local_rules(&self) -> Option<&[NativePermissionInspectionRule]> {
        self.local.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePermissionInspectionError {
    Configuration,
    Workspace,
}

impl fmt::Display for NativePermissionInspectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native permission inspection failed")
    }
}

impl std::error::Error for NativePermissionInspectionError {}

/// Projects rules for explicitly supplied normalized absolute workspace bytes.
/// No files are opened and no authority is derived from these display rows.
/// # Errors
/// Rejects invalid or oversized workspace labels using native config validation.
pub fn inspect_native_permissions(
    loaded: &LoadedNativeConfig,
    workspace: &Path,
) -> Result<NativePermissionInspection, NativePermissionInspectionError> {
    let sources = loaded
        .config()
        .permission_sources(workspace)
        .map_err(|_| NativePermissionInspectionError::Workspace)?;
    Ok(project(loaded, sources.local()))
}

fn project(
    loaded: &LoadedNativeConfig,
    local: Option<&NativeConfiguredPermissionRules>,
) -> NativePermissionInspection {
    fn rows(rules: &NativeConfiguredPermissionRules) -> Vec<NativePermissionInspectionRule> {
        rules
            .rules()
            .iter()
            .map(|rule| NativePermissionInspectionRule {
                permission: rule.permission().to_owned(),
                pattern: (rule.permission() != "web_fetch"
                    || crate::permission_patterns::canonical_web_domain(rule.pattern()))
                .then(|| rule.pattern().to_owned()),
                decision: rule.decision(),
            })
            .collect()
    }
    NativePermissionInspection {
        origin: loaded.origin(),
        mode: loaded.config().permission_mode(),
        user: rows(loaded.config().permission_rules()),
        local: local.map(rows),
    }
}

/// Loads configuration once, without opening a state store or discovering credentials.
/// When any local sources exist, captures and canonicalizes the current directory
/// once before selecting the exact native workspace source. With no local sources,
/// no workspace observation is needed, preserving defaults even without a usable CWD.
/// # Errors
/// Returns fixed errors for invalid configuration or unavailable workspace selection.
pub fn inspect_process_permissions()
-> Result<NativePermissionInspection, NativePermissionInspectionError> {
    let loaded =
        crate::load_process_config().map_err(|_| NativePermissionInspectionError::Configuration)?;
    inspect_process_loaded(&loaded, || {
        std::env::current_dir().and_then(std::fs::canonicalize)
    })
}

fn inspect_process_loaded(
    loaded: &LoadedNativeConfig,
    workspace: impl FnOnce() -> std::io::Result<std::path::PathBuf>,
) -> Result<NativePermissionInspection, NativePermissionInspectionError> {
    if !loaded.config().has_workspace_permission_rules() {
        return Ok(project(loaded, None));
    }
    let workspace = workspace().map_err(|_| NativePermissionInspectionError::Workspace)?;
    inspect_native_permissions(loaded, &workspace)
}

#[cfg(test)]
mod tests;
