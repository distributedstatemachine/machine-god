use crate::{
    MAX_NATIVE_PERMISSION_IDENTITY_BYTES, NativePermissionPolicySnapshot, NativePermissionRuleKey,
    NativePermissionRuleKind, NativePreparedPermissionTargets, NativeSandboxMode,
    PreparedFileApproval,
};
use machine_god_core::{TerminalActionRequest, TerminalProfile};

pub(super) fn saved(
    targets: &NativePreparedPermissionTargets,
    file: Option<&PreparedFileApproval>,
    policy: &NativePermissionPolicySnapshot,
    semantic: &str,
) -> Option<NativePermissionRuleKey> {
    let mut fields = Fields::default();
    fields.push("machine-god-permission-v1")?;
    fields.push(targets.workspace())?;
    let kind = if let Some(file) = file {
        fields.push(file.saved_rule_key().ok()?.canonical())?;
        NativePermissionRuleKind::FileMutation
    } else if let Some(command) = command(targets) {
        let resolution = targets.terminal_resolution()?;
        let shell = resolution.shell()?;
        fields.push(command.command)?;
        fields.push(command.cwd)?;
        fields.push(if command.background {
            "background"
        } else {
            "foreground"
        })?;
        fields.push(backend(policy))?;
        fields.push(std::env::consts::OS)?;
        fields.push("restricted")?;
        fields.push(shell.program().to_str()?)?;
        fields.push(match shell.profile() {
            TerminalProfile::User => "user",
            TerminalProfile::Clean => "clean",
        })?;
        fields.push(resolution.environment_sha256())?;
        fields.push(resolution.shell_selection_sha256())?;
        // Preserve additional execution-affecting start/monitor/PTY controls.
        fields.push(semantic)?;
        NativePermissionRuleKind::Command
    } else {
        fields.push(targets.tool_name())?;
        fields.push(semantic)?;
        if let Some(resolution) = targets.terminal_resolution() {
            fields.push(resolution.environment_sha256())?;
            fields.push(resolution.shell_selection_sha256())?;
            fields.push(backend(policy))?;
        }
        for target in targets.targets() {
            fields.push(target.role())?;
            fields.push(target.path())?;
        }
        NativePermissionRuleKind::StructuredTool
    };
    NativePermissionRuleKey::new(kind, &fields.0).ok()
}

pub(super) fn read_grant(workspace: &str, path: &str) -> Option<NativePermissionRuleKey> {
    let mut fields = Fields::default();
    for field in ["machine-god-read-grant-v1", workspace, "read_file", path] {
        fields.push(field)?;
    }
    NativePermissionRuleKey::new(NativePermissionRuleKind::StructuredTool, &fields.0).ok()
}

pub(super) const fn backend(policy: &NativePermissionPolicySnapshot) -> &'static str {
    match policy.effective_sandbox_mode() {
        NativeSandboxMode::None => "none",
        NativeSandboxMode::Os => "os",
    }
}

pub(super) struct Execution<'a> {
    pub command: &'a str,
    pub cwd: &'a str,
    pub background: bool,
}
pub(super) fn command(targets: &NativePreparedPermissionTargets) -> Option<Execution<'_>> {
    match targets.terminal_action()? {
        TerminalActionRequest::Exec { request } => Some(Execution {
            command: &request.command,
            cwd: &request.cwd,
            background: false,
        }),
        TerminalActionRequest::Start { request } => Some(Execution {
            command: request.command.as_deref()?,
            cwd: &request.cwd,
            background: true,
        }),
        _ => None,
    }
}

#[derive(Default)]
struct Fields(String);
impl Fields {
    fn push(&mut self, field: &str) -> Option<()> {
        let prefix = field.len().to_string();
        let needed = self
            .0
            .len()
            .checked_add(prefix.len())?
            .checked_add(1)?
            .checked_add(field.len())?;
        if needed > MAX_NATIVE_PERMISSION_IDENTITY_BYTES {
            return None;
        }
        self.0.push_str(&prefix);
        self.0.push(':');
        self.0.push_str(field);
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_boundaries_cannot_collide_and_limits_do_not_truncate() {
        let mut first = Fields::default();
        first.push("ab").unwrap();
        first.push("c").unwrap();
        let mut second = Fields::default();
        second.push("a").unwrap();
        second.push("bc").unwrap();
        assert_ne!(first.0, second.0);
        let before = first.0.clone();
        assert!(
            first
                .push(&"x".repeat(MAX_NATIVE_PERMISSION_IDENTITY_BYTES))
                .is_none()
        );
        assert_eq!(first.0, before);
        assert_ne!(
            read_grant("/one", "/one/file"),
            read_grant("/two", "/two/file")
        );
    }
}
