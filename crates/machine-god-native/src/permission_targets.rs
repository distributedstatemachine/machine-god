//! Explicit native target preparation; these observations are not execution grants.

mod paths;
mod terminal;
pub use terminal::{NativePermissionTerminalResolution, NativePermissionTerminalResolver};
#[cfg(any(test, feature = "ai-gateway-http"))]
mod host;
#[cfg(any(test, feature = "ai-gateway-http"))]
pub(crate) use host::HostPermissionResolver;

use crate::tool_output_serializer::{CompactToolOutputLimits, measure_json_value_compact};
use crate::{
    NativeConfiguredPermissionDecision as Decision, NativeConfiguredPermissionRules,
    NativePermissionConfiguredOutcome as Outcome, NativePermissionTargetKind as Kind,
    NativePreparedPermissionTarget, PermissionMode, TerminalActionTool,
};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PermissionError, PermissionInvocation,
    PermissionRequest, TerminalActionRequest, Tool, ToolCall,
};
use serde_json::Value;
use std::{fmt, fs::File, sync::Arc};

/// Explicit registration of the real native implementation used by the host.
/// The terminal variant validates its private prepared host/call envelope.
pub enum NativePermissionTargetTool {
    Ordinary(Arc<dyn Tool>),
    Question(Arc<crate::AskUserQuestionTool>),
    Grep(Arc<crate::GrepFilesTool>),
    Terminal(Arc<TerminalActionTool>),
    TerminalWithResolver {
        tool: Arc<TerminalActionTool>,
        resolver: Arc<dyn NativePermissionTerminalResolver>,
    },
}

impl fmt::Debug for NativePermissionTargetTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionTargetTool { .. }")
    }
}

/// Retained workspace authority. Construction performs no filesystem effects.
pub struct NativePermissionTargetAuthority {
    root: Arc<File>,
    workspace: String,
    tools: Vec<NativePermissionTargetTool>,
}
impl fmt::Debug for NativePermissionTargetAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionTargetAuthority { .. }")
    }
}

impl NativePermissionTargetAuthority {
    pub(crate) fn validate_file_authority(
        &self,
        files: &crate::NativeFileApprovalAuthority,
    ) -> Result<(), PermissionError> {
        paths::validate_root(&self.root, &self.workspace)?;
        paths::validate_root(files.directory(), &self.workspace)
    }

    /// Projects existing file evidence without duplicating preimages or I/O.
    /// The caller retains the approval and its same-workspace execution binding.
    /// # Errors
    /// Rejects mismatched names and bounded argument/path failures.
    pub fn from_file(
        &self,
        file: &crate::PreparedFileApproval,
        invocation: PermissionInvocation<'_>,
        cancellation: &CancellationToken,
    ) -> Result<NativePreparedPermissionTargets, PermissionError> {
        check_cancel(cancellation)?;
        if file.tool_name() != invocation.tool_name.as_str() {
            return Err(invalid());
        }
        let mut result = NativePreparedPermissionTargets {
            root: Arc::clone(&self.root),
            workspace: self.workspace.clone(),
            tool_name: file.tool_name().to_owned(),
            arguments_json: bounded_arguments(invocation.arguments, false, cancellation)?,
            targets: Vec::new(),
            observations: Vec::new(),
            bypass: Bypass::Never,
            terminal: None,
        };
        let target = paths::absolute(&self.workspace, file.target_path())?;
        match file.kind() {
            crate::NativeFileApprovalKind::Write | crate::NativeFileApprovalKind::Edit => {
                let parent = target
                    .rsplit_once('/')
                    .map_or(
                        "/",
                        |(parent, _)| if parent.is_empty() { "/" } else { parent },
                    )
                    .to_owned();
                result.add("target", target, Kind::PathExisting);
                result.add("parent", parent, Kind::PathExisting);
            }
            crate::NativeFileApprovalKind::Delete => {
                result.add("target", target, Kind::PathExisting);
            }
            crate::NativeFileApprovalKind::Copy | crate::NativeFileApprovalKind::Rename => {
                let source =
                    paths::absolute(&self.workspace, file.source_path().ok_or_else(invalid)?)?;
                result.add("source", source, Kind::None);
                result.add("destination", target, Kind::None);
            }
        }
        Ok(result)
    }

