use super::{Action, Error, Pending, Result, capabilities};
use crate::acp::session::NativeAcpSession;
use crate::{
    NativeInteractiveControlOutcome, NativeInteractiveControlReceipt as Receipt,
    NativeSlashCommand as Command,
};
use machine_god_core::BackgroundOutputOwner;
use serde_json::{Value, json};
use std::{
    fmt,
    io::{self, Write},
};

/// Inclusive serialized update limit, including command-result framing.
pub const MAX_ACP_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
const DATA_BYTES: usize = 32 * 1024;
const MAX_ROWS: usize = 128;

pub struct NativeAcpCommandResult {
    principal: BackgroundOutputOwner,
    name: Command,
    failed: bool,
    cancelled: bool,
    data: Value,
    _receipt: Option<NativeInteractiveControlOutcome>,
}
impl fmt::Debug for NativeAcpCommandResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpCommandResult(..)")
    }
}
impl NativeAcpCommandResult {
    pub(super) fn new(
        principal: BackgroundOutputOwner,
        name: Command,
        failed: bool,
        data: Value,
    ) -> Self {
        let data = if fits(&data, DATA_BYTES) {
            data
        } else {
            json!({"outputOmitted":true})
        };
        Self {
            principal,
            name,
            failed,
            cancelled: false,
            data,
            _receipt: None,
        }
    }
    #[must_use]
    pub const fn principal(&self) -> &BackgroundOutputOwner {
        &self.principal
    }
    #[must_use]
    pub const fn failed(&self) -> bool {
        self.failed
    }
    #[must_use]
    pub const fn cancelled(&self) -> bool {
        self.cancelled
    }
    /// A human command is not a model tool call and receives no invented call ID.
    #[must_use]
    pub fn update(&self) -> Value {
        let status = if self.failed { "failed" } else { "completed" };
        let command = token(self.name);
        let rendered =
            serde_json::to_string(&self.data).unwrap_or_else(|_| "[Output unavailable]".to_owned());
        let mut end = rendered.len().min(8192);
        while !rendered.is_char_boundary(end) {
            end -= 1;
        }
        let suffix = if end < rendered.len() {
            "\n[Preview shortened; structured result attached.]"
        } else {
            ""
        };
        let text = format!("{command}: {status}\n{}{suffix}", &rendered[..end]);
        let mut update = json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":text},
            "command_result":{"kind":"native_command", "command":command, "status":status,
                "cancelled":self.cancelled}});
        update["command_result"]["receipt"] = self.data.clone();
        update
    }
}

fn token(command: Command) -> &'static str {
    crate::native_slash_registry()
        .iter()
        .find(|spec| spec.command == command)
        .expect("native command registry")
        .token
}
fn supported(session: &NativeAcpSession, command: Command) -> bool {
    match command {
        Command::Help | Command::Status | Command::Model | Command::Compact | Command::Undo => true,
        Command::Permissions | Command::Allowlist => session.runtime().permissions().is_some(),
        Command::Models => session.runtime().model_catalog().is_some(),
        Command::Fast => {
            capabilities(session, session.runtime().model_preferences().model()).supports_fast()
        }
        Command::Skills => session.command_services.skills,
        Command::Mcp => session.command_services.mcp.is_some(),
        _ => false,
    }
}
/// Capability-sensitive static advertisements; no discovery, I/O or model work.
#[must_use]
pub fn available_commands(session: &NativeAcpSession) -> Value {
    let commands: Vec<_> = crate::native_slash_registry()
        .iter()
        .filter(|spec| supported(session, spec.command))
        .map(|spec| {
            let (description, hint) = match spec.command {
                Command::Model => (
                    "Select or save the current session model",
                    Some("ID | effort LEVEL | save"),
                ),
                Command::Permissions => ("Show current session permission policy", None),
                Command::Allowlist => (
                    "Show effective configured allow rules",
                    Some("view effective"),
                ),
                Command::Mcp => (
                    "Inspect ephemeral MCP readiness or run a resource/prompt action",
                    Some("list | resource ... | prompt ..."),
                ),
                Command::Skills => ("List the injected native skills catalog", Some("list")),
                _ => (spec.description, None),
            };
            let mut entry = json!({"name":&spec.token[1..], "description":description});
            if let Some(hint) = hint {
                entry["input"] = json!({"hint":hint});
            }
            entry
        })
        .collect();
    json!({"sessionUpdate":"available_commands_update", "availableCommands":commands})
}

