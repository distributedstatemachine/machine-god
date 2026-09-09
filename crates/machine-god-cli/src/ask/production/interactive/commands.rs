//! Slash presentation and dispatch only; native owns admission and publication.

use super::{Driver, presentation};
use machine_god_core::{InferenceOptions, ModelCatalog};
use machine_god_native::{
    MAX_NATIVE_QUEUED_PROMPT_BYTES, MAX_NATIVE_SLASH_INPUT_BYTES, NativeFastModeChange,
    NativeInteractiveControl, NativeInteractiveTransition, NativeModelCapabilities,
    NativeModelPreferences, NativeReasoningEffort, NativeSandboxMode, NativeSlashCommand,
    NativeSlashSubmission, NativeSlashSubmissionContext, PermissionMode, resolve_model_query,
    resolve_native_slash_submission,
};
use std::fmt::Write;

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;

const REJECTED: &[u8] = b"\n[command rejected; use /help for the supported grammar]\n> ";
const BUSY: &[u8] = b"\n[previous control is still pending; wait for its receipt]\n> ";
const UNAVAILABLE: &[u8] = b"\n[command unavailable in this interactive host]\n> ";
const HELP: &[u8] = b"\nCommands implemented in this host:\n\
/help /status /version /quit (/exit) /cancel\n\
/clear /new /reset /resume (picker) /continue /rename <title> /compact /undo /copy\n\
/permissions [ask|auto|yolo|reset] /sandbox [os|none] /allowlist\n\
/models /model [id-or-query|effort <name>|save|save-default] /fast\n\
Model selection and /fast request native session and available user-default saves;\n\
their independent results are reported separately. /resume has no arguments.\n\
Cmd/Super+R opens the all-workspace session picker.\n\
/allowlist [view [effective|local|user]|[local|user] add|remove|reset ...]\n\
/workspace [list|add PATH|remove PATH|clear]\n> ";

enum Submission<'a> {
    Empty,
    Prompt(&'a str),
    Cancel,
    Slash(NativeSlashCommand, &'a str),
}

fn submission(line: &str) -> Result<Submission<'_>, ()> {
    if line.len() > MAX_NATIVE_QUEUED_PROMPT_BYTES {
        return Err(());
    }
    let text = line.trim_start_matches([' ', '\t', '\r', '\n']);
    if text.is_empty() {
        return Ok(Submission::Empty);
    }
    // Ordinary prompts use the native 256 KiB bound, not the slash envelope.
    if !text.starts_with('/') {
        return Ok(Submission::Prompt(line));
    }
    if line.len() > MAX_NATIVE_SLASH_INPUT_BYTES {
        return Err(());
    }
    if text.trim_end_matches([' ', '\t']) == "/cancel" {
        return Ok(Submission::Cancel);
    }
    match resolve_native_slash_submission(line, NativeSlashSubmissionContext::default())
        .map_err(|_| ())?
    {
        NativeSlashSubmission::Valid(invocation) => {
            Ok(Submission::Slash(invocation.command, invocation.payload))
        }
        NativeSlashSubmission::NotLocal => Ok(Submission::Prompt(line)),
        NativeSlashSubmission::KnownInvalid { .. } | NativeSlashSubmission::UnknownLocal => Err(()),
    }
}

impl Driver {
    pub(super) fn command(&mut self, line: &str, now_ms: i64) {
        match submission(line) {
            Err(()) => self.note(REJECTED),
            Ok(Submission::Empty) => {}
            Ok(Submission::Cancel) => self.cancel_command(),
            Ok(Submission::Prompt(prompt)) => {
                if self.control_outcome.is_some() {
                    self.note(BUSY);
                    return;
                }
                if self.owner.enqueue(prompt.into()).is_err() {
                    self.note(BUSY);
                }
            }
            Ok(Submission::Slash(command, payload)) => self.slash(command, payload, now_ms),
        }
    }