    /// # Errors
    /// Rejects noncanonical workspace spellings or an oversized trusted registry.
    pub fn new(
        root: File,
        workspace: String,
        tools: Vec<NativePermissionTargetTool>,
    ) -> Result<Self, PermissionError> {
        paths::validate_workspace(&workspace)?;
        if tools.len() > 64 {
            return Err(invalid());
        }
        Ok(Self {
            root: Arc::new(root),
            workspace,
            tools,
        })
    }

    /// Validates actual canonical invocation/capability agreement before observing
    /// paths. Unknown tools require a separate explicitly trusted dynamic adapter.
    #[must_use]
    pub fn prepare<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<NativePreparedPermissionTargets, PermissionError>> {
        Box::pin(async move {
            check_cancel(&cancellation)?;
            let name = invocation.tool_name.as_str();
            if !known_builtin(name) {
                return Err(invalid());
            }
            let arguments_json =
                bounded_arguments(invocation.arguments, name == "terminal", &cancellation)?;
            let entry = self.registered_tool(name)?;
            let terminal = match entry {
                NativePermissionTargetTool::Ordinary(tool) => {
                    validate_ordinary(tool.as_ref(), request, invocation)?;
                    None
                }
                NativePermissionTargetTool::Question(tool) => {
                    tool.validate_permission_preparation(request, invocation)
                        .map_err(|_| invalid())?;
                    None
                }
                NativePermissionTargetTool::Grep(tool) => {
                    tool.validate_permission_preparation(request, invocation)
                        .map_err(|_| invalid())?;
                    None
                }
                NativePermissionTargetTool::Terminal(tool)
                | NativePermissionTargetTool::TerminalWithResolver { tool, .. } => {
                    if tool.permission_workspace() != self.workspace {
                        return Err(invalid());
                    }
                    Some(
                        tool.validate_permission_preparation(request, invocation)
                            .map_err(|_| invalid())?,
                    )
                }
            };
            check_cancel(&cancellation)?;
            paths::validate_root(&self.root, &self.workspace)?;
            let mut prepared = NativePreparedPermissionTargets {
                root: Arc::clone(&self.root),
                workspace: self.workspace.clone(),
                tool_name: name.to_owned(),
                arguments_json,
                targets: Vec::new(),
                observations: Vec::new(),
                bypass: Bypass::Never,
                terminal: None,
            };
            if let Some(terminal) = terminal {
                let resolution = match entry {
                    NativePermissionTargetTool::TerminalWithResolver { resolver, .. } => {
                        resolver.resolve(terminal, cancellation.clone()).await?
                    }
                    NativePermissionTargetTool::Terminal(tool) => {
                        if let Some(resolver) = tool.permission_resolver() {
                            resolver.resolve(terminal, cancellation.clone()).await?
                        } else {
                            if terminal.has_workspace_filter() {
                                return Err(invalid());
                            }
                            let action = terminal
                                .resolve_cwd(|_| {
                                    Err(machine_god_core::ToolError::new(
                                        machine_god_core::ToolErrorKind::InvalidInput,
                                        "permission_target_invalid",
                                        "permission target preparation failed",
                                        false,
                                    ))
                                })
                                .map_err(|_| invalid())?;
                            let identity = tool.permission_host_identity();
                            NativePermissionTerminalResolution::new(
                                action,
                                None,
                                None,
                                identity.environment_sha256.clone(),
                                identity.shell_selection_sha256.clone(),
                            )?
                        }
                    }
                    NativePermissionTargetTool::Ordinary(_)
                    | NativePermissionTargetTool::Question(_)
                    | NativePermissionTargetTool::Grep(_) => return Err(invalid()),
                };
                prepared.prepare_terminal(resolution);
            } else {
                prepared.prepare_ordinary(request, invocation.arguments)?;
            }
            check_cancel(&cancellation)?;
            Ok(prepared)
        })
    }

