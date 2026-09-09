//! Human confirmation presentation; native tokens own exact rule authority.

use super::{Driver, bounded_output, input_lines::InputBinding, presentation};
use machine_god_native::{
    NativeInteractiveControl, NativeInteractiveControlId, NativeInteractivePromptToken,
    NativePermissionRuleChange, NativePermissionRuleDecision, NativePermissionRuleProposal,
    NativeSessionPermissionRules,
};
use std::fmt::Write;

#[cfg(test)]
mod tests;

const UNAVAILABLE: &[u8] = b"\n[saved rule unavailable or stale; no success confirmed]\n> ";
const USAGE: &[u8] = b"\n[usage: /permissions rules [offset] | /permissions revoke <id>]\n> ";

pub(super) struct Confirmation {
    generation: u64,
    displayed: bool,
    proposal: Option<NativePermissionRuleProposal>,
    pending: Option<NativeInteractiveControlId>,
    prompt: Option<NativeInteractivePromptToken>,
    bytes: Vec<u8>,
}

impl Confirmation {
    pub fn binding(&self) -> InputBinding {
        if self.displayed && self.pending.is_none() {
            InputBinding::SavedRule(self.generation)
        } else {
            InputBinding::AwaitingPrompt
        }
    }

    pub fn render(&self) -> Option<(Vec<u8>, InputBinding)> {
        (!self.displayed && self.pending.is_none())
            .then(|| (self.bytes.clone(), InputBinding::SavedRule(self.generation)))
    }
}

impl Driver {
    pub(super) fn prompt_policy_command(
        &mut self,
        line: &str,
        binding: &InputBinding,
        now_ms: i64,
    ) -> bool {
        let line = line.trim_matches([' ', '\t', '\r', '\n']);
        if !matches!(
            line,
            "/permissions ask" | "/permissions auto" | "/permissions yolo" | "/permissions reset"
        ) || (self.saved_rule.is_none() && self.modal.is_none())
        {
            return false;
        }
        let current = self
            .saved_rule
            .as_ref()
            .map(Confirmation::binding)
            .or_else(|| self.modal.as_ref().map(super::presentation::Modal::binding));
        if current.as_ref() == Some(binding) && !matches!(binding, InputBinding::AwaitingPrompt) {
            self.command(line, now_ms);
        } else {
            self.note(b"\n[stale prompt policy input ignored]\n> ");
        }
        true
    }

    pub(super) fn saved_rules_command(&mut self, payload: &str) -> bool {
        let mut words = payload.split_ascii_whitespace();
        match words.next() {
            Some("rules") => {
                let offset = words.next().map_or(Ok(0), str::parse::<usize>);
                if let (Ok(offset), None) = (offset, words.next()) {
                    self.show_saved_rules(offset);
                } else {
                    self.note(USAGE);
                }
            }
            Some("revoke") => {
                let id = words.next().and_then(|value| value.parse::<u64>().ok());
                if words.next().is_some() || id.is_none_or(|id| id == 0) {
                    self.note(USAGE);
                } else {
                    self.propose_revoke(id.expect("checked id"));
                }
            }
            _ => return false,
        }
        true
    }

    fn show_saved_rules(&mut self, offset: usize) {
        let snapshot = self.owner.runtime().record_snapshot();
        let Ok(rules) = NativeSessionPermissionRules::from_metadata(&snapshot.metadata) else {
            self.note(UNAVAILABLE);
            return;
        };
        if offset > rules.rules().len() {
            self.note(USAGE);
            return;
        }
        let mut output = bounded_output();
        let result = (|| {
            writeln!(
                output,
                "\n[saved exact rules] {} total; offset {offset}",
                rules.rules().len()
            )
            .map_err(|_| ())?;
            let mut shown = 0;
            for rule in &rules.rules()[offset..] {
                if output.len() + rule.display_identity().len() * 6 + 256
                    > super::MAX_PRESENTATION_OUTPUT_BYTES
                {
                    break;
                }
                write!(
                    output,
                    "{} {:?} {:?}: ",
                    rule.id(),
                    rule.decision(),
                    rule.key().kind()
                )
                .map_err(|_| ())?;
                presentation::escaped(&mut output, rule.display_identity())?;
                output.write_char('\n').map_err(|_| ())?;
                shown += 1;
            }
            if offset + shown < rules.rules().len() {
                writeln!(output, "More: /permissions rules {}", offset + shown).map_err(|_| ())?;
            }
            output
                .write_str("Revoke with /permissions revoke <id>; confirmation required.\n> ")
                .map_err(|_| ())
        })();
        if result.is_err() {
            self.note(UNAVAILABLE);
        } else if self.notice.is_none() {
            self.notice = Some(output.finish().into_bytes());
        }
    }

    fn propose_revoke(&mut self, id: u64) {
        if self.control_outcome.is_some() || self.saved_rule.is_some() {
            self.note(UNAVAILABLE);
            return;
        }
        let Some(owner) = self.owner.runtime().permissions() else {
            self.note(UNAVAILABLE);
            return;
        };
        // Freeze the native generation before observing display metadata. Any
        // subsequent replacement makes confirmation stale, rather than letting
        // an old display describe a token pinned to a newer rule generation.
        let Ok(proposal) = owner.propose_rule_change(NativePermissionRuleChange::Revoke { id })
        else {
            self.note(UNAVAILABLE);
            return;
        };
        let snapshot = self.owner.runtime().record_snapshot();
        let Ok(rules) = NativeSessionPermissionRules::from_metadata(&snapshot.metadata) else {
            self.note(UNAVAILABLE);
            return;
        };
        let Some(rule) = rules.rule_for_id(id) else {
            self.note(UNAVAILABLE);
            return;
        };
        let mut output = bounded_output();
        let result = (|| {
            write!(
                output,
                "\n[confirm saved-rule revocation] id={id} {:?} {:?}\n",
                rule.decision(),
                rule.key().kind()
            )
            .map_err(|_| ())?;
            presentation::escaped(&mut output, rule.display_identity())?;
            output
                .write_str("\n[yes] revoke this exact saved rule  [no] cancel\n> ")
                .map_err(|_| ())
        })();
        if result.is_err() {
            self.note(UNAVAILABLE);
            return;
        }
        self.install_rule_confirmation(proposal, None, output.finish().into_bytes());
    }

