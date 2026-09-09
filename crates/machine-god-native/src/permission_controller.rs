//! Native permission ownership. Preparation adapters retain effect authority;
//! this owner retains policy, prompt generations, grants and publication state.

#[cfg(test)]
mod lifecycle_tests;
mod rules;

use crate::conversation_lifecycle::{LifecycleGate, LifecyclePermit, LifecyclePhase};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use machine_god_core::{
    BoxFuture, CancellationToken, PermissionAuthorization, PermissionDecision, PermissionError,
    PermissionExecutionAdmission, PermissionGrantScope, PermissionHandler, PermissionInvocation,
    PermissionRequest, Session, Turn, TurnHandle, TurnId, TurnMetadataEditor,
};

use crate::{
    NATIVE_SESSION_PERMISSION_RULES_KEY, NativeConfiguredPermissionRules,
    NativePermissionRuleDecision, NativePermissionRuleKey, NativeSandboxMode,
    NativeSessionPermissionRules, PermissionMode, PermissionPromptDecision, PermissionPrompter,
};

pub use rules::{NativePermissionRuleChange, NativePermissionRuleProposal};

const MAX_SESSIONS: usize = 64;
const MAX_GRANTS: usize = 1024;
const MAX_CONTROL_OPERATIONS: usize = 256;

/// Combined result after a preparer evaluates every actual target. A deny on
/// any target wins; `Allow` requires all targets to have an explicit allow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePermissionConfiguredOutcome {
    Deny,
    Ask,
    Allow,
    Unresolved,
}

/// Auto uncertainty is a recoverable denial, not authority to open a prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePermissionAutomaticOutcome {
    Allow,
    Ask,
}

/// Explicitly injected native preparation authority. Constructors and futures
/// must remain inert before polling. Implementations validate the actual
/// invocation and retain all descriptors/evidence needed by their final effect.
pub trait NativePermissionActionPreparer: Send + Sync + 'static {
    /// Releases only the closed turn's admitted-but-unclaimed effect proofs.
    /// Implementations with retained registries override this bounded cleanup;
    /// non-retaining adapters need no additional cleanup.
    fn close_turn(
        &self,
        _session: &machine_god_core::SessionId,
        _incarnation: &machine_god_core::SessionIncarnationId,
        _turn: &TurnId,
    ) {
    }

    fn prepare<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>>;
}

/// Owned prepared action. This is an authority boundary, not model-supplied
/// policy: implementations belong in trusted native adapters. A file adapter
/// must bind the proof to its final publication check, not only tool entry.
pub trait NativePreparedPermissionAction: Send + Sync + 'static {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey>;
    fn grant_key(&self) -> Option<&NativePermissionRuleKey>;
    fn is_file_mutation(&self) -> bool;
    /// # Errors
    /// Rejects invalid targets or exhausted bounded matching work.
    fn configured_outcome(
        &self,
        rules: &NativeConfiguredPermissionRules,
    ) -> Result<NativePermissionConfiguredOutcome, PermissionError>;
    /// Only a tool-specific proven bypass; a low risk hint is not sufficient.
    fn allows_without_review(&self, mode: PermissionMode) -> bool;
    fn automatic_review(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionAutomaticOutcome, PermissionError>>;
    /// Retain/revalidate `proof` through the adapter's final effect boundary.
    /// # Errors
    /// Rejects stale, missing or unbindable native preparation authority.
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> Result<Box<dyn PermissionExecutionAdmission>, PermissionError>;
}

/// A taken job's immutable mode/configured-pattern selection. Saved rules and
/// resettable grants deliberately do not belong to this snapshot.
#[derive(Clone, Debug)]
pub struct NativePermissionPolicySnapshot {
    mode: PermissionMode,
    configured: Arc<NativeConfiguredPermissionRules>,
    sandbox_mode: NativeSandboxMode,
}