pub(super) fn observation(session: &NativeAcpSession, action: &Action) -> Result<Value> {
    Ok(match action {
        Action::Help => available_commands(session),
        Action::Status => {
            let preferences = session.runtime().model_preferences();
            let status = session.runtime().status();
            json!({"model":preferences.model(), "effort":preferences.effort().label(),
                "requestedFast":preferences.requested_fast(), "active":status.active,
                "queuedJobs":status.queued_jobs, "modelPreferencesPending":status.model_preferences_pending})
        }
        Action::Permissions => {
            let policy = policy(session)?;
            json!({"mode":policy.mode().as_str(), "sandbox":policy.sandbox_mode().as_str(),
                "effectiveSandbox":policy.effective_sandbox_mode().as_str()})
        }
        Action::Allowlist => {
            let policy = policy(session)?;
            let mut rows = Rows::new();
            for rule in policy.configured_rules().rules() {
                if rule.decision() == crate::NativeConfiguredPermissionDecision::Allow
                    && (rule.permission() != "web_fetch"
                        || crate::permission_patterns::canonical_web_domain(rule.pattern()))
                {
                    rows.push(json!({"permission":rule.permission(), "pattern":rule.pattern()}));
                }
            }
            rows.finish()
        }
        Action::Models => {
            let catalog = session
                .runtime()
                .model_catalog()
                .ok_or(Error::Unavailable)?;
            let mut rows = Rows::new();
            rows.omitted = catalog.entries().len().saturating_sub(MAX_ROWS);
            for entry in catalog.entries().iter().take(MAX_ROWS) {
                rows.push(
                    json!({"id":entry.model().id(), "fast":entry.capabilities().supports_fast()}),
                );
            }
            rows.finish()
        }
        Action::Mcp => {
            let runtime = session
                .command_services
                .mcp
                .as_ref()
                .ok_or(Error::Unavailable)?;
            let checkpoint = runtime
                .publication_checkpoint()
                .map_err(|_| Error::Unavailable)?;
            json!({"selection":"ephemeral", "published":!checkpoint.is_unpublished(), "profileFallback":false})
        }
        _ => return Err(Error::Unsupported),
    })
}
fn policy(session: &NativeAcpSession) -> Result<crate::NativePermissionPolicySnapshot> {
    session
        .runtime()
        .permissions()
        .ok_or(Error::Unavailable)?
        .snapshot()
        .map_err(|_| Error::Unavailable)
}

pub(super) fn receipt(
    pending: Pending,
    outcome: NativeInteractiveControlOutcome,
) -> NativeAcpCommandResult {
    let mut failed = outcome.failed();
    let mut data = match &outcome.result {
        Ok(Receipt::Compacted(changed)) => json!({"changed":changed}),
        Ok(Receipt::Undone(crate::FileUndoOutcome::Empty)) => json!({"outcome":"empty"}),
        Ok(Receipt::Undone(crate::FileUndoOutcome::Restored(path))) => {
            json!({"outcome":"restored", "path":path})
        }
        Ok(Receipt::Undone(crate::FileUndoOutcome::Removed(path))) => {
            json!({"outcome":"removed", "path":path})
        }
        Ok(Receipt::ModelSession(persistence)) => match persistence {
            crate::NativeModelPreferencePersistence::Unchanged => {
                json!({"persistence":"unchanged"})
            }
            crate::NativeModelPreferencePersistence::Deferred => json!({"persistence":"deferred"}),
            crate::NativeModelPreferencePersistence::Saved {
                generation,
                revision,
            } => json!({"persistence":"saved", "generation":generation, "revision":revision}),
        },
        Ok(Receipt::Skills(crate::NativeSkillsServiceResult::Catalog(view))) => {
            let mut rows = Rows::new();
            rows.omitted = view.snapshot.entries().len().saturating_sub(MAX_ROWS);
            for entry in view.snapshot.entries().iter().take(MAX_ROWS) {
                rows.push(
                    json!({"name":entry.selection_ref().name(), "location":entry.location()}),
                );
            }
            let mut data = rows.finish();
            data["complete"] = json!(view.snapshot.complete());
            data["diagnostics"] = json!(view.snapshot.diagnostics().len());
            data
        }
        Ok(Receipt::McpFeature(receipt)) => {
            if receipt.revalidate().is_err() {
                failed = true;
            }
            mcp_receipt(receipt)
        }
        Err(_) => json!({"outcome":"nativeFailure"}),
        Ok(_) => {
            failed = true;
            json!({"outcome":"unexpectedReceipt"})
        }
    };
    if let Some(generation) = pending.accepted_model_generation {
        data["acceptedGeneration"] = json!(generation);
    }
    let result = NativeAcpCommandResult::new(pending.principal, pending.name, failed, data);
    NativeAcpCommandResult {
        cancelled: pending.cancellation_requested,
        _receipt: Some(outcome),
        ..result
    }
}

