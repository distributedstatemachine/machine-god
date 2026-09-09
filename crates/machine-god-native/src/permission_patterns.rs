//! Pure configured-pattern evaluation over explicitly prepared target strings.

use std::borrow::Cow;
use std::fmt;
use std::io::{self, Write};

use machine_god_core::ToolName;
use serde::Serialize;

use crate::{MAX_CONFIG_BYTES, MAX_TERMINAL_COMMAND_BYTES, MAX_TERMINAL_CWD_BYTES};

/// Covers a maximum command, cwd, shell path, and environment-identity framing.
pub const MAX_CONFIGURED_PERMISSION_TARGET_BYTES: usize =
    MAX_TERMINAL_COMMAND_BYTES + 2 * MAX_TERMINAL_CWD_BYTES + 128;
/// Shared matching-work budget for one complete target evaluation.
pub const MAX_CONFIGURED_PERMISSION_MATCH_STEPS: usize = 4 * 1024 * 1024;

/// A configured decision; absence of a matching rule is a separate outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeConfiguredPermissionDecision {
    Allow,
    Ask,
    Deny,
}

/// Fixed failures never retain a rule, target, command, or environment identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConfiguredPermissionError {
    InvalidPermission,
    InvalidTarget,
    Limit,
}

impl fmt::Display for NativeConfiguredPermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidPermission => "configured permission category is invalid",
            Self::InvalidTarget => "prepared permission target is invalid",
            Self::Limit => "configured permission evaluation limit exceeded",
        })
    }
}

impl std::error::Error for NativeConfiguredPermissionError {}

type Error = NativeConfiguredPermissionError;

/// One ordered configured pattern, not a saved exact-action rule or grant.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeConfiguredPermissionRule {
    permission: String,
    pattern: String,
    action: NativeConfiguredPermissionDecision,
}

impl NativeConfiguredPermissionRule {
    /// Trims only ASCII space, tab, CR and LF from the category and pattern,
    /// matching pinned configuration keys. An empty pattern remains meaningful.
    /// Invalid web-fetch domain patterns are retained but never match that tool.
    /// # Errors
    /// Rejects empty permission categories and encoded values above the config bound.
    pub fn new(
        permission: &str,
        pattern: &str,
        decision: NativeConfiguredPermissionDecision,
    ) -> Result<Self, Error> {
        #[derive(Serialize)]
        struct View<'a> {
            permission: &'a str,
            pattern: &'a str,
            action: NativeConfiguredPermissionDecision,
        }
        if permission.len() > MAX_CONFIG_BYTES || pattern.len() > MAX_CONFIG_BYTES {
            return Err(Error::Limit);
        }
        let permission = permission.trim_matches([' ', '\t', '\r', '\n']);
        let pattern = pattern.trim_matches([' ', '\t', '\r', '\n']);
        if permission.is_empty() {
            return Err(Error::InvalidPermission);
        }
        check_encoded_bound(&View {
            permission,
            pattern,
            action: decision,
        })?;
        Ok(Self {
            permission: permission.to_owned(),
            pattern: pattern.to_owned(),
            action: decision,
        })
    }

    #[must_use]
    pub fn permission(&self) -> &str {
        &self.permission
    }
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
    #[must_use]
    pub const fn decision(&self) -> NativeConfiguredPermissionDecision {
        self.action
    }
}

impl fmt::Debug for NativeConfiguredPermissionRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConfiguredPermissionRule")
            .field("decision", &self.action)
            .finish_non_exhaustive()
    }
}

/// Ordered rules bounded by the fully encoded JSON array size (64 KiB).
/// This value does not read configuration or participate in authorization itself.
#[derive(Clone, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct NativeConfiguredPermissionRules {
    rules: Vec<NativeConfiguredPermissionRule>,
}

impl NativeConfiguredPermissionRules {
    /// # Errors
    /// Rejects a complete encoded array above `MAX_CONFIG_BYTES`, including
    /// object fields, escaping and list overhead. No saved-rule count cap applies.
    pub fn new(rules: Vec<NativeConfiguredPermissionRule>) -> Result<Self, Error> {
        check_encoded_bound(&rules)?;
        Ok(Self { rules })
    }

    #[must_use]
    pub fn rules(&self) -> &[NativeConfiguredPermissionRule] {
        &self.rules
    }