    fn slash(&mut self, command: NativeSlashCommand, payload: &str, now_ms: i64) {
        use NativeSlashCommand as Command;
        match command {
            Command::Help => self.note(HELP),
            Command::Quit => self.shutdown(),
            Command::Status => self.show_status(),
            Command::Version => {
                self.note(concat!("\nmachine-god ", env!("CARGO_PKG_VERSION"), "\n> ").as_bytes());
            }
            Command::Clear => self.transition_command(NativeInteractiveTransition::Clear, now_ms),
            Command::New => self.transition_command(NativeInteractiveTransition::New, now_ms),
            Command::Reset => self.transition_command(NativeInteractiveTransition::Reset, now_ms),
            Command::Resume => {
                self.open_picker(machine_god_native::NativeSessionCatalogScope::CurrentWorkspace);
            }
            Command::Continue => self.control_command(
                NativeInteractiveControl::Continue {
                    options: InferenceOptions::default(),
                },
                now_ms,
            ),
            Command::Rename => self.control_command(
                NativeInteractiveControl::Rename {
                    title: payload.to_owned(),
                },
                now_ms,
            ),
            Command::Compact => self.control_command(NativeInteractiveControl::Compact, now_ms),
            Command::Undo => self.control_command(NativeInteractiveControl::UndoLast, now_ms),
            Command::Copy => self.copy_command(),
            Command::Permissions => self.permissions_command(payload),
            Command::Sandbox => self.sandbox_command(payload),
            Command::Model => self.model_command(payload, now_ms),
            Command::Models => self.show_models(),
            Command::Fast => self.fast_command(now_ms),
            Command::Allowlist => self.allowlist_command(payload, now_ms),
            Command::Workspace => self.workspace_command(payload, now_ms),
        }
    }

    fn cancel_command(&mut self) {
        if let Some(modal) = self.modal.take() {
            if self.inbox.cancel(modal.view.token()).is_ok() {
                self.note(b"\n[prompt cancelled]\n> ");
            } else {
                self.note(b"\n[prompt expired; cancellation ignored]\n> ");
            }
        } else if self.owner.request_cancel() {
            self.note(b"\n[cancellation requested; native work will settle]\n> ");
        } else {
            self.note(b"\n[no cancellable current work]\n> ");
        }
    }

    fn copy_command(&mut self) {
        if self.copy_outcome.is_some() || self.owner.request_copy().is_err() {
            self.note(b"\n[copy unavailable or previous copy receipt still pending]\n> ");
        }
    }

    fn allowlist_command(&mut self, payload: &str, now_ms: i64) {
        if self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        let Ok(request) = self.owner.parse_allowlist(payload) else {
            self.note(b"\n[usage: /allowlist [view [effective|local|user]|[local|user] add|remove|reset ...]]\n> ");
            return;
        };
        let Some(store) = self.user_config.clone() else {
            self.note(UNAVAILABLE);
            return;
        };
        self.control_command(
            NativeInteractiveControl::Allowlist { request, store },
            now_ms,
        );
    }

    fn workspace_command(&mut self, payload: &str, now_ms: i64) {
        let Ok(action) = crate::workspace::parse_slash(payload) else {
            self.note(b"\n[usage: /workspace [list|add PATH|remove PATH|clear]]\n> ");
            return;
        };
        let Some(store) = self.user_config.clone() else {
            self.note(UNAVAILABLE);
            return;
        };
        self.control_command(
            NativeInteractiveControl::Workspace { action, store },
            now_ms,
        );
    }

    fn control_command(&mut self, control: NativeInteractiveControl, now_ms: i64) {
        if self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        if self.owner.request_control(control, now_ms).is_err() {
            self.note(REJECTED);
        }
    }

    fn transition_command(&mut self, transition: NativeInteractiveTransition, now_ms: i64) {
        if self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        if self.owner.request_transition(transition, now_ms).is_err() {
            self.note(REJECTED);
            return;
        }
        // Request acceptance is synchronous/inert. Invalidate unresolved UI
        // authority before the next native poll, never an already confirmed save.
        self.inbox.deactivate();
        self.scope_active = false;
        self.modal.take();
    }

