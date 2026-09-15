//! Native same-session commands. Accepted effects stay in the interactive owner.

use super::session::NativeAcpSession;
use crate::{
    NativeInteractiveControl, NativeInteractiveControlId, NativeInteractiveControlOutcome,
    NativeModelCapabilities, NativeModelPreferences, NativeSlashCommand,
};
use machine_god_core::{BackgroundOutputOwner, SessionId};
use std::{fmt, sync::Arc};

mod output;
mod parse;
pub use output::{MAX_ACP_COMMAND_OUTPUT_BYTES, NativeAcpCommandResult, available_commands};
pub use parse::classify;
#[cfg(test)]
mod tests;

/// Opaque, bounded intent produced only by the native command grammar.
pub struct NativeAcpCommand {
    name: NativeSlashCommand,
    action: Action,
}
impl fmt::Debug for NativeAcpCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpCommand(..)")
    }
}
enum Action {
    Help,
    Status,
    Models,
    Permissions,
    Allowlist,
    SaveModel,
    Model(String),
    Effort(crate::NativeReasoningEffort),
    Fast,
    Compact,
    Undo,
    Skills,
    Mcp,
    McpFeature(crate::mcp::commands::McpFeatureCommand),
}

/// Stable diagnostic categories; no user content or nested host errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpCommandError {
    Invalid,
    Unsupported,
    Limit,
    Busy,
    WrongSession,
    Unavailable,
}
impl fmt::Display for NativeAcpCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid ACP command",
            Self::Unsupported => "unsupported ACP command",
            Self::Limit => "ACP command limit exceeded",
            Self::Busy => "ACP command lane is busy",
            Self::WrongSession => "ACP command identity does not match",
            Self::Unavailable => "ACP command unavailable",
        })
    }
}
impl std::error::Error for NativeAcpCommandError {}
type Error = NativeAcpCommandError;
type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Services {
    pub(crate) skills: bool,
    pub(crate) mcp: Option<Arc<crate::mcp::runtime::NativeMcpRuntime>>,
}
impl Services {
    pub(crate) fn from_session(session: &crate::NativeInteractiveSession) -> Self {
        Self {
            skills: session.skills_catalog().is_some(),
            mcp: session.acp_mcp_runtime(),
        }
    }
}

struct Pending {
    id: NativeInteractiveControlId,
    principal: BackgroundOutputOwner,
    name: NativeSlashCommand,
    accepted_model_generation: Option<u64>,
    cancellation_requested: bool,
}