    /// Evaluates all rules in order; the last matching rule wins for this target.
    /// The caller owns multi-target policy and all target preparation/authority.
    /// # Errors
    /// Exhausting the shared matcher budget rejects the entire evaluation; no
    /// earlier partial decision is returned.
    pub fn decide(
        &self,
        target: &NativePreparedPermissionTarget<'_>,
    ) -> Result<Option<NativeConfiguredPermissionDecision>, Error> {
        let permission = permission_name(target.tool_name);
        if permission == "web_fetch" {
            if !canonical_web_domain(target.target) {
                return Ok(None);
            }
            return Ok(self
                .rules
                .iter()
                .rev()
                .find(|rule| {
                    rule.permission == "web_fetch"
                        && canonical_web_domain(&rule.pattern)
                        && rule.pattern == target.target
                })
                .map(|rule| rule.action));
        }
        let candidate = target.pattern();
        let mut remaining = MAX_CONFIGURED_PERMISSION_MATCH_STEPS;
        let mut decision = None;
        for rule in &self.rules {
            if (wildcard_match(
                rule.permission.as_bytes(),
                permission.as_bytes(),
                &mut remaining,
            )? || wildcard_match(
                rule.permission.as_bytes(),
                target.tool_name.as_bytes(),
                &mut remaining,
            )?) && target_matches(rule.pattern.as_bytes(), &candidate, &mut remaining)?
            {
                decision = Some(rule.action);
            }
        }
        Ok(decision)
    }

    /// Counts inert noncanonical domain rules for the exact web-fetch category.
    #[must_use]
    pub fn web_fetch_warning_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|rule| rule.permission == "web_fetch" && !canonical_web_domain(&rule.pattern))
            .count()
    }
}

impl fmt::Debug for NativeConfiguredPermissionRules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConfiguredPermissionRules")
            .field("rule_count", &self.rules.len())
            .finish_non_exhaustive()
    }
}

/// Pinned target presentation kind, supplied by trusted native preparation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePermissionTargetKind {
    None,
    PathExisting,
    PathOptionalExisting,
    PathCreateParent,
    PathExistingParent,
    CommandCwd,
    Url,
}

/// Borrowed prepared target. Construction validates bounds, not filesystem
/// identity, canonicality, tool semantics, workspace membership, or authority.
#[derive(Clone, Copy)]
pub struct NativePreparedPermissionTarget<'a> {
    workspace_root: &'a str,
    tool_name: &'a str,
    target: &'a str,
    kind: NativePermissionTargetKind,
}

impl<'a> NativePreparedPermissionTarget<'a> {
    /// # Errors
    /// Rejects invalid core tool names, NUL-bearing prepared strings, workspace
    /// roots above the native 4-KiB cwd limit, or targets above the framed command limit.
    pub fn new(
        workspace_root: &'a str,
        tool_name: &'a str,
        target: &'a str,
        kind: NativePermissionTargetKind,
    ) -> Result<Self, Error> {
        if workspace_root.len() > MAX_TERMINAL_CWD_BYTES
            || target.len() > MAX_CONFIGURED_PERMISSION_TARGET_BYTES
        {
            return Err(Error::Limit);
        }
        if workspace_root.contains('\0')
            || target.contains('\0')
            || ToolName::validate(tool_name).is_err()
        {
            return Err(Error::InvalidTarget);
        }
        Ok(Self {
            workspace_root,
            tool_name,
            target,
            kind,
        })
    }

    #[must_use]
    pub const fn workspace_root(&self) -> &str {
        self.workspace_root
    }
    #[must_use]
    pub const fn tool_name(&self) -> &str {
        self.tool_name
    }
    #[must_use]
    pub const fn target(&self) -> &str {
        self.target
    }
    #[must_use]
    pub const fn kind(&self) -> NativePermissionTargetKind {
        self.kind
    }

    fn pattern(&self) -> Cow<'a, [u8]> {
        let raw = self.target.as_bytes();
        match permission_name(self.tool_name) {
            "bash" => {
                let identity = if raw.starts_with(ENV_PREFIX) {
                    raw
                } else {
                    after_separator(raw).unwrap_or(raw)
                };
                return Cow::Borrowed(command_from_identity(identity));
            }
            "sandbox" => return Cow::Borrowed(command_from_identity(raw)),
            _ => {}
        }
        if matches!(self.tool_name, "copy_file" | "rename_file")
            || matches!(
                self.kind,
                NativePermissionTargetKind::PathExisting
                    | NativePermissionTargetKind::PathOptionalExisting
                    | NativePermissionTargetKind::PathCreateParent
                    | NativePermissionTargetKind::PathExistingParent
            )
        {
            return Cow::Borrowed(display_path(self.workspace_root, self.target).as_bytes());
        }
        if self.kind == NativePermissionTargetKind::CommandCwd
            && let Some(separator) = raw.windows(2).position(|bytes| bytes == b"::")
        {
            let cwd = &self.target[..separator];
            let command = command_from_identity(&raw[separator + 2..]);
            let cwd = if cwd == self.workspace_root {
                b"."
            } else {
                display_path(self.workspace_root, cwd).as_bytes()
            };
            let mut result = Vec::with_capacity(cwd.len() + 2 + command.len());
            result.extend_from_slice(cwd);
            result.extend_from_slice(b"::");
            result.extend_from_slice(command);
            return Cow::Owned(result);
        }
        Cow::Borrowed(raw)
    }
}