impl NativePermissionPolicySnapshot {
    #[must_use]
    pub fn new(mode: PermissionMode, configured: Arc<NativeConfiguredPermissionRules>) -> Self {
        Self {
            mode,
            configured,
            sandbox_mode: NativeSandboxMode::default(),
        }
    }

    #[must_use]
    pub const fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// The immutable configured-pattern selection captured for this policy.
    #[must_use]
    pub fn configured_rules(&self) -> &NativeConfiguredPermissionRules {
        &self.configured
    }

    /// Captures the configured preference; construction grants no OS authority.
    #[must_use]
    pub const fn with_sandbox_mode(mut self, sandbox_mode: NativeSandboxMode) -> Self {
        self.sandbox_mode = sandbox_mode;
        self
    }

    #[must_use]
    pub const fn sandbox_mode(&self) -> NativeSandboxMode {
        self.sandbox_mode
    }

    /// Taken Yolo jobs bypass the OS sandbox without changing the preference.
    /// Actual enforcement still requires a native launch authority.
    #[must_use]
    pub const fn effective_sandbox_mode(&self) -> NativeSandboxMode {
        match self.mode {
            PermissionMode::Yolo => NativeSandboxMode::None,
            PermissionMode::Ask | PermissionMode::Auto => self.sandbox_mode,
        }
    }
}

/// Shared handler with weak, exact-incarnation routes. A live session owner is
/// required; the table never restores grants from durable metadata.
pub struct NativePermissionController {
    routes: Arc<Mutex<Vec<Weak<NativePermissionSession>>>>,
    preparer: Arc<dyn NativePermissionActionPreparer>,
    prompter: Arc<dyn PermissionPrompter>,
}

impl NativePermissionController {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn has_live_grant(
        &self,
        session: &machine_god_core::SessionId,
        incarnation: &machine_god_core::SessionIncarnationId,
        turn: &TurnId,
        key: &NativePermissionRuleKey,
    ) -> Result<bool, PermissionError> {
        let owner = lock(&self.routes)
            .iter()
            .filter_map(Weak::upgrade)
            .find(|owner| {
                &owner.session.id() == session && &owner.session.incarnation_id() == incarnation
            })
            .ok_or_else(unavailable)?;
        let state = lock(&owner.state);
        if state.retired
            || state.changing_rules
            || state.uncertain_rules
            || state.active.as_ref().is_none_or(|attempt| {
                attempt.handle.id() != turn
                    || attempt.handle.is_cancelled()
                    || !attempt.editor.is_active()
            })
        {
            return Err(unavailable());
        }
        Ok(state.grants.iter().any(|grant| {
            grant.key == *key && grant.turn.as_ref().is_none_or(|granted| granted == turn)
        }))
    }

    #[must_use]
    pub fn new(
        preparer: Arc<dyn NativePermissionActionPreparer>,
        prompter: Arc<dyn PermissionPrompter>,
    ) -> Self {
        Self {
            routes: Arc::new(Mutex::new(Vec::new())),
            preparer,
            prompter,
        }
    }

    /// Registers one exact session lifetime without effects or restored grants.
    /// # Errors
    /// Rejects duplicate/capacity-exhausted routing and malformed saved rules.
    pub fn register(
        &self,
        session: Session,
        policy: NativePermissionPolicySnapshot,
    ) -> Result<Arc<NativePermissionSession>, PermissionError> {
        read_rules(&session)?;
        let mut routes = lock(&self.routes);
        routes.retain(|route| route.strong_count() != 0);
        if routes.len() == MAX_SESSIONS
            || routes.iter().filter_map(Weak::upgrade).any(|route| {
                route.session.id() == session.id()
                    && route.session.incarnation_id() == session.incarnation_id()
            })
        {
            return Err(unavailable());
        }
        let owner = Arc::new(NativePermissionSession {
            session,
            preparer: Arc::clone(&self.preparer),
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            routes: Arc::downgrade(&self.routes),
            lifecycle: OnceLock::new(),
            state: Arc::new(Mutex::new(State {
                controls: 0,
                retired: false,
                policy,
                epoch: 1,
                rules_epoch: 1,
                active: None,
                grants: Vec::new(),
                changing_rules: false,
                uncertain_rules: false,
            })),
        });
        routes.push(Arc::downgrade(&owner));
        Ok(owner)
    }

