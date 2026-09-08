use super::{NativePermissionSession, lock, read_rules, unavailable};
use crate::{
    NATIVE_SESSION_PERMISSION_RULES_KEY, NativePermissionRuleDecision, NativePermissionRuleKey,
    NativeSessionPermissionRules,
};
use machine_god_core::{BoxFuture, PermissionError, SessionRevision, TurnMetadataEditor};
use std::fmt;
use std::sync::{Arc, Weak};

/// An explicit host-requested change. Preparing it is not confirmation.
#[derive(Clone, Debug)]
pub enum NativePermissionRuleChange {
    Set {
        key: NativePermissionRuleKey,
        display_identity: String,
        decision: NativePermissionRuleDecision,
    },
    Revoke {
        id: u64,
    },
}

/// Opaque, single-use proposal pinned to one owner and target-rule generation.
/// A host must separately obtain explicit human confirmation before consuming
/// it. A display string, restored record or model output is not confirmation.
pub struct NativePermissionRuleProposal {
    owner: Weak<NativePermissionSession>,
    change: NativePermissionRuleChange,
    expected_generation: Option<u64>,
}

impl fmt::Debug for NativePermissionRuleProposal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionRuleProposal { .. }")
    }
}

impl NativePermissionSession {
    /// Validates a proposed edit against current canonical rules, without I/O.
    /// # Errors
    /// Rejects malformed rules, unavailable ownership and invalid changes.
    pub fn propose_rule_change(
        self: &Arc<Self>,
        change: NativePermissionRuleChange,
    ) -> Result<NativePermissionRuleProposal, PermissionError> {
        let state = lock(&self.state);
        if state.changing_rules || state.uncertain_rules {
            return Err(unavailable());
        }
        let rules = read_rules(&self.session)?;
        let expected_generation = match &change {
            NativePermissionRuleChange::Set { key, .. } => rules
                .rule_for_key(key)
                .map(crate::NativePermissionRule::generation),
            NativePermissionRuleChange::Revoke { id } => {
                Some(rules.rule_for_id(*id).ok_or_else(unavailable)?.generation())
            }
        };
        let proposal = NativePermissionRuleProposal {
            owner: Arc::downgrade(self),
            change,
            expected_generation,
        };
        proposal.apply(&rules)?;
        Ok(proposal)
    }

    /// Publishes one separately human-confirmed proposal through core's CAS.
    /// No work occurs before polling. Concurrent unrelated metadata/rule edits
    /// are preserved; changes to the proposed rule reject stale confirmation.
    /// A failed/dropped save leaves authority blocked until reconciliation.
    #[must_use]
    pub fn confirm_rule_change(
        self: &Arc<Self>,
        proposal: NativePermissionRuleProposal,
    ) -> BoxFuture<'_, Result<SessionRevision, PermissionError>> {
        Box::pin(async move {
            let owner = proposal.owner.upgrade().ok_or_else(unavailable)?;
            if !Arc::ptr_eq(self, &owner) {
                return Err(unavailable());
            }
            let mut operation = RuleOperation::begin(self)?;
            let revision = if let Some(editor) = operation.editor.clone() {
                let snapshot = editor.read_entry().await.map_err(|_| unavailable())?;
                let rules = decode(snapshot.entry())?;
                let candidate = proposal.apply(&rules)?;
                operation.arm()?;
                editor
                    .compare_exchange(snapshot, Some(candidate.to_value()))
                    .await
                    .map_err(|_| unavailable())?
            } else {
                let snapshot = self.session.record_snapshot();
                let rules = NativeSessionPermissionRules::from_metadata(&snapshot.metadata)
                    .map_err(|_| unavailable())?;
                let candidate = proposal.apply(&rules)?;
                let mut metadata = snapshot.metadata.clone();
                metadata.insert(
                    NATIVE_SESSION_PERMISSION_RULES_KEY.to_owned(),
                    candidate.to_value(),
                );
                operation.arm()?;
                self.session
                    .update_metadata(snapshot.revision, metadata)
                    .await
                    .map_err(|_| unavailable())?
            };
            operation.confirmed();
            Ok(revision)
        })
    }

    /// Reconciles an uncertain active-turn rule publication through its core
    /// editor. Idle owners reconcile by normal core reservation/load before
    /// calling this during the next actual turn; no direct store writer exists.
    #[must_use]
    pub fn reconcile_rules(&self) -> BoxFuture<'_, Result<(), PermissionError>> {
        Box::pin(async move {
            let editor = {
                let mut state = lock(&self.state);
                if state.changing_rules {
                    return Err(unavailable());
                }
                let editor = state
                    .active
                    .as_ref()
                    .ok_or_else(unavailable)?
                    .editor
                    .clone();
                state.changing_rules = true;
                editor
            };
            let mut operation = RuleOperation {
                owner: self,
                editor: Some(editor.clone()),
                armed: true,
            };
            let snapshot = editor.read_entry().await.map_err(|_| unavailable())?;
            decode(snapshot.entry())?;
            operation.confirmed();
            Ok(())
        })
    }
}

impl NativePermissionRuleProposal {
    fn apply(
        &self,
        rules: &NativeSessionPermissionRules,
    ) -> Result<NativeSessionPermissionRules, PermissionError> {
        match &self.change {
            NativePermissionRuleChange::Set {
                key,
                display_identity,
                decision,
            } => rules.apply_set(key, display_identity, *decision, self.expected_generation),
            NativePermissionRuleChange::Revoke { id } => {
                rules.apply_revoke(*id, self.expected_generation.ok_or_else(unavailable)?)
            }
        }
        .map_err(|_| unavailable())
    }
}

struct RuleOperation<'a> {
    owner: &'a NativePermissionSession,
    editor: Option<TurnMetadataEditor>,
    armed: bool,
}

impl<'a> RuleOperation<'a> {
    fn begin(owner: &'a NativePermissionSession) -> Result<Self, PermissionError> {
        let mut state = lock(&owner.state);
        if state.changing_rules || state.uncertain_rules {
            return Err(unavailable());
        }
        let editor = state.active.as_ref().map(|attempt| attempt.editor.clone());
        state.changing_rules = true;
        Ok(Self {
            owner,
            editor,
            armed: false,
        })
    }

    fn arm(&mut self) -> Result<(), PermissionError> {
        self.armed = true;
        let mut state = lock(&self.owner.state);
        state.uncertain_rules = true;
        state.rules_epoch = state.rules_epoch.checked_add(1).ok_or_else(unavailable)?;
        Ok(())
    }

    fn confirmed(&mut self) {
        lock(&self.owner.state).uncertain_rules = false;
        self.armed = false;
    }
}

impl Drop for RuleOperation<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.owner.state);
        state.changing_rules = false;
        if self.armed {
            state.uncertain_rules = true;
        }
    }
}

fn decode(
    value: Option<&serde_json::Value>,
) -> Result<NativeSessionPermissionRules, PermissionError> {
    value
        .map_or_else(
            || Ok(NativeSessionPermissionRules::default()),
            NativeSessionPermissionRules::from_value,
        )
        .map_err(|_| unavailable())
}