    fn permissions_command(&mut self, payload: &str) {
        if !payload.is_empty() && self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        let Some(permissions) = self.owner.runtime().permissions() else {
            self.note(UNAVAILABLE);
            return;
        };
        let result = if payload.is_empty() {
            Ok(())
        } else if payload.eq_ignore_ascii_case("ask") {
            permissions.set_mode(PermissionMode::Ask)
        } else if payload.eq_ignore_ascii_case("auto") {
            permissions.set_mode(PermissionMode::Auto)
        } else if payload.eq_ignore_ascii_case("yolo") {
            permissions.set_mode(PermissionMode::Yolo)
        } else if payload.eq_ignore_ascii_case("reset") {
            permissions.reset()
        } else {
            self.note(REJECTED);
            return;
        };
        if result.is_err() {
            self.note(UNAVAILABLE);
            return;
        }
        self.show_policy(payload.eq_ignore_ascii_case("reset"));
    }

    fn sandbox_command(&mut self, payload: &str) {
        if !payload.is_empty() && self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        let Some(permissions) = self.owner.runtime().permissions() else {
            self.note(UNAVAILABLE);
            return;
        };
        let result = if payload.is_empty() {
            Ok(())
        } else if payload.eq_ignore_ascii_case("os") {
            permissions.set_sandbox_mode(NativeSandboxMode::Os)
        } else if payload.eq_ignore_ascii_case("none") {
            permissions.set_sandbox_mode(NativeSandboxMode::None)
        } else {
            self.note(REJECTED);
            return;
        };
        if result.is_err() {
            self.note(UNAVAILABLE);
            return;
        }
        self.show_policy(false);
    }

    fn show_policy(&mut self, reset: bool) {
        let Some(policy) = self
            .owner
            .runtime()
            .permissions()
            .and_then(|owner| owner.snapshot().ok())
        else {
            self.note(UNAVAILABLE);
            return;
        };
        let mut output = crate::ask::production::interactive::bounded_output();
        let result = (|| {
            writeln!(
                output,
                "\n[permissions] mode={} sandbox={} effective_sandbox={}",
                mode_label(policy.mode()),
                sandbox_label(policy.sandbox_mode()),
                sandbox_label(policy.effective_sandbox_mode())
            )
            .map_err(|_| ())?;
            if reset {
                output
                    .write_str(
                        "Session grants cleared; saved rules and configured patterns retained.\n",
                    )
                    .map_err(|_| ())?;
            }
            output.write_str("> ").map_err(|_| ())
        })();
        self.display(output, result);
    }

    fn model_command(&mut self, payload: &str, now_ms: i64) {
        if payload.is_empty() {
            self.show_status();
            return;
        }
        if self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        if payload == "save" {
            self.control_command(NativeInteractiveControl::SaveModelSession, now_ms);
            return;
        }
        if payload == "save-default" {
            let Some(store) = self.user_config.clone() else {
                self.note(UNAVAILABLE);
                return;
            };
            self.control_command(
                NativeInteractiveControl::SaveModelDefaults { store },
                now_ms,
            );
            return;
        }
        let mut preferences = self.owner.runtime().model_preferences();
        if let Some(effort) = payload.strip_prefix("effort ") {
            let Ok(effort) = NativeReasoningEffort::parse(effort) else {
                self.note(REJECTED);
                return;
            };
            preferences.set_effort(effort);
        } else {
            let resolved = self
                .catalog
                .as_ref()
                .map(|catalog| {
                    // Bounded ephemeral provider-neutral projection; native retains
                    // query ordering/scoring and the shared source remains unchanged.
                    let models = ModelCatalog::new(
                        catalog
                            .entries()
                            .iter()
                            .map(|entry| entry.model().clone())
                            .collect(),
                        catalog.access(),
                    );
                    resolve_model_query(payload, &models)
                })
                .transpose();
            let Ok(resolved) = resolved else {
                self.note(REJECTED);
                return;
            };
            if preferences
                .set_model(resolved.flatten().as_deref().unwrap_or(payload))
                .is_err()
            {
                self.note(REJECTED);
                return;
            }
        }
        self.select_and_save(preferences, now_ms);
    }