    fn route(
        &self,
        request: &PermissionRequest,
    ) -> Result<Arc<NativePermissionSession>, PermissionError> {
        lock(&self.routes)
            .iter()
            .filter_map(Weak::upgrade)
            .find(|owner| {
                owner.session.id() == request.session_id
                    && owner.session.incarnation_id() == request.session_incarnation_id
            })
            .ok_or_else(unavailable)
    }

    /// Observes the policy captured by one still-live exact tool turn.
    ///
    /// Native execution adapters use this to select a taken job's sandbox,
    /// never the mutable selection for future jobs. This bounded, effect-free
    /// observation is not an execution grant and does not revive a closed turn.
    /// # Errors
    /// Rejects unknown session incarnations, cancelled/closed turns and uncertain
    /// rule publication. Call IDs do not substitute for those ownership checks.
    pub fn policy_for_execution(
        &self,
        context: &machine_god_core::ToolContext,
    ) -> Result<NativePermissionPolicySnapshot, PermissionError> {
        let owner = lock(&self.routes)
            .iter()
            .filter_map(Weak::upgrade)
            .find(|owner| {
                owner.session.id() == context.session_id
                    && owner.session.incarnation_id() == context.session_incarnation_id
            })
            .ok_or_else(unavailable)?;
        let (attempt, _, _) = owner.attempt(&context.turn_id)?;
        Ok(attempt.policy.clone())
    }
}

impl fmt::Debug for NativePermissionController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionController { .. }")
    }
}

struct State {
    controls: usize,
    retired: bool,
    policy: NativePermissionPolicySnapshot,
    epoch: u64,
    rules_epoch: u64,
    active: Option<Arc<Attempt>>,
    grants: Vec<Grant>,
    changing_rules: bool,
    uncertain_rules: bool,
}

struct Attempt {
    handle: TurnHandle,
    editor: TurnMetadataEditor,
    policy: NativePermissionPolicySnapshot,
    cancellation: CancellationToken,
}

struct Grant {
    key: NativePermissionRuleKey,
    turn: Option<TurnId>,
}

/// Session-local grant/rule owner. Cloning an Arc keeps ownership, not an active
/// core turn lease. Direct core edits remain visible to each final rule check.
pub struct NativePermissionSession {
    session: Session,
    preparer: Arc<dyn NativePermissionActionPreparer>,
    state: Arc<Mutex<State>>,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    routes: Weak<Mutex<Vec<Weak<Self>>>>,
    lifecycle: OnceLock<Arc<LifecycleGate>>,
}

impl fmt::Debug for NativePermissionSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionSession { .. }")
    }
}

impl NativePermissionSession {
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn bind_lifecycle(&self, gate: &Arc<LifecycleGate>) -> Result<(), PermissionError> {
        let _permit = gate.acquire().map_err(|_| unavailable())?;
        let state = lock(&self.state);
        if state.retired || state.active.is_some() || state.changing_rules || state.controls != 0 {
            return Err(unavailable());
        }
        if let Some(current) = self.lifecycle.get() {
            return if Arc::ptr_eq(current, gate) {
                Ok(())
            } else {
                Err(unavailable())
            };
        }
        self.lifecycle
            .set(Arc::clone(gate))
            .map_err(|_| unavailable())
    }