    fn install_rule_confirmation(
        &mut self,
        proposal: NativePermissionRuleProposal,
        prompt: Option<NativeInteractivePromptToken>,
        bytes: Vec<u8>,
    ) {
        let Some(generation) = self.rule_generation.checked_add(1) else {
            self.note(UNAVAILABLE);
            return;
        };
        self.rule_generation = generation;
        self.saved_rule = Some(Confirmation {
            generation,
            displayed: false,
            proposal: Some(proposal),
            pending: None,
            prompt,
            bytes,
        });
    }

    pub(super) fn propose_prompt_rule(&mut self, line: &str, binding: &InputBinding) -> bool {
        let decision = match line.trim() {
            "a" => NativePermissionRuleDecision::Allow,
            "d" => NativePermissionRuleDecision::Deny,
            _ => return false,
        };
        let Some(modal) = &self.modal else {
            return false;
        };
        if !modal.view.can_save_rule() {
            return false;
        }
        if !modal.displayed || &modal.binding() != binding || self.control_outcome.is_some() {
            self.note(UNAVAILABLE);
            return true;
        }
        let token = modal.view.token().clone();
        let Ok(proposal) = self.inbox.propose_rule_change(&token, decision) else {
            self.note(UNAVAILABLE);
            return true;
        };
        let Ok(mut bytes) = modal.render() else {
            self.note(UNAVAILABLE);
            return true;
        };
        bytes.extend_from_slice(match decision {
            NativePermissionRuleDecision::Allow => {
                b"\n[confirm saved exact allow] Applies only after fresh native preparation.\n"
            }
            NativePermissionRuleDecision::Deny => {
                b"\n[confirm saved exact deny] Applies only after fresh native preparation.\n"
            }
        });
        bytes.extend_from_slice(b"The pending action will not execute or retry.\n[yes] save exact rule  [no] cancel\n> ");
        if bytes.len() > super::MAX_PRESENTATION_OUTPUT_BYTES {
            self.note(UNAVAILABLE);
            return true;
        }
        self.install_rule_confirmation(proposal, Some(token), bytes);
        true
    }

    pub(super) fn answer_saved_rule(&mut self, line: &str, binding: &InputBinding, now_ms: i64) {
        let Some(confirmation) = &mut self.saved_rule else {
            return;
        };
        if !confirmation.displayed
            || confirmation.pending.is_some()
            || binding != &confirmation.binding()
        {
            self.note(b"\n[stale confirmation input ignored]\n> ");
            return;
        }
        match line.trim() {
            "no" | "n" => {
                self.cancel_saved_rule();
            }
            "yes" => {
                let Some(proposal) = confirmation.proposal.take() else {
                    return;
                };
                if let Ok(id) = self.owner.request_control(
                    NativeInteractiveControl::ConfirmPermissionRule { proposal },
                    now_ms,
                ) {
                    confirmation.pending = Some(id);
                } else {
                    self.saved_rule.take();
                    self.note(UNAVAILABLE);
                }
            }
            _ => self.note(b"\n[answer yes or no to the displayed saved-rule confirmation]\n> "),
        }
    }

    pub(super) fn cancel_saved_rule(&mut self) -> bool {
        let Some(confirmation) = self.saved_rule.take() else {
            return false;
        };
        // An accepted save remains owned by the native control lane. Cancellation
        // retires only unanswered presentation authority, never claims rollback.
        if let Some(token) = &confirmation.prompt {
            let _ = self.inbox.cancel(token);
            self.modal.take();
        }
        self.note(if confirmation.pending.is_some() {
            b"\n[confirmation dismissed; already accepted save retains its native receipt]\n> "
        } else {
            b"\n[saved-rule proposal cancelled]\n> "
        });
        true
    }

    pub(super) fn poll_saved_rule(&mut self) {
        let Some(confirmation) = &self.saved_rule else {
            return;
        };
        let completed = confirmation.pending.is_some_and(|id| {
            self.control_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.id == id)
        });
        let stale = confirmation.prompt.as_ref().is_some_and(|token| {
            self.modal
                .as_ref()
                .is_none_or(|modal| modal.view.token() != token)
        });
        if completed || stale || !self.scope_active {
            let confirmation = self.saved_rule.take().expect("observed confirmation");
            if completed && let Some(token) = confirmation.prompt {
                let _ = self.inbox.cancel(&token);
                self.modal.take();
            }
        }
    }

    pub(super) fn acknowledge_saved_rule(&mut self, binding: &InputBinding) {
        if let (InputBinding::SavedRule(generation), Some(confirmation)) =
            (binding, &mut self.saved_rule)
            && *generation == confirmation.generation
            && self.scope_active
            && !self.shutting_down
        {
            confirmation.displayed = true;
        }
    }
}