fn mcp_receipt(receipt: &crate::NativeMcpHumanFeatureReceipt) -> Value {
    use crate::mcp::{catalog::McpDescriptor, control::McpFeatureReply};
    match receipt.reply() {
        McpFeatureReply::Response(response) => bounded_raw(response.result_json().get()),
        McpFeatureReply::Catalog(catalog) => {
            let mut rows = Rows::new();
            rows.omitted = catalog.descriptors().len().saturating_sub(MAX_ROWS);
            for descriptor in catalog.descriptors().iter().take(MAX_ROWS) {
                let raw = match descriptor {
                    McpDescriptor::Tool(value) => value.raw_json(),
                    McpDescriptor::Resource(value) => value.raw_json(),
                    McpDescriptor::ResourceTemplate(value) => value.raw_json(),
                    McpDescriptor::Prompt(value) => value.raw_json(),
                };
                rows.push(bounded_raw(raw.get()));
            }
            rows.finish()
        }
    }
}
fn bounded_raw(raw: &str) -> Value {
    if raw.len() <= DATA_BYTES / 2
        && let Ok(value) = crate::acp::protocol::decode_value(raw.as_bytes())
        && crate::acp::protocol::validate_value(&value, 8).is_ok()
    {
        return value;
    }
    json!({"outputOmitted":true, "bytes":raw.len()})
}

struct Rows {
    values: Vec<Value>,
    remaining: usize,
    omitted: usize,
}
impl Rows {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            remaining: DATA_BYTES - 1024,
            omitted: 0,
        }
    }
    fn push(&mut self, value: Value) {
        let mut counter = Counter(self.remaining);
        if self.values.len() < MAX_ROWS && serde_json::to_writer(&mut counter, &value).is_ok() {
            self.remaining = counter.0.saturating_sub(1);
            self.values.push(value);
        } else {
            self.omitted += 1;
        }
    }
    fn finish(self) -> Value {
        let mut value = json!({"omitted":self.omitted});
        value["items"] = Value::Array(self.values);
        value
    }
}
struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("command output limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn fits(value: &Value, maximum: usize) -> bool {
    serde_json::to_writer(Counter(maximum), value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_omissions_and_mcp_nesting_reserve_wire_framing_budget() {
        let mut rows = Rows::new();
        for _ in 0..MAX_ROWS + 3 {
            rows.push(json!({"name":"item"}));
        }
        let value = rows.finish();
        assert_eq!(value["items"].as_array().unwrap().len(), MAX_ROWS);
        assert_eq!(value["omitted"], 3);
        let raw = format!("{}0{}", "[".repeat(60), "]".repeat(60));
        assert!(crate::acp::protocol::decode_value(raw.as_bytes()).is_ok());
        assert_eq!(bounded_raw(&raw)["outputOmitted"], true);
        assert_eq!(
            bounded_raw("{\"n\":-0}")["n"].as_number().unwrap().as_str(),
            "-0"
        );
    }
}