    fn acquire_lifecycle(&self) -> Result<ControlPermit, PermissionError> {
        let mut state = lock(&self.state);
        if state.retired || state.controls == MAX_CONTROL_OPERATIONS {
            return Err(unavailable());
        }
        let lifecycle = self
            .lifecycle
            .get()
            .map(|gate| gate.acquire().map_err(|_| unavailable()))
            .transpose()?;
        state.controls += 1;
        Ok(ControlPermit {
            state: Arc::clone(&self.state),
            _lifecycle: lifecycle,
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    fn check_admitted(&self, permit: &LifecyclePermit) -> Result<(), PermissionError> {
        if lock(&self.state).retired
            || self
                .lifecycle
                .get()
                .is_none_or(|gate| !permit.belongs_to(gate))
        {
            return Err(unavailable());
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn retire(self: &Arc<Self>) {
        let attempt = {
            let mut state = lock(&self.state);
            state.retired = true;
            state.grants.clear();
            state.active.take()
        };
        if let Some(routes) = self.routes.upgrade() {
            lock(&routes).retain(|route| !Weak::ptr_eq(route, &Arc::downgrade(self)));
        }
        if let Some(attempt) = attempt {
            attempt.cancellation.cancel();
            self.preparer.close_turn(
                &self.session.id(),
                &self.session.incarnation_id(),
                attempt.handle.id(),
            );
        }
    }

    /// # Errors
    /// Rejects quiescing or retired lifecycle ownership.
    pub fn snapshot(&self) -> Result<NativePermissionPolicySnapshot, PermissionError> {
        let _permit = self.acquire_lifecycle()?;
        let snapshot = lock(&self.state).policy.clone();
        Ok(snapshot)
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn snapshot_admitted(
        &self,
        permit: &LifecyclePermit,
    ) -> Result<NativePermissionPolicySnapshot, PermissionError> {
        self.check_admitted(permit)?;
        Ok(lock(&self.state).policy.clone())
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn snapshot_quiescent(
        &self,
        quiescence: &crate::conversation_lifecycle::LifecycleQuiescence,
    ) -> Result<NativePermissionPolicySnapshot, PermissionError> {
        let gate = self.lifecycle.get().ok_or_else(unavailable)?;
        if !quiescence.belongs_to(gate) {
            return Err(unavailable());
        }
        quiescence.check_idle().map_err(|_| unavailable())?;
        let state = lock(&self.state);
        if state.retired {
            return Err(unavailable());
        }
        Ok(state.policy.clone())
    }

    /// Changes future taken jobs only. This is not a sandbox backend switch.
    /// # Errors
    /// Rejects quiescing or retired lifecycle ownership.
    pub fn set_mode(&self, mode: PermissionMode) -> Result<(), PermissionError> {
        let _permit = self.acquire_lifecycle()?;
        lock(&self.state).policy.mode = mode;
        Ok(())
    }

    /// Changes future taken jobs only; no running launch is silently widened.
    /// # Errors
    /// Rejects quiescing or retired lifecycle ownership.
    pub fn set_sandbox_mode(&self, mode: NativeSandboxMode) -> Result<(), PermissionError> {
        let _permit = self.acquire_lifecycle()?;
        lock(&self.state).policy.sandbox_mode = mode;
        Ok(())
    }

    /// Changes configured patterns for future taken jobs only. Mode, sandbox,
    /// saved exact rules and live exact grants are not replaced or revoked.
    /// # Errors
    /// Rejects quiescing or retired lifecycle ownership.
    pub fn set_configured_rules(
        &self,
        rules: Arc<NativeConfiguredPermissionRules>,
    ) -> Result<(), PermissionError> {
        let _permit = self.acquire_lifecycle()?;
        lock(&self.state).policy.configured = rules;
        Ok(())
    }

    #[cfg(all(
        feature = "ai-gateway-http",
        any(target_os = "linux", target_os = "macos", test)
    ))]
    pub(crate) fn set_configured_rules_admitted(
        &self,
        permit: &LifecyclePermit,
        rules: Arc<NativeConfiguredPermissionRules>,
    ) -> Result<(), PermissionError> {
        self.check_admitted(permit)?;
        lock(&self.state).policy.configured = rules;
        Ok(())
    }

    /// Clears grants and invalidates pending approvals before returning. Saved
    /// exact rules survive. Already taken jobs retain their captured mode.
    /// # Errors
    /// Refuses epoch wrap rather than admitting stale authority.
    pub fn reset(&self) -> Result<(), PermissionError> {
        let _permit = self.acquire_lifecycle()?;
        let mut state = lock(&self.state);
        state.epoch = state.epoch.checked_add(1).ok_or_else(unavailable)?;
        state.grants.clear();
        state.policy.mode = PermissionMode::Ask;
        Ok(())
    }

    /// Binds an actual reserved core turn before its first poll. Capture the
    /// snapshot when taking the queued job, before awaiting reservation.
    /// # Errors
    /// Rejects duplicate live turns and failed metadata-editor construction.
    pub fn begin_turn(
        self: &Arc<Self>,
        turn: &Turn,
        policy: NativePermissionPolicySnapshot,
    ) -> Result<NativePermissionTurn, PermissionError> {
        let permit = self.acquire_lifecycle()?;
        let mut registration = self.begin_turn_inner(turn, policy)?;
        registration.lifecycle = Some(permit);
        Ok(registration)
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn begin_turn_admitted(
        self: &Arc<Self>,
        turn: &Turn,
        policy: NativePermissionPolicySnapshot,
        permit: &LifecyclePermit,
    ) -> Result<NativePermissionTurn, PermissionError> {
        self.check_admitted(permit)?;
        self.begin_turn_inner(turn, policy)
    }

    fn begin_turn_inner(
        self: &Arc<Self>,
        turn: &Turn,
        policy: NativePermissionPolicySnapshot,
    ) -> Result<NativePermissionTurn, PermissionError> {
        if turn.session_id() != &self.session.id()
            || turn.session_incarnation_id() != &self.session.incarnation_id()
        {
            return Err(unavailable());
        }
        let editor = turn
            .metadata_editor(NATIVE_SESSION_PERMISSION_RULES_KEY)
            .map_err(|_| unavailable())?;
        let attempt = Arc::new(Attempt {
            handle: turn.handle(),
            editor,
            policy,
            cancellation: CancellationToken::new(),
        });
        let mut state = lock(&self.state);
        if state.retired || state.active.is_some() || !self.session.has_active_turn() {
            return Err(unavailable());
        }
        state.active = Some(Arc::clone(&attempt));
        Ok(NativePermissionTurn {
            owner: Arc::clone(self),
            attempt,
            lifecycle: None,
        })
    }

    fn attempt(&self, turn: &TurnId) -> Result<(Arc<Attempt>, u64, u64), PermissionError> {
        let state = lock(&self.state);
        if state.retired
            || self
                .lifecycle
                .get()
                .is_some_and(|gate| gate.phase() == LifecyclePhase::Retired)
        {
            return Err(unavailable());
        }
        let attempt = state
            .active
            .as_ref()
            .filter(|attempt| {
                attempt.handle.id() == turn
                    && !attempt.handle.is_cancelled()
                    && attempt.editor.is_active()
            })
            .ok_or_else(unavailable)?;
        if state.changing_rules || state.uncertain_rules {
            return Err(unavailable());
        }
        Ok((Arc::clone(attempt), state.epoch, state.rules_epoch))
    }
}

/// Exact-turn RAII registration. Closing it cannot clear a newer turn's grants.
pub struct NativePermissionTurn {
    owner: Arc<NativePermissionSession>,
    attempt: Arc<Attempt>,
    lifecycle: Option<ControlPermit>,
}

// Tracks even standalone control admission so attaching the runtime gate cannot
// race a previously unbound operation. Gate release/wakers follow state cleanup.
struct ControlPermit {
    state: Arc<Mutex<State>>,
    _lifecycle: Option<LifecyclePermit>,
}
impl Drop for ControlPermit {
    fn drop(&mut self) {
        lock(&self.state).controls -= 1;
    }
}

impl fmt::Debug for NativePermissionTurn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionTurn { .. }")
    }
}

impl Drop for NativePermissionTurn {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.owner.state);
            if state
                .active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &self.attempt))
            {
                state.active = None;
                state
                    .grants
                    .retain(|grant| grant.turn.as_ref() != Some(self.attempt.handle.id()));
            }
        }
        // Cancellation can invoke external wakers; never do it under the lock.
        self.attempt.cancellation.cancel();
        self.owner.preparer.close_turn(
            &self.owner.session.id(),
            &self.owner.session.incarnation_id(),
            self.attempt.handle.id(),
        );
        drop(self.lifecycle.take());
    }
}

