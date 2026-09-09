use super::{NativePermissionSession, lock, read_rules, unavailable};
use crate::{
    NATIVE_SESSION_PERMISSION_RULES_KEY, NativePermissionRuleDecision, NativePermissionRuleKey,
    NativeSessionPermissionRules,
};
use machine_god_core::{BoxFuture, PermissionError, SessionRevision, TurnMetadataEditor};
use std::fmt;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

/// Native-prepared exact action associated with one still-pending prompt.
/// Hosts cannot construct this value or obtain an execution grant from it.
#[derive(Clone)]
pub struct NativePermissionRulePrompt {
    owner: Weak<NativePermissionSession>,
    attempt: Weak<super::Attempt>,
    epoch: u64,
    rules_epoch: u64,
    key: NativePermissionRuleKey,
    live: Arc<AtomicBool>,
}

impl fmt::Debug for NativePermissionRulePrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionRulePrompt { .. }")
    }
}

impl NativePermissionRulePrompt {
    pub(super) fn new(
        owner: &Arc<NativePermissionSession>,
        attempt: &Arc<super::Attempt>,
        epoch: u64,
        rules_epoch: u64,
        key: NativePermissionRuleKey,
    ) -> Self {
        Self {
            owner: Arc::downgrade(owner),
            attempt: Arc::downgrade(attempt),
            epoch,
            rules_epoch,
            key,
            live: Arc::new(AtomicBool::new(true)),
        }
    }

    pub(crate) fn invalidate(&self) {
        self.live.store(false, Ordering::Release);
    }

    fn validate(&self) -> Result<Arc<NativePermissionSession>, PermissionError> {
        if !self.live.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        let owner = self.owner.upgrade().ok_or_else(unavailable)?;
        let attempt = self.attempt.upgrade().ok_or_else(unavailable)?;
        let (current, epoch, rules_epoch) = owner.attempt(attempt.handle.id())?;
        if !Arc::ptr_eq(&current, &attempt)
            || epoch != self.epoch
            || rules_epoch != self.rules_epoch
        {
            return Err(unavailable());
        }
        Ok(owner)
    }

    fn validate_publication(&self, state: &super::State) -> Result<(), PermissionError> {
        let attempt = self.attempt.upgrade().ok_or_else(unavailable)?;
        if !self.live.load(Ordering::Acquire)
            || state.retired
            || state.epoch != self.epoch
            || state.rules_epoch != self.rules_epoch
            || state
                .active
                .as_ref()
                .is_none_or(|active| !Arc::ptr_eq(active, &attempt))
            || attempt.handle.is_cancelled()
            || !attempt.editor.is_active()
        {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(crate) fn propose(
        &self,
        decision: NativePermissionRuleDecision,
    ) -> Result<NativePermissionRuleProposal, PermissionError> {
        use std::fmt::Write;
        let owner = self.validate()?;
        let mut display_identity = format!("Exact prepared {:?} action sha256:", self.key.kind());
        for byte in self.key.digest() {
            write!(display_identity, "{byte:02x}").map_err(|_| unavailable())?;
        }
        let mut proposal = owner.propose_rule_change(NativePermissionRuleChange::Set {
            key: self.key.clone(),
            display_identity,
            decision,
        })?;
        proposal.prompt = Some(self.clone());
        Ok(proposal)
    }
}

pub(super) struct RulePromptLifetime(pub Option<NativePermissionRulePrompt>);
impl Drop for RulePromptLifetime {
    fn drop(&mut self) {
        if let Some(prompt) = &self.0 {
            prompt.invalidate();
        }
    }
}

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
    expected_epoch: u64,
    prompt: Option<NativePermissionRulePrompt>,
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
        let _permit = self.acquire_lifecycle()?;
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
            expected_epoch: state.epoch,
            prompt: None,
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
            if let Some(prompt) = &proposal.prompt {
                prompt.validate()?;
            }
            if lock(&self.state).epoch != proposal.expected_epoch {
                return Err(unavailable());
            }
            let mut operation = RuleOperation::begin(self)?;
            let revision = if let Some(editor) = operation.editor.clone() {
                let snapshot = editor.read_entry().await.map_err(|_| unavailable())?;
                let rules = decode(snapshot.entry())?;
                let candidate = proposal.apply(&rules)?;
                operation.arm(&proposal)?;
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
                operation.arm(&proposal)?;
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
            let permit = self.acquire_lifecycle()?;
            self.reconcile_rules_inner(Some(permit)).await
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) async fn reconcile_rules_admitted(
        &self,
        permit: &crate::conversation_lifecycle::LifecyclePermit,
    ) -> Result<(), PermissionError> {
        self.check_admitted(permit)?;
        self.reconcile_rules_inner(None).await
    }

    async fn reconcile_rules_inner(
        &self,
        permit: Option<super::ControlPermit>,
    ) -> Result<(), PermissionError> {
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
            _permit: permit,
        };
        let snapshot = editor.read_entry().await.map_err(|_| unavailable())?;
        decode(snapshot.entry())?;
        operation.confirmed();
        Ok(())
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
    _permit: Option<super::ControlPermit>,
}

impl<'a> RuleOperation<'a> {
    fn begin(owner: &'a NativePermissionSession) -> Result<Self, PermissionError> {
        let permit = owner.acquire_lifecycle()?;
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
            _permit: Some(permit),
        })
    }

    fn arm(&mut self, proposal: &NativePermissionRuleProposal) -> Result<(), PermissionError> {
        let mut state = lock(&self.owner.state);
        if state.epoch != proposal.expected_epoch {
            return Err(unavailable());
        }
        if let Some(prompt) = &proposal.prompt {
            prompt.validate_publication(&state)?;
        }
        self.armed = true;
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