    fn registered_tool(&self, name: &str) -> Result<&NativePermissionTargetTool, PermissionError> {
        let mut registered = self.tools.iter().filter(|entry| match entry {
            NativePermissionTargetTool::Ordinary(tool) => tool.spec().name.as_str() == name,
            NativePermissionTargetTool::Question(_) => name == "ask_user_question",
            NativePermissionTargetTool::Grep(_) => name == "grep_files",
            NativePermissionTargetTool::Terminal(_)
            | NativePermissionTargetTool::TerminalWithResolver { .. } => name == "terminal",
        });
        let entry = registered.next().ok_or_else(invalid)?;
        if registered.next().is_some() {
            return Err(invalid());
        }
        Ok(entry)
    }
}

/// One owned, canonical policy target with its pinned presentation kind.
pub struct NativePermissionOwnedTarget {
    role: &'static str,
    path: String,
    kind: Kind,
}
impl NativePermissionOwnedTarget {
    #[must_use]
    pub const fn role(&self) -> &'static str {
        self.role
    }
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }
}
impl fmt::Debug for NativePermissionOwnedTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionOwnedTarget { .. }")
    }
}

#[derive(Clone, Copy)]
enum Bypass {
    Never,
    Always,
    Auto,
}

/// Bounded preparation evidence retained across policy evaluation and prompting.
/// Revalidation is an admission check, not an atomic check-and-native-effect operation.
pub struct NativePreparedPermissionTargets {
    root: Arc<File>,
    workspace: String,
    tool_name: String,
    arguments_json: String,
    targets: Vec<NativePermissionOwnedTarget>,
    observations: Vec<paths::Observation>,
    bypass: Bypass,
    terminal: Option<NativePermissionTerminalResolution>,
}
impl fmt::Debug for NativePreparedPermissionTargets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePreparedPermissionTargets { .. }")
    }
}
impl NativePreparedPermissionTargets {
    /// Semantic arguments excluding terminal call IDs and private host stamps.
    /// # Errors
    /// Rejects an encoding failure. The typed action was bounded during preparation.
    pub fn identity_arguments_json(&self) -> Result<String, PermissionError> {
        self.terminal.as_ref().map_or_else(
            || Ok(self.arguments_json.clone()),
            |resolution| serde_json::to_string(resolution.action()).map_err(|_| invalid()),
        )
    }
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }
    #[must_use]
    pub fn workspace(&self) -> &str {
        &self.workspace
    }
    #[must_use]
    pub fn arguments_json(&self) -> &str {
        &self.arguments_json
    }
    #[must_use]
    pub fn targets(&self) -> &[NativePermissionOwnedTarget] {
        &self.targets
    }
    /// Validated actual terminal action with a descriptor-resolved cwd. A command
    /// still needs the host's shell/environment selection and final execution binding.
    #[must_use]
    pub fn terminal_action(&self) -> Option<&TerminalActionRequest> {
        self.terminal
            .as_ref()
            .map(NativePermissionTerminalResolution::action)
    }
    #[must_use]
    pub const fn terminal_resolution(&self) -> Option<&NativePermissionTerminalResolution> {
        self.terminal.as_ref()
    }
    #[must_use]
    pub fn allows_without_review(&self, mode: PermissionMode) -> bool {
        matches!(self.bypass, Bypass::Always)
            || matches!((self.bypass, mode), (Bypass::Auto, PermissionMode::Auto))
    }
    /// # Errors
    /// Rejects malformed targets or exhausted matching work; no partial allow.
    pub fn configured_outcome(
        &self,
        rules: &NativeConfiguredPermissionRules,
    ) -> Result<Outcome, PermissionError> {
        let tool = if matches!(
            self.terminal_action(),
            Some(TerminalActionRequest::Exec { .. })
        ) {
            "run_command"
        } else {
            &self.tool_name
        };
        let mut ask = false;
        let mut unresolved = false;
        for target in &self.targets {
            let borrowed = NativePreparedPermissionTarget::new(
                &self.workspace,
                tool,
                &target.path,
                target.kind,
            )
            .map_err(|_| invalid())?;
            match rules.decide(&borrowed).map_err(|_| invalid())? {
                Some(Decision::Deny) => return Ok(Outcome::Deny),
                Some(Decision::Ask) => ask = true,
                Some(Decision::Allow) => {}
                None => unresolved = true,
            }
        }
        Ok(if ask {
            Outcome::Ask
        } else if unresolved || self.targets.is_empty() {
            Outcome::Unresolved
        } else {
            Outcome::Allow
        })
    }
    /// # Errors
    /// Rejects root, parent, selected-entry replacement, or missing-entry changes.
    pub fn revalidate(&self) -> Result<(), PermissionError> {
        paths::validate_root(&self.root, &self.workspace)?;
        for observed in &self.observations {
            observed.revalidate(&self.root)?;
        }
        Ok(())
    }
    fn add(&mut self, role: &'static str, path: String, kind: Kind) {
        self.targets
            .push(NativePermissionOwnedTarget { role, path, kind });
    }
    fn path(&mut self, raw: &str, create: bool) -> Result<String, PermissionError> {
        let observation = paths::observe(&self.root, raw, create, self.tool_name == "file_info")?;
        let canonical = if raw == "." {
            self.workspace.clone()
        } else {
            format!("{}/{raw}", self.workspace.trim_end_matches('/'))
        };
        self.observations.push(observation);
        Ok(canonical)
    }
    fn prepare_terminal(&mut self, resolution: NativePermissionTerminalResolution) {
        match resolution.action() {
            TerminalActionRequest::Exec { request } => {
                // Configured matching intentionally sees command text; exact
                // shell/grant identity belongs to native process preparation.
                self.add(
                    "target",
                    format!("{}::{}", request.cwd, request.command),
                    Kind::CommandCwd,
                );
            }
            TerminalActionRequest::Read { .. }
            | TerminalActionRequest::Screen { .. }
            | TerminalActionRequest::List { .. } => self.bypass = Bypass::Auto,
            TerminalActionRequest::Inspect { events, .. }
                if events.acknowledge_event_id.is_none() =>
            {
                self.bypass = Bypass::Auto;
            }
            _ => {}
        }
        if self.targets.is_empty() {
            self.add("target", "terminal".to_owned(), Kind::None);
        }
        self.terminal = Some(resolution);
    }
    fn prepare_ordinary(
        &mut self,
        request: &PermissionRequest,
        arguments: &Value,
    ) -> Result<(), PermissionError> {
        let name = self.tool_name.clone();
        match name.as_str() {
            "read_file" | "file_info" | "open_file" | "list_files" | "glob_files"
            | "grep_files" | "semantic_search" => {
                let optional = matches!(
                    name.as_str(),
                    "list_files" | "glob_files" | "grep_files" | "semantic_search"
                );
                let raw = if optional {
                    arguments.get("path").and_then(Value::as_str).unwrap_or(".")
                } else {
                    string(arguments, "path")?
                };
                let path = self.path(raw, false)?;
                self.add(
                    "target",
                    path,
                    if optional {
                        Kind::PathOptionalExisting
                    } else {
                        Kind::PathExisting
                    },
                );
                if name != "open_file" {
                    self.bypass = Bypass::Always;
                }
            }
            "create_folder" => {
                let path = self.path(string(arguments, "path")?, true)?;
                let parent = path
                    .rsplit_once('/')
                    .map_or(
                        "/",
                        |(parent, _)| if parent.is_empty() { "/" } else { parent },
                    )
                    .to_owned();
                if !sensitive_path(&path) {
                    self.bypass = Bypass::Auto;
                }
                self.add("target", path, Kind::PathCreateParent);
                self.add("parent", parent, Kind::PathCreateParent);
            }
            "web_fetch" => {
                let Capability::Network { target } = &request.capability else {
                    return Err(invalid());
                };
                self.add("target", format!("domain:{}", target.host), Kind::None);
                self.bypass = Bypass::Always;
            }
            "skill" => {
                self.add("target", string(arguments, "name")?.to_owned(), Kind::None);
                self.bypass = Bypass::Always;
            }
            "install_skill" => {
                let source = string(arguments, "source")?;
                let target = arguments
                    .get("skill")
                    .and_then(Value::as_str)
                    .map_or_else(|| source.to_owned(), |skill| format!("{source}#{skill}"));
                self.add("target", target, Kind::None);
            }
            "vision" => {
                if let Some(paths) = arguments.get("paths").and_then(Value::as_array) {
                    for raw in paths {
                        let path = self.path(raw.as_str().ok_or_else(invalid)?, false)?;
                        if !self
                            .observations
                            .last()
                            .is_some_and(paths::Observation::is_regular)
                            || self.targets.iter().any(|target| target.path == path)
                        {
                            return Err(invalid());
                        }
                        self.add("image", path, Kind::None);
                    }
                } else {
                    self.add("target", name, Kind::None);
                }
                self.bypass = Bypass::Auto;
            }
            "terminal" => return Err(invalid()),
            _ => {
                self.add("target", name, Kind::None);
                self.bypass = Bypass::Always;
            }
        }
        Ok(())
    }
}