/// Revocable native proof. File adapters retain it through their final effect
/// checkpoint; other adapters may consume it directly at core tool admission.
pub struct NativePermissionExecutionProof {
    owner: Weak<NativePermissionSession>,
    attempt: Weak<Attempt>,
    epoch: Option<u64>,
    rules_epoch: Option<u64>,
    saved: Option<NativePermissionRuleKey>,
    saved_generation: Option<u64>,
}

impl fmt::Debug for NativePermissionExecutionProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionExecutionProof { .. }")
    }
}

impl NativePermissionExecutionProof {
    /// Bounded synchronous final policy check; it performs no prompt or I/O.
    /// # Errors
    /// Rejects closed turns, reset authority and changed applicable saved rules.
    pub fn revalidate(&self) -> Result<(), PermissionError> {
        let owner = self.owner.upgrade().ok_or_else(unavailable)?;
        let attempt = self.attempt.upgrade().ok_or_else(unavailable)?;
        let (active, epoch, rules_epoch) = owner.attempt(attempt.handle.id())?;
        if !Arc::ptr_eq(&attempt, &active)
            || self.epoch.is_some_and(|expected| epoch != expected)
            || self
                .rules_epoch
                .is_some_and(|expected| rules_epoch != expected)
        {
            return Err(unavailable());
        }
        if let Some(key) = &self.saved {
            let rules = read_rules(&owner.session)?;
            let current = rules.rule_for_key(key);
            if current.is_some_and(|rule| rule.decision() == NativePermissionRuleDecision::Deny)
                || current.map(crate::NativePermissionRule::generation) != self.saved_generation
            {
                return Err(unavailable());
            }
        }
        Ok(())
    }
}