/// One command and one bounded result. Dropping this presentation owner cannot
/// discard an accepted native effect; its control receipt remains in the session.
#[derive(Default)]
pub struct NativeAcpCommandOwner {
    pending: Option<Pending>,
    result: Option<NativeAcpCommandResult>,
}
impl fmt::Debug for NativeAcpCommandOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpCommandOwner(..)")
    }
}
impl NativeAcpCommandOwner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    #[must_use]
    pub fn pending_control(&self) -> Option<(NativeInteractiveControlId, BackgroundOutputOwner)> {
        self.pending
            .as_ref()
            .map(|pending| (pending.id, pending.principal.clone()))
    }

    /// Records already-observed native cancellation for the exact pending owner.
    /// This changes presentation metadata only and never cancels native work.
    /// # Errors
    /// Rejects a foreign session incarnation without changing this command.
    pub fn note_cancellation_requested(
        &mut self,
        principal: &BackgroundOutputOwner,
    ) -> Result<bool> {
        let Some(pending) = &mut self.pending else {
            return Ok(false);
        };
        if pending.principal != *principal {
            return Err(Error::WrongSession);
        }
        pending.cancellation_requested = true;
        Ok(true)
    }
    /// Accepts intent only after exact current-session and bounded lane checks.
    /// # Errors
    /// Rejects unavailable capabilities, invalid identities or an occupied lane.
    pub fn begin(
        &mut self,
        session: &mut NativeAcpSession,
        expected: &SessionId,
        command: NativeAcpCommand,
        now_ms: i64,
    ) -> Result<()> {
        if self.pending.is_some() || self.result.is_some() {
            return Err(Error::Busy);
        }
        session
            .check_command_admission(expected)
            .map_err(|error| match error {
                super::session::AcpSessionError::WrongSession => Error::WrongSession,
                super::session::AcpSessionError::Busy => Error::Busy,
                _ => Error::Unavailable,
            })?;
        let principal = session.principal();
        let name = command.name;
        let (control, generation) = match command.action {
            Action::Compact => (NativeInteractiveControl::Compact, None),
            Action::Undo => (NativeInteractiveControl::UndoLast, None),
            Action::SaveModel => (NativeInteractiveControl::SaveModelSession, None),
            Action::Model(model) => {
                let mut preferences = session.runtime().model_preferences();
                preferences
                    .set_model(&resolve_model(session, &model)?)
                    .map_err(|_| Error::Invalid)?;
                model_control(session, expected, preferences)?
            }
            Action::Effort(effort) => {
                let mut preferences = session.runtime().model_preferences();
                preferences.set_effort(effort);
                model_control(session, expected, preferences)?
            }
            Action::Fast => {
                let mut preferences = session.runtime().model_preferences();
                let capabilities = capabilities(session, preferences.model());
                if preferences.toggle_fast(&capabilities)
                    == crate::NativeFastModeChange::Unsupported
                {
                    return Err(Error::Unsupported);
                }
                model_control(session, expected, preferences)?
            }
            Action::Skills if session.command_services.skills => (
                NativeInteractiveControl::Skills {
                    command: crate::NativeSkillsCommand::List,
                },
                None,
            ),
            Action::McpFeature(feature) if session.command_services.mcp.is_some() => (
                NativeInteractiveControl::Mcp {
                    command: crate::mcp::commands::McpCommand::Feature(feature),
                },
                None,
            ),
            action => {
                let data = output::observation(session, &action)?;
                self.result = Some(NativeAcpCommandResult::new(principal, name, false, data));
                return Ok(());
            }
        };
        match session.request_command_control(expected, control, now_ms) {
            Ok(id) => {
                self.pending = Some(Pending {
                    id,
                    principal,
                    name,
                    accepted_model_generation: generation,
                    cancellation_requested: false,
                });
            }
            Err(_) if generation.is_some() => {
                self.result = Some(NativeAcpCommandResult::new(
                    principal,
                    name,
                    true,
                    serde_json::json!({
                        "acceptedGeneration":generation, "persistence":"notStarted"
                    }),
                ));
            }
            Err(_) => return Err(Error::Unavailable),
        }
        Ok(())
    }
    /// Requests cancellation without consuming the exact native receipt.
    /// # Errors
    /// Rejects a foreign incarnation or unavailable session owner.
    pub fn cancel(&mut self, session: &mut NativeAcpSession) -> Result<bool> {
        let Some(pending) = &mut self.pending else {
            return Ok(false);
        };
        if pending.principal != session.principal() {
            return Err(Error::WrongSession);
        }
        let accepted = session
            .request_cancel(&session.id())
            .map_err(|_| Error::Unavailable)?;
        pending.cancellation_requested |= accepted;
        Ok(accepted)
    }
    /// Consumes only an exact ID+incarnation receipt; mismatches leave this
    /// pending command untouched. Native failures/partial effects are retained.
    /// # Errors
    /// Rejects an unsolicited or mismatched receipt.
    pub fn complete(
        &mut self,
        outcome: &mut Option<NativeInteractiveControlOutcome>,
    ) -> Result<()> {
        let pending = self.pending.as_ref().ok_or(Error::Unavailable)?;
        let observed = outcome.as_ref().ok_or(Error::Unavailable)?;
        if pending.id != observed.id || pending.principal != observed.source {
            return Err(Error::WrongSession);
        }
        let pending = self.pending.take().ok_or(Error::Unavailable)?;
        self.result = Some(output::receipt(
            pending,
            outcome.take().ok_or(Error::Unavailable)?,
        ));
        Ok(())
    }
    #[must_use]
    pub fn take_result(&mut self) -> Option<NativeAcpCommandResult> {
        self.result.take()
    }

    pub(crate) fn result(&self) -> Option<&NativeAcpCommandResult> {
        self.result.as_ref()
    }
}

fn model_control(
    session: &mut NativeAcpSession,
    expected: &SessionId,
    preferences: NativeModelPreferences,
) -> Result<(NativeInteractiveControl, Option<u64>)> {
    let generation = session
        .set_command_model_preferences(expected, preferences)
        .map_err(|_| Error::Unavailable)?;
    Ok((NativeInteractiveControl::SaveModelSession, Some(generation)))
}
fn capabilities(session: &NativeAcpSession, model: &str) -> NativeModelCapabilities {
    session
        .runtime()
        .model_catalog()
        .and_then(|catalog| {
            catalog
                .details(model)
                .map(|entry| entry.capabilities().clone())
        })
        .unwrap_or_default()
}
fn resolve_model(session: &NativeAcpSession, query: &str) -> Result<String> {
    let Some(catalog) = session.runtime().model_catalog() else {
        return Ok(query.to_owned());
    };
    let catalog = machine_god_core::ModelCatalog::new(
        catalog
            .entries()
            .iter()
            .map(|entry| entry.model().clone())
            .collect(),
        catalog.access(),
    );
    crate::resolve_model_query(query, &catalog)
        .map_err(|_| Error::Invalid)
        .map(|resolved| resolved.unwrap_or_else(|| query.to_owned()))
}