impl fmt::Debug for NativePreparedPermissionTarget<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePreparedPermissionTarget")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

fn permission_name(tool: &str) -> &str {
    match tool {
        "read_file" => "read",
        "write_file" | "edit_file" => "edit",
        "list_files" => "list",
        "glob_files" => "glob",
        "grep_files" => "grep",
        "run_command" => "bash",
        "install_skill" => "skill",
        _ => tool,
    }
}

fn display_path<'a>(root: &str, target: &'a str) -> &'a str {
    if !target.starts_with('/') || root.is_empty() {
        return target;
    }
    if target == root {
        return ".";
    }
    if let Some(rest) = target.strip_prefix(root) {
        if root.ends_with('/') {
            return rest;
        }
        if let Some(relative) = rest.strip_prefix('/') {
            return if relative.is_empty() { "." } else { relative };
        }
    }
    target
}

const ENV_PREFIX: &[u8] = b"@fx-terminal-env:";

fn after_separator(bytes: &[u8]) -> Option<&[u8]> {
    bytes
        .windows(2)
        .position(|pair| pair == b"::")
        .map(|index| &bytes[index + 2..])
}

fn command_from_identity(identity: &[u8]) -> &[u8] {
    let Some(rest) = identity.strip_prefix(ENV_PREFIX) else {
        return identity;
    };
    let Some(profile_end) = rest.iter().position(|byte| *byte == b':') else {
        return identity;
    };
    let rest = &rest[profile_end + 1..];
    let Some(length_end) = rest.iter().position(|byte| *byte == b':') else {
        return identity;
    };
    let Some(shell_len) = parse_length(&rest[..length_end]) else {
        return identity;
    };
    let rest = &rest[length_end + 1..];
    let Some(after_shell) = rest.get(shell_len..) else {
        return identity;
    };
    after_shell.strip_prefix(b"::").unwrap_or(identity)
}

fn parse_length(raw: &[u8]) -> Option<usize> {
    let (negative, raw) = match raw.first() {
        Some(b'+') => (false, &raw[1..]),
        Some(b'-') => (true, &raw[1..]),
        _ => (false, raw),
    };
    if raw.is_empty() || raw.first() == Some(&b'_') || raw.last() == Some(&b'_') {
        return None;
    }
    let length = raw.iter().try_fold(0usize, |length, digit| {
        if *digit == b'_' {
            return Some(length);
        }
        digit.is_ascii_digit().then_some(())?;
        length
            .checked_mul(10)?
            .checked_add(usize::from(digit - b'0'))
    })?;
    (!negative || length == 0).then_some(length)
}

pub(crate) fn canonical_web_domain(pattern: &str) -> bool {
    let Some(host) = pattern.strip_prefix("domain:") else {
        return false;
    };
    if host.is_empty() || host.ends_with('.') || host.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return false;
    }
    if host.starts_with('[') {
        return host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .is_some_and(|inner| inner.parse::<std::net::Ipv6Addr>().is_ok());
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn spend(remaining: &mut usize, steps: usize) -> Result<(), Error> {
    *remaining = remaining.checked_sub(steps).ok_or(Error::Limit)?;
    Ok(())
}

fn target_matches(pattern: &[u8], candidate: &[u8], remaining: &mut usize) -> Result<bool, Error> {
    if let Some(root) = pattern.strip_suffix(b"/**") {
        spend(remaining, root.len().min(candidate.len()) + 1)?;
        if root.is_empty() {
            return Ok(candidate.starts_with(b"/"));
        }
        if candidate == root
            || candidate
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with(b"/"))
        {
            return Ok(true);
        }
    }
    wildcard_match(pattern, candidate, remaining)
}

// Greedy star backtracking needs constant space, no recursion, and at most the
// explicit shared comparison budget even for repetitive adversarial patterns.
fn wildcard_match(pattern: &[u8], candidate: &[u8], remaining: &mut usize) -> Result<bool, Error> {
    let (mut p, mut c) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while c < candidate.len() {
        spend(remaining, 1)?;
        if p < pattern.len()
            && pattern[p] != b'*'
            && (pattern[p] == b'?' || pattern[p] == candidate[c])
        {
            p += 1;
            c += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = c;
        } else if let Some(previous) = star {
            retry += 1;
            c = retry;
            p = previous + 1;
        } else {
            return Ok(false);
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        spend(remaining, 1)?;
        p += 1;
    }
    Ok(p == pattern.len())
}

fn check_encoded_bound(value: &impl Serialize) -> Result<(), Error> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|size| *size <= MAX_CONFIG_BYTES)
                .ok_or_else(|| io::Error::other("configured permission size exceeded"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(0), value).map_err(|_| Error::Limit)
}