impl PermissionExecutionAdmission for NativePermissionExecutionProof {
    fn admit(self: Box<Self>) -> Result<(), PermissionError> {
        self.revalidate()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl crate::NativeFileApprovalPolicy for NativePermissionExecutionProof {
    fn revalidate(&self) -> Result<(), PermissionError> {
        NativePermissionExecutionProof::revalidate(self)
    }
}

impl PermissionHandler for NativePermissionController {
    fn authorize(
        &self,
        _request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async { Err(unavailable()) })
    }

    fn authorize_invocation<'a>(
        &'a self,
        request: PermissionRequest,
        invocation: PermissionInvocation<'a>,
    ) -> BoxFuture<'a, Result<PermissionAuthorization, PermissionError>> {
        Box::pin(async move {
            let owner = self.route(&request)?;
            let (attempt, epoch, rules_epoch) = owner.attempt(&request.turn_id)?;
            let Ok(action) = self
                .preparer
                .prepare(&request, invocation, attempt.cancellation.clone())
                .await
            else {
                // An unavailable selected target is an unexecutable action,
                // not a failed permission service. Keep the model's bounded
                // replanning path without prompting or fabricating authority.
                return Ok(denied("native action preparation requires replanning"));
            };
            let rules = read_rules(&owner.session)?;
            let bypass = attempt.policy.mode == PermissionMode::Yolo;
            let saved = if bypass && !action.is_file_mutation() {
                None
            } else {
                action.saved_rule_key().cloned()
            };
            let saved_rule = saved.as_ref().and_then(|key| rules.rule_for_key(key));
            let configured = if bypass {
                NativePermissionConfiguredOutcome::Unresolved
            } else {
                action.configured_outcome(&attempt.policy.configured)?
            };
            if configured == NativePermissionConfiguredOutcome::Deny
                || saved_rule
                    .is_some_and(|rule| rule.decision() == NativePermissionRuleDecision::Deny)
            {
                return Ok(denied("permission denied by rule"));
            }
            let proof = NativePermissionExecutionProof {
                owner: Arc::downgrade(&owner),
                attempt: Arc::downgrade(&attempt),
                epoch: (!bypass).then_some(epoch),
                rules_epoch: (!bypass || action.is_file_mutation()).then_some(rules_epoch),
                saved,
                saved_generation: saved_rule.map(crate::NativePermissionRule::generation),
            };
            let granted = action.grant_key().is_some_and(|key| {
                lock(&owner.state).grants.iter().any(|grant| {
                    grant.key == *key
                        && grant
                            .turn
                            .as_ref()
                            .is_none_or(|turn| turn == &request.turn_id)
                })
            });
            let saved_allow = saved_rule
                .is_some_and(|rule| rule.decision() == NativePermissionRuleDecision::Allow);
            if bypass
                || saved_allow
                || granted
                || (configured != NativePermissionConfiguredOutcome::Ask
                    && (configured == NativePermissionConfiguredOutcome::Allow
                        || action.allows_without_review(attempt.policy.mode)))
            {
                return admit_action(action, proof);
            }
            if attempt.policy.mode == PermissionMode::Auto
                && configured != NativePermissionConfiguredOutcome::Ask
            {
                return match action.automatic_review(attempt.cancellation.clone()).await {
                    Ok(NativePermissionAutomaticOutcome::Allow) => admit_action(action, proof),
                    Ok(NativePermissionAutomaticOutcome::Ask) | Err(_) => {
                        Ok(denied("automatic permission review requires replanning"))
                    }
                };
            }
            let decision = self
                .prompter
                .prompt(request)
                .await
                .map_err(|_| unavailable())?;
            proof.revalidate()?;
            if decision == PermissionPromptDecision::Deny {
                return Ok(denied("permission denied"));
            }
            remember_grant(&owner, &attempt, epoch, action.grant_key(), decision)?;
            admit_action(action, proof)
        })
    }
}