    fn fast_command(&mut self, now_ms: i64) {
        if self.control_outcome.is_some() {
            self.note(BUSY);
            return;
        }
        let mut preferences = self.owner.runtime().model_preferences();
        let capabilities = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.details(preferences.model()))
            .map_or_else(NativeModelCapabilities::default, |entry| {
                entry.capabilities().clone()
            });
        if preferences.toggle_fast(&capabilities) == NativeFastModeChange::Unsupported {
            self.note(b"\n[fast] selected model does not advertise fast mode\n> ");
            return;
        }
        self.select_and_save(preferences, now_ms);
    }

    fn select_and_save(&mut self, preferences: NativeModelPreferences, now_ms: i64) {
        if self.owner.set_model_preferences(preferences).is_err() {
            self.note(BUSY);
            return;
        }
        let control = self
            .user_config
            .clone()
            .map_or(NativeInteractiveControl::SaveModelSession, |store| {
                NativeInteractiveControl::SaveModelDefaults { store }
            });
        if self.owner.request_control(control, now_ms).is_err() {
            self.note(b"\n[model selection accepted, but persistence could not start]\n> ");
        }
    }

    fn show_status(&mut self) {
        let preferences = self.owner.runtime().model_preferences();
        let status = self.owner.runtime().status();
        let mut output = crate::ask::production::interactive::bounded_output();
        let result = (|| {
            output.write_str("\n[session] ").map_err(|_| ())?;
            presentation::escaped(&mut output, self.owner.runtime().id().as_str())?;
            write!(
                output,
                "\nphase={:?} active={} queued={} preferences_pending={}\nmodel=",
                status.phase, status.active, status.queued_jobs, status.model_preferences_pending
            )
            .map_err(|_| ())?;
            presentation::escaped(&mut output, preferences.model())?;
            output.write_str(" requested_effort=").map_err(|_| ())?;
            presentation::escaped(&mut output, preferences.effort().label())?;
            writeln!(
                output,
                " requested_fast={}\n> ",
                preferences.requested_fast()
            )
            .map_err(|_| ())
        })();
        self.display(output, result);
    }

    fn show_models(&mut self) {
        let Some(catalog) = &self.catalog else {
            self.note(UNAVAILABLE);
            return;
        };
        let mut output = crate::ask::production::interactive::bounded_output();
        let result = (|| {
            writeln!(
                output,
                "\n[models] {} cached entries",
                catalog.entries().len()
            )
            .map_err(|_| ())?;
            let mut shown = 0;
            for entry in catalog.entries() {
                // Reserve a worst-case escaped model ID plus the omission notice.
                if output.len() + entry.model().id().len() * 6 + 256
                    > crate::ask::production::interactive::MAX_PRESENTATION_OUTPUT_BYTES
                {
                    break;
                }
                presentation::escaped(&mut output, entry.model().id())?;
                output.write_char('\n').map_err(|_| ())?;
                shown += 1;
            }
            if shown < catalog.entries().len() {
                writeln!(
                    output,
                    "[{} further entries omitted at output bound]",
                    catalog.entries().len() - shown
                )
                .map_err(|_| ())?;
            }
            output
                .write_str("Select with /model <id-or-query>.\n> ")
                .map_err(|_| ())
        })();
        self.display(output, result);
    }

    fn display(&mut self, output: crate::bounded_output::BoundedOutput, result: Result<(), ()>) {
        if result.is_err() {
            self.note(b"\n[command output exceeds display bound]\n> ");
        } else if self.notice.is_none() {
            self.notice = Some(output.finish().into_bytes());
        }
    }
}

fn mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "ask",
        PermissionMode::Auto => "auto",
        PermissionMode::Yolo => "yolo",
    }
}
fn sandbox_label(mode: NativeSandboxMode) -> &'static str {
    match mode {
        NativeSandboxMode::Os => "os",
        NativeSandboxMode::None => "none",
    }
}
