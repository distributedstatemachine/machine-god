//! Explicit human configuration edits, distinct from saved exact-action grants.

use std::fmt;

use crate::{
    NativeConfiguredPermissionMutation, NativeConfiguredPermissionMutationOutcome,
    NativeConfiguredPermissionRules, NativeConfiguredPermissionScope, NativeUserConfigError,
};

mod grammar;
pub(crate) mod service;

/// Bounded native slash input, including quoting and command words.
pub const MAX_NATIVE_ALLOWLIST_REQUEST_BYTES: usize = crate::MAX_CONFIG_BYTES + 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAllowlistView {
    Effective,
    Local,
    User,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeAllowlistCommand {
    View(NativeAllowlistView),
    Mutate {
        scope: NativeConfiguredPermissionScope,
        mutation: NativeConfiguredPermissionMutation,
    },
}

/// Constructed only by native parsing against an actual host registry.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeAllowlistRequest {
    command: NativeAllowlistCommand,
    tool: Option<String>,
}
impl NativeAllowlistRequest {
    #[must_use]
    pub const fn command(&self) -> &NativeAllowlistCommand {
        &self.command
    }

    pub(crate) fn validate_registry(
        &self,
        registered: impl Fn(&str) -> bool,
    ) -> Result<(), NativeAllowlistParseError> {
        if self
            .tool
            .as_deref()
            .is_some_and(|tool| !grammar::known_tool(tool, &registered))
        {
            Err(NativeAllowlistParseError::Invalid)
        } else {
            Ok(())
        }
    }
}
impl fmt::Debug for NativeAllowlistRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAllowlistRequest { .. }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAllowlistParseError {
    Invalid,
    Limit,
}
impl fmt::Display for NativeAllowlistParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid native allowlist request")
    }
}
impl std::error::Error for NativeAllowlistParseError {}

/// Only the bounded rule lists are retained, never config credentials/settings.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeAllowlistSources {
    user: NativeConfiguredPermissionRules,
    local: Option<NativeConfiguredPermissionRules>,
}
impl NativeAllowlistSources {
    #[must_use]
    pub const fn user(&self) -> &NativeConfiguredPermissionRules {
        &self.user
    }
    #[must_use]
    pub const fn local(&self) -> Option<&NativeConfiguredPermissionRules> {
        self.local.as_ref()
    }
    #[must_use]
    pub fn effective(&self) -> &NativeConfiguredPermissionRules {
        self.local.as_ref().unwrap_or(&self.user)
    }
    #[must_use]
    pub const fn user_shadowed_by_local(&self) -> bool {
        self.local.is_some()
    }

    /// Pinned display omits ask/deny and inert malformed web-fetch rules.
    pub fn display_rules(
        &self,
        view: NativeAllowlistView,
    ) -> impl Iterator<Item = &crate::NativeConfiguredPermissionRule> {
        let rules = match view {
            NativeAllowlistView::Effective => Some(self.effective()),
            NativeAllowlistView::Local => self.local(),
            NativeAllowlistView::User => Some(self.user()),
        };
        rules
            .into_iter()
            .flat_map(|rules| rules.rules())
            .filter(|rule| {
                rule.decision() == crate::NativeConfiguredPermissionDecision::Allow
                    && (rule.permission() != "web_fetch"
                        || crate::permission_patterns::canonical_web_domain(rule.pattern()))
            })
    }
}
impl fmt::Debug for NativeAllowlistSources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAllowlistSources { .. }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAllowlistReloadError {
    Config(NativeUserConfigError),
    Permission,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAllowlistError {
    Config(NativeUserConfigError),
    Unavailable,
    Ambiguous,
}
impl fmt::Display for NativeAllowlistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native allowlist operation failed")
    }
}
impl std::error::Error for NativeAllowlistError {}

pub enum NativeAllowlistReceipt {
    View {
        view: NativeAllowlistView,
        sources: NativeAllowlistSources,
        reload: Result<(), NativeAllowlistReloadError>,
    },
    Mutation {
        scope: NativeConfiguredPermissionScope,
        mutation: NativeConfiguredPermissionMutation,
        outcome: NativeConfiguredPermissionMutationOutcome,
        sources: Option<NativeAllowlistSources>,
        reload: Option<Result<(), NativeAllowlistReloadError>>,
    },
}
impl NativeAllowlistReceipt {
    pub(crate) fn failed(&self) -> bool {
        match self {
            Self::View { reload, .. } => reload.is_err(),
            Self::Mutation { reload, .. } => reload.as_ref().is_some_and(Result::is_err),
        }
    }
}
impl fmt::Debug for NativeAllowlistReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAllowlistReceipt { .. }")
    }
}

pub(crate) use grammar::parse;