fn remember_grant(
    owner: &NativePermissionSession,
    attempt: &Attempt,
    epoch: u64,
    key: Option<&NativePermissionRuleKey>,
    decision: PermissionPromptDecision,
) -> Result<(), PermissionError> {
    let mut state = lock(&owner.state);
    if state.retired
        || state.epoch != epoch
        || state
            .active
            .as_ref()
            .is_none_or(|active| active.handle.id() != attempt.handle.id())
    {
        return Err(unavailable());
    }
    let Some(key) = key else {
        return Ok(());
    };
    let turn = match decision {
        PermissionPromptDecision::AllowTurn => Some(attempt.handle.id().clone()),
        PermissionPromptDecision::AllowSession => None,
        PermissionPromptDecision::AllowOnce | PermissionPromptDecision::Deny => return Ok(()),
    };
    if !state
        .grants
        .iter()
        .any(|grant| grant.key == *key && grant.turn == turn)
    {
        if state.grants.len() == MAX_GRANTS {
            return Err(unavailable());
        }
        state.grants.push(Grant {
            key: key.clone(),
            turn,
        });
    }
    Ok(())
}

fn admit_action(
    action: Box<dyn NativePreparedPermissionAction>,
    proof: NativePermissionExecutionProof,
) -> Result<PermissionAuthorization, PermissionError> {
    proof.revalidate()?;
    Ok(PermissionAuthorization {
        decision: PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        },
        admission: Some(action.bind_execution(proof)?),
    })
}

fn denied(reason: &str) -> PermissionAuthorization {
    PermissionAuthorization::new(PermissionDecision::Deny {
        reason: reason.to_owned(),
    })
}

fn read_rules(session: &Session) -> Result<NativeSessionPermissionRules, PermissionError> {
    NativeSessionPermissionRules::from_metadata(&session.record_snapshot().metadata)
        .map_err(|_| unavailable())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn unavailable() -> PermissionError {
    PermissionError::new(
        "native_permission_unavailable",
        "native permission authority unavailable",
    )
}