fn validate_ordinary(
    tool: &dyn Tool,
    request: &PermissionRequest,
    invocation: PermissionInvocation<'_>,
) -> Result<(), PermissionError> {
    let prepared = tool
        .prepare(ToolCall {
            id: invocation.call_id.clone(),
            name: invocation.tool_name.clone(),
            arguments: invocation.arguments.clone(),
        })
        .map_err(|_| invalid())?;
    if prepared.arguments() != invocation.arguments {
        return Err(invalid());
    }
    if let Some(capability) = prepared.capability() {
        if capability != &request.capability {
            return Err(invalid());
        }
    } else if !matches!(&request.capability, Capability::Tool { name, call_id, arguments }
        if name == invocation.tool_name && call_id == invocation.call_id && arguments == invocation.arguments)
    {
        return Err(invalid());
    }
    Ok(())
}
fn known_builtin(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_files"
            | "glob_files"
            | "grep_files"
            | "file_info"
            | "open_file"
            | "create_folder"
            | "semantic_search"
            | "web_fetch"
            | "web_search"
            | "memory"
            | "skill"
            | "install_skill"
            | "vision"
            | "terminal"
            | "read_tool_result"
            | "ask_user_question"
            | "subagent"
            | "mcp_search_tools"
            | "mcp_select_tool"
            | "mcp_features"
    )
}
fn bounded_arguments(
    value: &Value,
    terminal: bool,
    cancellation: &CancellationToken,
) -> Result<String, PermissionError> {
    let (output_bytes, json_nodes) = if terminal {
        (
            crate::MAX_TERMINAL_PREPARED_ARGUMENT_BYTES,
            crate::MAX_TERMINAL_ACTION_ARGUMENT_NODES + 128,
        )
    } else {
        (1024 * 1024, 65_536)
    };
    measure_json_value_compact(
        value,
        CompactToolOutputLimits {
            output_bytes,
            json_nodes,
            json_depth: machine_god_core::MAX_SAFE_JSON_DEPTH,
        },
        cancellation,
    )
    .map_err(|_| invalid())?;
    serde_json::to_string(value).map_err(|_| invalid())
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, PermissionError> {
    value.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn check_cancel(cancellation: &CancellationToken) -> Result<(), PermissionError> {
    if cancellation.is_cancelled() {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> PermissionError {
    PermissionError::new(
        "permission_target_invalid",
        "permission target preparation failed",
    )
}

pub(crate) fn sensitive_path(path: &str) -> bool {
    const PARTS: &[&str] = &[
        ".git/hooks",
        ".git/config",
        ".git/config.worktree",
        ".ssh/authorized_keys",
        ".ssh/config",
        "Library/LaunchAgents",
        "Library/LaunchDaemons",
        ".config/autostart",
        ".config/fish/config.fish",
        ".zshrc",
        ".bashrc",
        ".bash_profile",
        ".profile",
    ];
    PARTS.iter().any(|part| {
        path.split('/').enumerate().any(|(index, _)| {
            let mut actual = path.split('/').skip(index);
            part.split('/')
                .all(|expected| actual.next() == Some(expected))
        })
    })
}
