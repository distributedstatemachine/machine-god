//! Deterministic bounded terminal monitors and attention waits.
//!
//! Condition and schedule semantics follow pinned fx monitor.zig at
//! b1774fbf6c7602b503026f96f6e960e946c692ef. Native effects are explicit typed
//! probe requests; this module never reads a clock, path, socket, or process.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use machine_god_core::{
    TerminalCellKind, TerminalCursor, TerminalEventQuery, TerminalLifecycle,
    TerminalMonitorCondition as Condition, TerminalMonitorDefinition as Definition,
    TerminalMonitorEvent, TerminalMonitorEventReason as Reason, TerminalMonitorId,
    TerminalMonitorLifetime as Lifetime, TerminalMonitorOperation as Operation,
    TerminalMonitorState as State, TerminalNotifySchedule as Notify, TerminalReturnCondition,
    TerminalScreen, TerminalSessionId, TerminalSignal, TerminalWaitRequest,
};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_MONITORS: usize = 64;
pub(crate) const MAX_MONITOR_EVENTS: usize = 256;
pub(crate) const MAX_MONITOR_FEED_BYTES: usize = 16 * 1024;
pub(crate) const PROBE_TIMEOUT_MS: u64 = 2_000;
pub(crate) const PROBE_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_PATTERN_BYTES: usize = 256;
const WORDS: usize = 5;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalMonitorError {
    Invalid,
    NotFound,
    InvalidState,
    Capacity,
    Clock,
    Counter,
    Snapshot,
}
impl fmt::Display for TerminalMonitorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("terminal monitor state is unavailable")
    }
}
impl std::error::Error for TerminalMonitorError {}
type Result<T> = std::result::Result<T, TerminalMonitorError>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalMonitorContext {
    pub now_ms: i64,
    pub cursor: TerminalCursor,
    pub lifecycle: TerminalLifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum TerminalProcessOutcome {
    Exited(i32),
    Signaled(i32),
}
impl TerminalProcessOutcome {
    fn validate(self) -> Result<()> {
        require(match self {
            Self::Exited(code) => (0..=255).contains(&code),
            Self::Signaled(signal) => (1..=255).contains(&signal),
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalPathBaseline {
    pub exists: bool,
    pub size: u64,
    pub modified_ns: i128,
}
impl TerminalPathBaseline {
    fn validate(self) -> Result<()> {
        require(self.exists || (self.size == 0 && self.modified_ns == 0))
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TerminalMonitorActivation {
    pub path_baseline: Option<TerminalPathBaseline>,
    pub cwd_sha256: Option<[u8; 32]>,
}

#[derive(Clone)]
pub(crate) enum TerminalProbeTarget {
    Tcp {
        host: String,
        port: u16,
    },
    Http {
        url: String,
    },
    Path {
        path: String,
    },
    Custom {
        command: String,
        cwd: String,
        approved_cwd_sha256: [u8; 32],
    },
}

#[derive(Clone)]
pub(crate) struct TerminalProbeRequest {
    pub session_id: TerminalSessionId,
    pub monitor_id: TerminalMonitorId,
    pub generation: u64,
    pub request_sequence: u64,
    pub started_at_ms: i64,
    pub deadline_ms: i64,
    pub output_limit_bytes: usize,
    pub target: TerminalProbeTarget,
}

#[derive(Clone)]
pub(crate) enum TerminalProbeObservation {
    Tcp {
        connected: bool,
    },
    Http {
        response_prefix: Vec<u8>,
    },
    Path {
        baseline: TerminalPathBaseline,
    },
    Custom {
        exit_code: i32,
        cwd_sha256: [u8; 32],
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalProbeFailure {
    Unavailable,
    Denied,
    Timeout,
    OutputLimit,
    InvalidEvidence,
}
#[derive(Clone)]
pub(crate) struct TerminalProbeEvidence {
    pub session_id: TerminalSessionId,
    pub monitor_id: TerminalMonitorId,
    pub generation: u64,
    pub request_sequence: u64,
    pub completed_at_ms: i64,
    pub output_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
    pub result: std::result::Result<TerminalProbeObservation, TerminalProbeFailure>,
}

#[derive(Clone)]
pub(crate) struct TerminalMonitorMutation {
    pub monitor_id: TerminalMonitorId,
    pub removed: bool,
    /// Exact post-mutation generation, or the retired generation for removal.
    pub generation: u64,
}

/// Bounded observation-only projection. It contains no probe approval or
/// activation fingerprints, pending effect requests, or execution handles.
#[cfg(test)]
#[derive(Clone, Serialize)]
pub(crate) struct TerminalMonitorView {
    pub monitor_id: TerminalMonitorId,
    pub generation: u64,
    pub definition: Definition,
    pub state: State,
    pub created_at_ms: i64,
    pub lifetime_deadline_ms: Option<i64>,
    pub next_check_ms: Option<i64>,
    pub next_notification_ms: Option<i64>,
    pub check_count: u64,
    pub notification_count: u64,
    pub last_event_id: u64,
    pub last_event_reason: Option<Reason>,
    pub condition_matched: bool,
}

#[cfg(test)]
impl fmt::Debug for TerminalMonitorView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TerminalMonitorView { .. }")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingProbe {
    sequence: u64,
    started_at_ms: i64,
    deadline_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Runtime {
    state: State,
    generation: u64,
    created_at_ms: i64,
    lifetime_deadline_ms: Option<i64>,
    next_check_ms: Option<i64>,
    next_notification_ms: Option<i64>,
    check_count: u64,
    notification_count: u64,
    last_event_id: u64,
    last_event_reason: Option<Reason>,
    condition_matched: bool,
    matcher_states: [u64; WORDS],
    path_baseline: Option<TerminalPathBaseline>,
    cwd_sha256: Option<[u8; 32]>,
    pending: Option<PendingProbe>,
}

#[derive(Clone)]
struct Monitor {
    id: TerminalMonitorId,
    definition: Arc<Definition>,
    runtime: Runtime,
    pattern: Option<Arc<Pattern>>,
}

/// Clone-on-transition makes counter/deadline/validation failures atomic.
/// Definitions and compiled matchers are shared; output and screens are never retained.
#[derive(Clone)]
pub(crate) struct TerminalMonitorSet {
    session_id: TerminalSessionId,
    context: TerminalMonitorContext,
    next_monitor_id: u64,
    next_event_id: u64,
    next_probe_sequence: u64,
    acknowledged_event_id: u64,
    dropped_through_event_id: u64,
    monitors: Vec<Monitor>,
    events: VecDeque<TerminalMonitorEvent>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedMonitor {
    id: TerminalMonitorId,
    definition: Definition,
    runtime: Runtime,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u16,
    session_id: TerminalSessionId,
    context: TerminalMonitorContext,
    next_monitor_id: u64,
    next_event_id: u64,
    next_probe_sequence: u64,
    acknowledged_event_id: u64,
    dropped_through_event_id: u64,
    #[serde(deserialize_with = "bounded_vec::<_, SavedMonitor, MAX_MONITORS>")]
    monitors: Vec<SavedMonitor>,
    #[serde(deserialize_with = "bounded_vec::<_, TerminalMonitorEvent, MAX_MONITOR_EVENTS>")]
    events: Vec<TerminalMonitorEvent>,
}

impl TerminalMonitorSet {
    pub(crate) fn live_generations(&self) -> Vec<(TerminalMonitorId, u64)> {
        self.monitors
            .iter()
            .map(|monitor| (monitor.id.clone(), monitor.runtime.generation))
            .collect()
    }

    pub(crate) fn session_id(&self) -> &TerminalSessionId {
        &self.session_id
    }

    pub(crate) fn context(&self) -> &TerminalMonitorContext {
        &self.context
    }

    /// Advance only the observation boundary; no timer, matcher, or probe effects.
    pub(crate) fn checkpoint_context(&mut self, context: TerminalMonitorContext) -> Result<()> {
        self.transition(context, |_| Ok(()))
    }

    pub(crate) fn new(
        session_id: TerminalSessionId,
        context: TerminalMonitorContext,
    ) -> Result<Self> {
        validate_context(&context)?;
        Ok(Self {
            session_id,
            context,
            next_monitor_id: 1,
            next_event_id: 1,
            next_probe_sequence: 1,
            acknowledged_event_id: 0,
            dropped_through_event_id: 0,
            monitors: Vec::new(),
            events: VecDeque::new(),
        })
    }

    fn transition<T>(
        &mut self,
        context: TerminalMonitorContext,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        validate_context(&context)?;
        if context.now_ms < self.context.now_ms || context.cursor < self.context.cursor {
            return Err(TerminalMonitorError::Clock);
        }
        if matches!(
            self.context.lifecycle,
            TerminalLifecycle::Exited | TerminalLifecycle::Lost | TerminalLifecycle::Closed
        ) && matches!(
            context.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            return Err(TerminalMonitorError::InvalidState);
        }
        let mut candidate = self.clone();
        candidate.context = context;
        let result = operation(&mut candidate)?;
        *self = candidate;
        Ok(result)
    }

    #[cfg(test)]
    pub(crate) fn apply(
        &mut self,
        operation: Operation,
        context: TerminalMonitorContext,
    ) -> Result<TerminalMonitorMutation> {
        self.apply_with_activation(operation, TerminalMonitorActivation::default(), context)
    }

    pub(crate) fn apply_with_activation(
        &mut self,
        operation: Operation,
        activation: TerminalMonitorActivation,
        context: TerminalMonitorContext,
    ) -> Result<TerminalMonitorMutation> {
        operation
            .validate()
            .map_err(|_| TerminalMonitorError::Invalid)?;
        self.transition(context, move |set| set.apply_inner(operation, activation))
    }

    fn apply_inner(
        &mut self,
        operation: Operation,
        activation: TerminalMonitorActivation,
    ) -> Result<TerminalMonitorMutation> {
        let now = self.context.now_ms;
        match operation {
            Operation::Add { definition } => {
                if self.monitors.len() >= MAX_MONITORS {
                    return Err(TerminalMonitorError::Capacity);
                }
                if !matches!(
                    self.context.lifecycle,
                    TerminalLifecycle::Starting | TerminalLifecycle::Running
                ) {
                    return Err(TerminalMonitorError::InvalidState);
                }
                let id = stable_id(self.next_monitor_id)?;
                let monitor = Monitor::new(id.clone(), definition, now, activation)?;
                self.next_monitor_id = increment(self.next_monitor_id)?;
                let generation = monitor.runtime.generation;
                self.monitors.push(monitor);
                Ok(TerminalMonitorMutation {
                    monitor_id: id,
                    removed: false,
                    generation,
                })
            }
            Operation::Update {
                monitor_id,
                definition,
            } => {
                let index = self.index(&monitor_id)?;
                let generation = increment(self.monitors[index].runtime.generation)?;
                let mut monitor = Monitor::new(monitor_id.clone(), definition, now, activation)?;
                monitor.runtime.generation = generation;
                self.monitors[index] = monitor;
                self.state_event(index, Reason::Updated)?;
                Ok(TerminalMonitorMutation {
                    monitor_id,
                    removed: false,
                    generation,
                })
            }
            Operation::Pause { monitor_id } => {
                require_empty_activation(&activation)?;
                let index = self.index(&monitor_id)?;
                let runtime = &mut self.monitors[index].runtime;
                if matches!(runtime.state, State::Paused | State::Degraded) {
                    return Err(TerminalMonitorError::InvalidState);
                }
                runtime.state = State::Paused;
                runtime.generation = increment(runtime.generation)?;
                runtime.pending = None;
                self.state_event(index, Reason::Paused)?;
                Ok(TerminalMonitorMutation {
                    monitor_id,
                    removed: false,
                    generation: self.monitors[index].runtime.generation,
                })
            }
            Operation::Resume { monitor_id } => {
                require_empty_activation(&activation)?;
                let index = self.index(&monitor_id)?;
                let monitor = &mut self.monitors[index];
                if monitor.runtime.state != State::Paused {
                    return Err(TerminalMonitorError::InvalidState);
                }
                monitor.runtime.state = if monitor.runtime.condition_matched {
                    State::Matched
                } else {
                    State::Active
                };
                monitor.runtime.generation = increment(monitor.runtime.generation)?;
                monitor.runtime.pending = None;
                monitor.runtime.next_check_ms = check_deadline(&monitor.definition, now)?;
                monitor.runtime.next_notification_ms = notify_deadline(&monitor.definition, now)?;
                self.state_event(index, Reason::Resumed)?;
                Ok(TerminalMonitorMutation {
                    monitor_id,
                    removed: false,
                    generation: self.monitors[index].runtime.generation,
                })
            }
            Operation::Remove { monitor_id } => {
                require_empty_activation(&activation)?;
                let index = self.index(&monitor_id)?;
                self.state_event(index, Reason::Removed)?;
                let removed = self.monitors.remove(index);
                Ok(TerminalMonitorMutation {
                    monitor_id,
                    removed: true,
                    generation: removed.runtime.generation,
                })
            }
        }
    }

    fn index(&self, id: &TerminalMonitorId) -> Result<usize> {
        self.monitors
            .iter()
            .position(|monitor| monitor.id == *id)
            .ok_or(TerminalMonitorError::NotFound)
    }
    fn state_event(&mut self, index: usize, reason: Reason) -> Result<()> {
        if self.monitors[index].definition.notify == Notify::OnStateChange {
            self.emit(index, reason)?;
        }
        Ok(())
    }
    fn emit(&mut self, index: usize, reason: Reason) -> Result<()> {
        let event_id = self.next_event_id;
        self.next_event_id = increment(event_id)?;
        let monitor = &mut self.monitors[index];
        monitor.runtime.notification_count = increment(monitor.runtime.notification_count)?;
        monitor.runtime.last_event_id = event_id;
        monitor.runtime.last_event_reason = Some(reason);
        let event = TerminalMonitorEvent {
            event_id,
            monitor_id: monitor.id.clone(),
            reason,
            lifecycle: self.context.lifecycle,
            cursor: self.context.cursor.clone(),
            created_at_ms: self.context.now_ms,
        };
        if self.events.len() == MAX_MONITOR_EVENTS {
            self.dropped_through_event_id =
                self.events.pop_front().expect("full event queue").event_id;
        }
        self.events.push_back(event);
        Ok(())
    }

    fn observe(&mut self, index: usize, matched: bool, check: bool, quiet: bool) -> Result<bool> {
        if !self.monitors[index].enabled() {
            return Ok(false);
        }
        if self.expire(index)? {
            return Ok(true);
        }
        let monitor = &mut self.monitors[index];
        let runtime = &mut monitor.runtime;
        runtime.check_count = increment(runtime.check_count)?;
        if check {
            let interval = monitor
                .definition
                .check_schedule
                .as_ref()
                .ok_or(TerminalMonitorError::InvalidState)?
                .interval_ms;
            runtime.next_check_ms = Some(advance_deadline(
                runtime
                    .next_check_ms
                    .ok_or(TerminalMonitorError::InvalidState)?,
                interval,
                self.context.now_ms,
            )?);
        }
        if quiet {
            runtime.next_check_ms = check_deadline(&monitor.definition, self.context.now_ms)?;
        }
        let newly_matched = matched && !runtime.condition_matched;
        if newly_matched {
            runtime.condition_matched = true;
            runtime.state = State::Matched;
        }
        let reason = match monitor.definition.notify {
            Notify::OnMatch if newly_matched => Some(Reason::Matched),
            Notify::OnStateChange if newly_matched => Some(Reason::StateChanged),
            Notify::EveryCheck => Some(Reason::Check),
            Notify::EveryNChecks { count }
                if runtime.check_count.is_multiple_of(u64::from(count)) =>
            {
                Some(Reason::Check)
            }
            _ => None,
        };
        let remove = newly_matched && monitor.definition.lifetime == Lifetime::UntilMatch;
        if let Some(reason) = reason {
            self.emit(index, reason)?;
        }
        if remove {
            self.monitors.remove(index);
        }
        Ok(remove)
    }

    fn expire(&mut self, index: usize) -> Result<bool> {
        if self.monitors[index]
            .runtime
            .lifetime_deadline_ms
            .is_some_and(|due| self.context.now_ms >= due)
        {
            self.state_event(index, Reason::Expired)?;
            self.monitors.remove(index);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Monitor {
    fn new(
        id: TerminalMonitorId,
        definition: Definition,
        now: i64,
        activation: TerminalMonitorActivation,
    ) -> Result<Self> {
        validate_definition(&definition)?;
        let pattern = condition_pattern(&definition.condition)?;
        let runtime = Runtime {
            state: State::Active,
            generation: 1,
            created_at_ms: now,
            lifetime_deadline_ms: match definition.lifetime {
                Lifetime::Duration { duration_ms } => Some(deadline(now, duration_ms)?),
                _ => None,
            },
            next_check_ms: check_deadline(&definition, now)?,
            next_notification_ms: notify_deadline(&definition, now)?,
            check_count: 0,
            notification_count: 0,
            last_event_id: 0,
            last_event_reason: None,
            condition_matched: false,
            matcher_states: [0; WORDS],
            path_baseline: None,
            cwd_sha256: None,
            pending: None,
        };
        let mut monitor = Self {
            id,
            definition: Arc::new(definition),
            runtime,
            pattern,
        };
        install_activation(&mut monitor, &activation)?;
        Ok(monitor)
    }
    fn enabled(&self) -> bool {
        !matches!(self.runtime.state, State::Paused | State::Degraded)
    }
}

fn validate_definition(definition: &Definition) -> Result<()> {
    definition
        .validate()
        .map_err(|_| TerminalMonitorError::Invalid)?;
    if let Notify::EveryNChecks { count } = definition.notify {
        require(count <= 1_000_000)?;
    }
    if let Condition::OutputQuiet { duration_ms } = definition.condition {
        require(duration_ms >= 10)?;
    }
    Ok(())
}
fn validate_context(context: &TerminalMonitorContext) -> Result<()> {
    if context.now_ms < 0 {
        return Err(TerminalMonitorError::Clock);
    }
    context
        .cursor
        .validate()
        .map_err(|_| TerminalMonitorError::Invalid)
}
fn require(valid: bool) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(TerminalMonitorError::Invalid)
    }
}
fn increment(value: u64) -> Result<u64> {
    value.checked_add(1).ok_or(TerminalMonitorError::Counter)
}
fn deadline(now: i64, duration: u64) -> Result<i64> {
    if now < 0 {
        return Err(TerminalMonitorError::Clock);
    }
    now.checked_add(i64::try_from(duration).map_err(|_| TerminalMonitorError::Clock)?)
        .ok_or(TerminalMonitorError::Clock)
}
fn advance_deadline(current: i64, interval: u64, now: i64) -> Result<i64> {
    if current > now {
        return Ok(current);
    }
    let interval = i64::try_from(interval).map_err(|_| TerminalMonitorError::Clock)?;
    require(interval > 0 && current >= 0)?;
    let steps = (now - current)
        .checked_div(interval)
        .and_then(|value| value.checked_add(1))
        .ok_or(TerminalMonitorError::Clock)?;
    current
        .checked_add(
            steps
                .checked_mul(interval)
                .ok_or(TerminalMonitorError::Clock)?,
        )
        .ok_or(TerminalMonitorError::Clock)
}
fn check_deadline(definition: &Definition, now: i64) -> Result<Option<i64>> {
    if let Some(schedule) = &definition.check_schedule {
        return deadline(now, schedule.interval_ms).map(Some);
    }
    match definition.condition {
        Condition::OutputQuiet { duration_ms } => deadline(now, duration_ms).map(Some),
        _ => Ok(None),
    }
}
fn notify_deadline(definition: &Definition, now: i64) -> Result<Option<i64>> {
    match definition.notify {
        Notify::Interval { interval_ms } => deadline(now, interval_ms).map(Some),
        _ => Ok(None),
    }
}
fn stable_id(sequence: u64) -> Result<TerminalMonitorId> {
    require(sequence > 0)?;
    TerminalMonitorId::new(format!("monitor-{sequence}")).map_err(|_| TerminalMonitorError::Counter)
}
fn monitor_sequence(id: &TerminalMonitorId) -> Result<u64> {
    let number = id
        .as_str()
        .strip_prefix("monitor-")
        .ok_or(TerminalMonitorError::Invalid)?
        .parse::<u64>()
        .map_err(|_| TerminalMonitorError::Invalid)?;
    require(stable_id(number)? == *id)?;
    Ok(number)
}
fn require_empty_activation(activation: &TerminalMonitorActivation) -> Result<()> {
    require(activation.path_baseline.is_none() && activation.cwd_sha256.is_none())
}
fn install_activation(monitor: &mut Monitor, activation: &TerminalMonitorActivation) -> Result<()> {
    match monitor.definition.condition {
        Condition::PathChanged { .. } => {
            require(activation.cwd_sha256.is_none())?;
            let baseline = activation
                .path_baseline
                .ok_or(TerminalMonitorError::Invalid)?;
            baseline.validate()?;
            monitor.runtime.path_baseline = Some(baseline);
        }
        Condition::CustomProbe { .. } => {
            require(activation.path_baseline.is_none())?;
            monitor.runtime.cwd_sha256 =
                Some(activation.cwd_sha256.ok_or(TerminalMonitorError::Invalid)?);
        }
        _ => require_empty_activation(activation)?,
    }
    Ok(())
}
fn probe_target(monitor: &Monitor) -> Result<TerminalProbeTarget> {
    Ok(match &monitor.definition.condition {
        Condition::TcpReady { host, port } => TerminalProbeTarget::Tcp {
            host: host.clone(),
            port: *port,
        },
        Condition::HttpReady { url } => TerminalProbeTarget::Http { url: url.clone() },
        Condition::PathExists { path }
        | Condition::PathChanged { path }
        | Condition::PathSize { path, .. } => TerminalProbeTarget::Path { path: path.clone() },
        Condition::CustomProbe { command, cwd } => TerminalProbeTarget::Custom {
            command: command.clone(),
            cwd: cwd.clone(),
            approved_cwd_sha256: monitor
                .runtime
                .cwd_sha256
                .ok_or(TerminalMonitorError::InvalidState)?,
        },
        _ => return Err(TerminalMonitorError::InvalidState),
    })
}
fn probe_matches(
    monitor: &mut Monitor,
    result: std::result::Result<TerminalProbeObservation, TerminalProbeFailure>,
) -> Result<bool> {
    let Ok(observation) = result else {
        return Ok(false);
    };
    match (&monitor.definition.condition, observation) {
        (Condition::TcpReady { .. }, TerminalProbeObservation::Tcp { connected }) => Ok(connected),
        (Condition::HttpReady { .. }, TerminalProbeObservation::Http { response_prefix }) => {
            require(response_prefix.len() <= 1024)?;
            Ok(response_prefix.len() >= 12 && response_prefix.starts_with(b"HTTP/"))
        }
        (
            condition @ (Condition::PathExists { .. }
            | Condition::PathSize { .. }
            | Condition::PathChanged { .. }),
            TerminalProbeObservation::Path { baseline },
        ) => {
            baseline.validate()?;
            Ok(match condition {
                Condition::PathExists { .. } => baseline.exists,
                Condition::PathSize { minimum_bytes, .. } => {
                    baseline.exists && baseline.size >= *minimum_bytes
                }
                _ => {
                    let previous = monitor.runtime.path_baseline.replace(baseline);
                    previous.is_some_and(|previous| previous != baseline)
                }
            })
        }
        (
            Condition::CustomProbe { .. },
            TerminalProbeObservation::Custom {
                exit_code,
                cwd_sha256,
            },
        ) => {
            require((0..=255).contains(&exit_code))?;
            Ok(exit_code == 0 && monitor.runtime.cwd_sha256 == Some(cwd_sha256))
        }
        _ => Err(TerminalMonitorError::Invalid),
    }
}
fn exit_matches(condition: &Condition, outcome: TerminalProcessOutcome) -> bool {
    match (condition, outcome) {
        (Condition::ProcessExit, _) => true,
        (Condition::ExitCode { exit_code }, TerminalProcessOutcome::Exited(actual)) => {
            *exit_code == actual
        }
        (Condition::Signal { signal }, TerminalProcessOutcome::Signaled(actual)) => {
            let number = match signal {
                TerminalSignal::Hangup => 1,
                TerminalSignal::Interrupt => 2,
                TerminalSignal::Quit => 3,
                TerminalSignal::Kill => 9,
                TerminalSignal::Terminate => 15,
            };
            actual == number
        }
        _ => false,
    }
}

fn condition_pattern(condition: &Condition) -> Result<Option<Arc<Pattern>>> {
    match condition {
        Condition::OutputContains { pattern } => {
            Pattern::new(pattern.as_bytes(), false).map(|pattern| Some(Arc::new(pattern)))
        }
        Condition::OutputMatches { pattern } | Condition::ScreenMatches { pattern } => {
            Pattern::new(pattern.as_bytes(), true).map(|pattern| Some(Arc::new(pattern)))
        }
        _ => Ok(None),
    }
}

/// Five-word NFA: one bounded bit-parallel pass per input byte. Consecutive
/// wildcard stars are equivalent and collapsed, making epsilon closure one shift.
struct Pattern {
    length: usize,
    masks: Box<[[u64; WORDS]]>,
    stars: [u64; WORDS],
}
impl Pattern {
    fn new(pattern: &[u8], wildcard: bool) -> Result<Self> {
        require(!pattern.is_empty() && pattern.len() <= MAX_PATTERN_BYTES)?;
        let mut tokens = Vec::with_capacity(pattern.len());
        for byte in pattern {
            if !(wildcard && *byte == b'*' && tokens.last() == Some(&b'*')) {
                tokens.push(*byte);
            }
        }
        let mut masks = vec![[0; WORDS]; 256].into_boxed_slice();
        let mut stars = [0; WORDS];
        for (index, byte) in tokens.iter().enumerate() {
            if wildcard && *byte == b'*' {
                set_bit(&mut stars, index);
            } else if wildcard && *byte == b'?' {
                for mask in &mut masks {
                    set_bit(mask, index);
                }
            } else {
                set_bit(&mut masks[usize::from(*byte)], index);
            }
        }
        Ok(Self {
            length: tokens.len(),
            masks,
            stars,
        })
    }
    fn closure(&self, states: &mut [u64; WORDS]) {
        let mut carry = 0;
        for (state, star) in states.iter_mut().zip(self.stars) {
            let advancing = *state & star;
            *state |= (advancing << 1) | carry;
            carry = advancing >> 63;
        }
    }
    fn feed(&self, states: &mut [u64; WORDS], bytes: &[u8]) -> bool {
        states[0] |= 1;
        self.closure(states);
        for byte in bytes {
            let mut next = [0; WORDS];
            next[0] = 1;
            let mut carry = 0;
            for index in 0..WORDS {
                let advancing = states[index] & self.masks[usize::from(*byte)][index];
                next[index] |= (advancing << 1) | carry | (states[index] & self.stars[index]);
                carry = advancing >> 63;
            }
            self.closure(&mut next);
            *states = next;
            if bit_is_set(states, self.length) {
                return true;
            }
        }
        bit_is_set(states, self.length)
    }
    fn valid_states(&self, states: &[u64; WORDS]) -> bool {
        (self.length + 1..WORDS * 64).all(|bit| !bit_is_set(states, bit))
    }
}
fn set_bit(states: &mut [u64; WORDS], index: usize) {
    states[index / 64] |= 1 << (index % 64);
}
fn bit_is_set(states: &[u64; WORDS], index: usize) -> bool {
    states[index / 64] & (1 << (index % 64)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::TerminalSchedule;

    fn context(now_ms: i64) -> TerminalMonitorContext {
        TerminalMonitorContext {
            now_ms,
            cursor: TerminalCursor::new(1, u64::try_from(now_ms).unwrap()).unwrap(),
            lifecycle: TerminalLifecycle::Running,
        }
    }
    fn set() -> TerminalMonitorSet {
        TerminalMonitorSet::new(TerminalSessionId::new("session-test").unwrap(), context(0))
            .unwrap()
    }
    #[test]
    fn mutation_receipts_identify_exact_live_and_retired_generation() {
        let mut set = set();
        let added = set
            .apply(
                Operation::Add {
                    definition: definition(tcp()),
                },
                context(0),
            )
            .unwrap();
        assert_eq!(added.generation, 1);
        let id = added.monitor_id;
        for (operation, generation, removed) in [
            (
                Operation::Update {
                    monitor_id: id.clone(),
                    definition: definition(tcp()),
                },
                2,
                false,
            ),
            (
                Operation::Pause {
                    monitor_id: id.clone(),
                },
                3,
                false,
            ),
            (
                Operation::Resume {
                    monitor_id: id.clone(),
                },
                4,
                false,
            ),
            (Operation::Remove { monitor_id: id }, 4, true),
        ] {
            let receipt = set.apply(operation, context(0)).unwrap();
            assert_eq!(receipt.generation, generation);
            assert_eq!(receipt.removed, removed);
        }
    }
    fn definition(condition: Condition) -> Definition {
        let check_schedule = condition
            .requires_polling()
            .then_some(TerminalSchedule { interval_ms: 10 });
        Definition {
            condition,
            check_schedule,
            notify: Notify::OnMatch,
            lifetime: Lifetime::UntilSessionEnd,
        }
    }
    fn add(set: &mut TerminalMonitorSet, definition: Definition) -> TerminalMonitorId {
        let activation = match definition.condition {
            Condition::PathChanged { .. } => TerminalMonitorActivation {
                path_baseline: Some(TerminalPathBaseline {
                    exists: false,
                    size: 0,
                    modified_ns: 0,
                }),
                cwd_sha256: None,
            },
            Condition::CustomProbe { .. } => TerminalMonitorActivation {
                path_baseline: None,
                cwd_sha256: Some([7; 32]),
            },
            _ => TerminalMonitorActivation::default(),
        };
        set.apply_with_activation(
            Operation::Add { definition },
            activation,
            set.context.clone(),
        )
        .unwrap()
        .monitor_id
    }
    fn events(set: &mut TerminalMonitorSet) -> Vec<TerminalMonitorEvent> {
        set.events(&TerminalEventQuery {
            after_event_id: 0,
            acknowledge_event_id: None,
            max_events: 256,
        })
        .unwrap()
    }
    fn evidence(
        request: &TerminalProbeRequest,
        result: TerminalProbeObservation,
    ) -> TerminalProbeEvidence {
        let output_bytes = match &result {
            TerminalProbeObservation::Http { response_prefix } => response_prefix.len() as u64,
            _ => 0,
        };
        TerminalProbeEvidence {
            session_id: request.session_id.clone(),
            monitor_id: request.monitor_id.clone(),
            generation: request.generation,
            request_sequence: request.request_sequence,
            completed_at_ms: request.started_at_ms,
            output_bytes,
            truncated: false,
            timed_out: false,
            result: Ok(result),
        }
    }
    fn tcp() -> Condition {
        Condition::TcpReady {
            host: "localhost".into(),
            port: 80,
        }
    }
    fn contains(pattern: &str) -> Condition {
        Condition::OutputContains {
            pattern: pattern.into(),
        }
    }

    #[test]
    fn local_conditions_produce_exact_matches() {
        let cases = [
            (
                Condition::ProcessExit,
                Some(TerminalProcessOutcome::Signaled(6)),
            ),
            (
                Condition::ExitCode { exit_code: 42 },
                Some(TerminalProcessOutcome::Exited(42)),
            ),
            (
                Condition::Signal {
                    signal: TerminalSignal::Terminate,
                },
                Some(TerminalProcessOutcome::Signaled(15)),
            ),
        ];
        for (condition, outcome) in cases {
            let mut set = set();
            add(&mut set, definition(condition));
            let mut ctx = context(1);
            ctx.lifecycle = TerminalLifecycle::Exited;
            set.end_session(outcome, ctx).unwrap();
            assert_eq!(events(&mut set)[0].reason, Reason::Matched);
            assert_eq!(set.len(), 0);
        }
        for condition in [
            contains("ready"),
            Condition::OutputMatches {
                pattern: "r*?dy".into(),
            },
        ] {
            let mut set = set();
            let id = add(&mut set, definition(condition));
            set.output(b"xxrea", context(1)).unwrap();
            assert_eq!(set.state(&id), Some(State::Active));
            set.output(b"dy!", context(2)).unwrap();
            assert_eq!(set.state(&id), Some(State::Matched));
        }
        let mut quiet = set();
        let id = add(
            &mut quiet,
            definition(Condition::OutputQuiet { duration_ms: 10 }),
        );
        quiet.tick(context(10)).unwrap();
        assert_eq!(quiet.state(&id), Some(State::Matched));
        let mut screen = set();
        let id = add(
            &mut screen,
            definition(Condition::ScreenMatches {
                pattern: "界?\n".into(),
            }),
        );
        let mut grid = crate::terminal_grid::TerminalGrid::new(8, 2).unwrap();
        grid.feed("界x".as_bytes()).unwrap();
        screen
            .screen(&grid.structured_screen().unwrap(), context(1))
            .unwrap();
        assert_eq!(screen.state(&id), Some(State::Matched));
    }

    #[test]
    fn probe_conditions_produce_exact_matches() {
        let probes = [
            (tcp(), TerminalProbeObservation::Tcp { connected: true }),
            (
                Condition::HttpReady {
                    url: "http://localhost/".into(),
                },
                TerminalProbeObservation::Http {
                    response_prefix: b"HTTP/1.1 503 Service Unavailable".to_vec(),
                },
            ),
            (
                Condition::PathExists {
                    path: "ready".into(),
                },
                TerminalProbeObservation::Path {
                    baseline: TerminalPathBaseline {
                        exists: true,
                        size: 0,
                        modified_ns: 1,
                    },
                },
            ),
            (
                Condition::PathChanged {
                    path: "ready".into(),
                },
                TerminalProbeObservation::Path {
                    baseline: TerminalPathBaseline {
                        exists: true,
                        size: 0,
                        modified_ns: 1,
                    },
                },
            ),
            (
                Condition::PathSize {
                    path: "ready".into(),
                    minimum_bytes: 4,
                },
                TerminalProbeObservation::Path {
                    baseline: TerminalPathBaseline {
                        exists: true,
                        size: 4,
                        modified_ns: 1,
                    },
                },
            ),
            (
                Condition::CustomProbe {
                    command: "true".into(),
                    cwd: ".".into(),
                },
                TerminalProbeObservation::Custom {
                    exit_code: 0,
                    cwd_sha256: [7; 32],
                },
            ),
        ];
        for (condition, result) in probes {
            let mut set = set();
            let id = add(&mut set, definition(condition));
            let request = set.tick(context(10)).unwrap().remove(0);
            assert!(
                set.complete_probe(evidence(&request, result), context(10))
                    .unwrap()
            );
            assert_eq!(set.state(&id), Some(State::Matched));
            assert_eq!(events(&mut set).len(), 1);
        }
    }

    #[test]
    fn output_quiet_resets_without_counting_output_and_repeats_checks() {
        let mut set = set();
        let mut def = definition(Condition::OutputQuiet { duration_ms: 10 });
        def.notify = Notify::EveryCheck;
        add(&mut set, def);
        assert_eq!(set.next_deadline(), Some(10));
        set.output(b"x", context(9)).unwrap();
        set.tick(context(10)).unwrap();
        assert!(events(&mut set).is_empty());
        set.output(b"", context(18)).unwrap();
        assert_eq!(set.next_deadline(), Some(19));
        set.tick(context(19)).unwrap();
        set.tick(context(50)).unwrap();
        assert_eq!(events(&mut set).len(), 2);
        assert_eq!(set.next_deadline(), Some(60));
    }

    #[test]
    fn notifications_latch_and_every_n_count_observations_not_bytes() {
        for (notify, expected) in [
            (Notify::OnMatch, vec![Reason::Matched]),
            (Notify::OnStateChange, vec![Reason::StateChanged]),
            (Notify::EveryCheck, vec![Reason::Check; 3]),
            (Notify::EveryNChecks { count: 2 }, vec![Reason::Check]),
            (Notify::OnExit, vec![]),
            (Notify::Interval { interval_ms: 10 }, vec![]),
        ] {
            let mut set = set();
            let mut def = definition(contains("x"));
            def.notify = notify;
            add(&mut set, def);
            set.output(b"xxxxxxxx", context(1)).unwrap();
            set.output(b"z", context(2)).unwrap();
            set.output(b"x", context(3)).unwrap();
            assert_eq!(
                events(&mut set)
                    .iter()
                    .map(|event| event.reason)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(set.monitors[0].runtime.check_count, 3);
        }
    }

    #[test]
    fn intervals_coalesce_keep_phase_and_expiration_wins() {
        let mut set = set();
        let mut def = definition(contains("x"));
        def.notify = Notify::Interval { interval_ms: 10 };
        def.lifetime = Lifetime::Duration { duration_ms: 100 };
        add(&mut set, def);
        set.tick(context(35)).unwrap();
        assert_eq!(set.next_deadline(), Some(40));
        assert_eq!(events(&mut set).len(), 1);
        set.tick(context(99)).unwrap();
        set.tick(context(100)).unwrap();
        assert_eq!(set.len(), 0);
        assert_eq!(events(&mut set).len(), 2);
    }

    #[test]
    fn operations_generation_lifetimes_and_raw_gap_are_explicit() {
        let mut set = set();
        let mut def = definition(contains("ready"));
        def.notify = Notify::OnStateChange;
        def.lifetime = Lifetime::Duration { duration_ms: 100 };
        let id = add(&mut set, def.clone());
        set.output(b"rea", context(1)).unwrap();
        set.apply(
            Operation::Pause {
                monitor_id: id.clone(),
            },
            context(2),
        )
        .unwrap();
        assert_eq!(
            set.apply(
                Operation::Pause {
                    monitor_id: id.clone()
                },
                context(3)
            )
            .err(),
            Some(TerminalMonitorError::InvalidState)
        );
        set.output(b"dy", context(3)).unwrap();
        assert_eq!(set.monitors[0].runtime.check_count, 1);
        set.apply(
            Operation::Resume {
                monitor_id: id.clone(),
            },
            context(4),
        )
        .unwrap();
        set.output(b"dy", context(5)).unwrap();
        assert_eq!(set.state(&id), Some(State::Matched));
        set.raw_gap(context(6)).unwrap();
        assert_eq!(set.state(&id), Some(State::Degraded));
        assert_eq!(
            set.apply(
                Operation::Resume {
                    monitor_id: id.clone()
                },
                context(7)
            )
            .err(),
            Some(TerminalMonitorError::InvalidState)
        );
        set.apply(
            Operation::Update {
                monitor_id: id.clone(),
                definition: def,
            },
            context(7),
        )
        .unwrap();
        assert_eq!(set.state(&id), Some(State::Active));
        assert_eq!(set.monitors[0].runtime.generation, 5);
        assert!(
            set.apply(
                Operation::Remove {
                    monitor_id: id.clone()
                },
                context(8)
            )
            .unwrap()
            .removed
        );
        assert_eq!(
            events(&mut set)
                .iter()
                .map(|event| event.reason)
                .collect::<Vec<_>>(),
            vec![
                Reason::Paused,
                Reason::Resumed,
                Reason::StateChanged,
                Reason::StateChanged,
                Reason::Updated,
                Reason::Removed
            ]
        );
        assert_eq!(
            set.apply(Operation::Remove { monitor_id: id }, context(9))
                .err(),
            Some(TerminalMonitorError::NotFound)
        );
    }

    #[test]
    fn paused_duration_expires_and_until_match_removes_without_notification() {
        let mut set = set();
        let mut def = definition(contains("x"));
        def.notify = Notify::OnStateChange;
        def.lifetime = Lifetime::Duration { duration_ms: 10 };
        let id = add(&mut set, def);
        set.apply(Operation::Pause { monitor_id: id }, context(1))
            .unwrap();
        set.tick(context(10)).unwrap();
        assert_eq!(events(&mut set).last().unwrap().reason, Reason::Expired);
        assert_eq!(set.len(), 0);
        let mut def = definition(contains("x"));
        def.notify = Notify::EveryNChecks { count: 2 };
        def.lifetime = Lifetime::UntilMatch;
        add(&mut set, def);
        let before = events(&mut set).len();
        set.output(b"x", context(11)).unwrap();
        assert_eq!(set.len(), 0);
        assert_eq!(events(&mut set).len(), before);
    }

    #[test]
    fn session_end_handles_non_control_signals_without_inventing_exits() {
        let mut set = set();
        add(&mut set, definition(Condition::ProcessExit));
        add(
            &mut set,
            definition(Condition::Signal {
                signal: TerminalSignal::Terminate,
            }),
        );
        let mut def = definition(contains("ready"));
        def.notify = Notify::OnExit;
        add(&mut set, def.clone());
        let paused = add(&mut set, def);
        set.apply(Operation::Pause { monitor_id: paused }, context(0))
            .unwrap();
        let mut ctx = context(1);
        ctx.lifecycle = TerminalLifecycle::Exited;
        set.end_session(Some(TerminalProcessOutcome::Signaled(6)), ctx)
            .unwrap();
        assert_eq!(
            events(&mut set)
                .iter()
                .map(|event| event.reason)
                .collect::<Vec<_>>(),
            vec![Reason::Matched, Reason::SessionExit]
        );
        assert_eq!(set.len(), 0);
        assert_eq!(
            set.tick(context(2)).err(),
            Some(TerminalMonitorError::InvalidState)
        );
        let mut lost = super::tests::set();
        add(&mut lost, definition(Condition::ProcessExit));
        let mut ctx = context(1);
        ctx.lifecycle = TerminalLifecycle::Lost;
        lost.end_session(None, ctx).unwrap();
        assert!(events(&mut lost).is_empty());
    }

    #[test]
    fn path_changed_activation_baseline_and_resume_are_preserved() {
        let mut set = set();
        let def = definition(Condition::PathChanged {
            path: "ready".into(),
        });
        assert_eq!(
            set.apply(
                Operation::Add {
                    definition: def.clone()
                },
                context(0)
            )
            .err(),
            Some(TerminalMonitorError::Invalid)
        );
        let id = add(&mut set, def);
        let request = set.tick(context(10)).unwrap().remove(0);
        let absent = TerminalPathBaseline {
            exists: false,
            size: 0,
            modified_ns: 0,
        };
        set.complete_probe(
            evidence(
                &request,
                TerminalProbeObservation::Path { baseline: absent },
            ),
            context(10),
        )
        .unwrap();
        assert!(events(&mut set).is_empty());
        set.apply(
            Operation::Pause {
                monitor_id: id.clone(),
            },
            context(11),
        )
        .unwrap();
        set.apply(
            Operation::Resume {
                monitor_id: id.clone(),
            },
            context(12),
        )
        .unwrap();
        assert_eq!(set.next_deadline(), Some(22));
        let request = set.tick(context(22)).unwrap().remove(0);
        set.complete_probe(
            evidence(
                &request,
                TerminalProbeObservation::Path {
                    baseline: TerminalPathBaseline {
                        exists: true,
                        size: 10,
                        modified_ns: 1,
                    },
                },
            ),
            context(22),
        )
        .unwrap();
        assert_eq!(set.state(&id), Some(State::Matched));
    }

    #[test]
    fn stale_probe_evidence_cannot_touch_replacement_or_clock() {
        let mut set = set();
        let id = add(&mut set, definition(tcp()));
        let request = set.tick(context(10)).unwrap().remove(0);
        set.apply(
            Operation::Update {
                monitor_id: id.clone(),
                definition: definition(tcp()),
            },
            context(11),
        )
        .unwrap();
        let before = set.snapshot().unwrap();
        assert!(
            !set.complete_probe(
                evidence(&request, TerminalProbeObservation::Tcp { connected: true }),
                context(0)
            )
            .unwrap()
        );
        assert_eq!(set.snapshot().unwrap(), before);
        let request = set.tick(context(21)).unwrap().remove(0);
        let mut wrong = evidence(&request, TerminalProbeObservation::Tcp { connected: true });
        wrong.session_id = TerminalSessionId::new("other").unwrap();
        assert!(!set.complete_probe(wrong, context(0)).unwrap());
        let wrong = evidence(
            &request,
            TerminalProbeObservation::Path {
                baseline: TerminalPathBaseline {
                    exists: false,
                    size: 0,
                    modified_ns: 0,
                },
            },
        );
        let before = set.snapshot().unwrap();
        assert_eq!(
            set.complete_probe(wrong, context(21)).err(),
            Some(TerminalMonitorError::Invalid)
        );
        assert_eq!(set.snapshot().unwrap(), before);
        set.complete_probe(
            evidence(&request, TerminalProbeObservation::Tcp { connected: true }),
            context(21),
        )
        .unwrap();
        assert_eq!(set.state(&id), Some(State::Matched));
    }

    #[test]
    fn missing_late_failed_and_oversized_probes_reschedule_without_matches() {
        let mut set = set();
        let mut def = definition(tcp());
        def.notify = Notify::EveryCheck;
        let id = add(&mut set, def);
        let request = set.tick(context(10)).unwrap().remove(0);
        assert_eq!(request.deadline_ms, 2010);
        assert_eq!(request.output_limit_bytes, PROBE_OUTPUT_BYTES);
        assert!(set.tick(context(2009)).unwrap().is_empty());
        assert!(set.tick(context(2010)).unwrap().is_empty());
        assert_eq!(set.next_deadline(), Some(2020));
        assert!(
            !set.complete_probe(
                evidence(&request, TerminalProbeObservation::Tcp { connected: true }),
                context(2010)
            )
            .unwrap()
        );
        for (index, failure) in [
            TerminalProbeFailure::Unavailable,
            TerminalProbeFailure::Denied,
            TerminalProbeFailure::Timeout,
            TerminalProbeFailure::OutputLimit,
            TerminalProbeFailure::InvalidEvidence,
        ]
        .into_iter()
        .enumerate()
        {
            let now = 2020 + i64::try_from(index).unwrap() * 10;
            let request = set.tick(context(now)).unwrap().remove(0);
            let mut result = evidence(&request, TerminalProbeObservation::Tcp { connected: true });
            result.result = Err(failure);
            assert!(set.complete_probe(result, context(now)).unwrap());
        }
        for (index, mode) in [0, 1, 2, 3].into_iter().enumerate() {
            let now = 2070 + i64::try_from(index).unwrap() * 2010;
            let request = set.tick(context(now)).unwrap().remove(0);
            let mut result = evidence(&request, TerminalProbeObservation::Tcp { connected: true });
            match mode {
                0 => result.output_bytes = PROBE_OUTPUT_BYTES as u64 + 1,
                1 => result.truncated = true,
                2 => result.timed_out = true,
                _ => result.completed_at_ms = request.deadline_ms,
            }
            let completed = result.completed_at_ms;
            assert!(set.complete_probe(result, context(completed)).unwrap());
        }
        assert_eq!(set.state(&id), Some(State::Active));
        assert_eq!(events(&mut set).len(), 10);
    }

    #[test]
    fn custom_probe_requires_approved_cwd_and_http_requires_consistent_prefix() {
        let mut set = set();
        let def = definition(Condition::CustomProbe {
            command: "true".into(),
            cwd: ".".into(),
        });
        assert!(
            set.apply(
                Operation::Add {
                    definition: def.clone()
                },
                context(0)
            )
            .is_err()
        );
        let id = add(&mut set, def);
        let request = set.tick(context(10)).unwrap().remove(0);
        assert!(matches!(
            request.target,
            TerminalProbeTarget::Custom {
                approved_cwd_sha256: [7, ..],
                ..
            }
        ));
        set.complete_probe(
            evidence(
                &request,
                TerminalProbeObservation::Custom {
                    exit_code: 0,
                    cwd_sha256: [8; 32],
                },
            ),
            context(10),
        )
        .unwrap();
        assert_eq!(set.state(&id), Some(State::Active));
        let mut http = super::tests::set();
        add(
            &mut http,
            definition(Condition::HttpReady {
                url: "http://localhost/".into(),
            }),
        );
        let request = http.tick(context(10)).unwrap().remove(0);
        let mut result = evidence(
            &request,
            TerminalProbeObservation::Http {
                response_prefix: b"HTTP/1.1 200 OK".to_vec(),
            },
        );
        result.output_bytes = 0;
        assert_eq!(
            http.complete_probe(result, context(10)).err(),
            Some(TerminalMonitorError::Invalid)
        );
    }

    #[test]
    fn event_bounds_acknowledgment_and_stable_ids_survive_restore() {
        let mut set = set();
        let mut def = definition(contains("x"));
        def.notify = Notify::EveryCheck;
        let id = add(&mut set, def);
        for now in 1..=300 {
            set.output(b"x", context(now)).unwrap();
        }
        assert_eq!(set.dropped_through_event_id(), 44);
        assert_eq!(events(&mut set)[0].event_id, 45);
        let page = set
            .events(&TerminalEventQuery {
                after_event_id: 295,
                acknowledge_event_id: Some(298),
                max_events: 2,
            })
            .unwrap();
        assert_eq!(
            page.iter().map(|event| event.event_id).collect::<Vec<_>>(),
            vec![296, 297]
        );
        assert_eq!(set.acknowledged_event_id(), 298);
        assert_eq!(events(&mut set).len(), 256);
        let before = set.snapshot().unwrap();
        assert_eq!(
            set.events(&TerminalEventQuery {
                after_event_id: 0,
                acknowledge_event_id: Some(301),
                max_events: 2
            })
            .err(),
            Some(TerminalMonitorError::Invalid)
        );
        assert_eq!(set.snapshot().unwrap(), before);
        let mut restored = TerminalMonitorSet::restore(&before).unwrap();
        assert_eq!(restored.snapshot().unwrap(), before);
        restored
            .apply(Operation::Remove { monitor_id: id }, context(300))
            .unwrap();
        assert_eq!(
            add(&mut restored, definition(contains("z"))).as_str(),
            "monitor-2"
        );
    }

    #[test]
    fn checkpoint_retains_split_match_pending_probe_and_event_order() {
        let mut original = set();
        add(
            &mut original,
            definition(Condition::OutputMatches {
                pattern: "a*界?".into(),
            }),
        );
        add(&mut original, definition(tcp()));
        original.output(b"a\xe7", context(1)).unwrap();
        let request = original.tick(context(10)).unwrap().remove(0);
        let mut restored = TerminalMonitorSet::restore(&original.snapshot().unwrap()).unwrap();
        for set in [&mut original, &mut restored] {
            set.output(b"\x95\x8cx", context(11)).unwrap();
            let mut result = evidence(&request, TerminalProbeObservation::Tcp { connected: true });
            result.completed_at_ms = 12;
            set.complete_probe(result, context(12)).unwrap();
        }
        assert_eq!(original.snapshot().unwrap(), restored.snapshot().unwrap());
        let events = events(&mut restored);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].monitor_id.as_str(), "monitor-1");
        assert_eq!(events[1].monitor_id.as_str(), "monitor-2");
    }

    #[test]
    fn corrupt_and_overcapacity_snapshots_are_rejected() {
        let mut set = set();
        let mut def = definition(contains("x"));
        def.notify = Notify::EveryCheck;
        add(&mut set, def);
        set.output(b"x", context(1)).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&set.snapshot().unwrap()).unwrap();
        for (path, value) in [
            ("/schema", serde_json::json!(2)),
            ("/next_event_id", serde_json::json!(4)),
            ("/events/0/event_id", serde_json::json!(2)),
            ("/monitors/0/runtime/generation", serde_json::json!(0)),
            (
                "/monitors/0/runtime/matcher_states/4",
                serde_json::json!(u64::MAX),
            ),
            (
                "/monitors/0/runtime/condition_matched",
                serde_json::json!(false),
            ),
            (
                "/monitors/0/runtime/path_baseline",
                serde_json::json!({"exists":false,"size":0,"modified_ns":0}),
            ),
            ("/context/now_ms", serde_json::json!(-1)),
        ] {
            let mut corrupt = original.clone();
            *corrupt.pointer_mut(path).unwrap() = value;
            assert!(
                TerminalMonitorSet::restore(&serde_json::to_vec(&corrupt).unwrap()).is_err(),
                "{path}"
            );
        }
        let mut corrupt = original;
        corrupt["monitors"] =
            serde_json::json!(vec![corrupt["monitors"][0].clone(); MAX_MONITORS + 1]);
        assert!(TerminalMonitorSet::restore(&serde_json::to_vec(&corrupt).unwrap()).is_err());
        assert!(TerminalMonitorSet::restore(b"{}").is_err());
    }

    #[test]
    fn transition_bounds_clock_and_counter_failures_are_atomic() {
        let mut set = set();
        add(&mut set, definition(contains("x")));
        set.output(b"a", context(5)).unwrap();
        let before = set.snapshot().unwrap();
        assert_eq!(
            set.output(b"x", context(4)).err(),
            Some(TerminalMonitorError::Clock)
        );
        let mut ctx = context(6);
        ctx.cursor = TerminalCursor::new(1, 0).unwrap();
        assert_eq!(set.tick(ctx).err(), Some(TerminalMonitorError::Clock));
        assert_eq!(
            set.output(&vec![0; MAX_MONITOR_FEED_BYTES + 1], context(6))
                .err(),
            Some(TerminalMonitorError::Invalid)
        );
        assert_eq!(set.snapshot().unwrap(), before);
        set.next_event_id = u64::MAX;
        let before = set.snapshot().unwrap();
        assert_eq!(
            set.output(b"x", context(6)).err(),
            Some(TerminalMonitorError::Counter)
        );
        assert_eq!(set.snapshot().unwrap(), before);
        let mut full = super::tests::set();
        for _ in 0..MAX_MONITORS {
            add(&mut full, definition(tcp()));
        }
        assert_eq!(
            full.apply(
                Operation::Add {
                    definition: definition(tcp())
                },
                context(0)
            )
            .err(),
            Some(TerminalMonitorError::Capacity)
        );
        assert_eq!(full.tick(context(10)).unwrap().len(), MAX_MONITORS);
    }

    fn scalar_match(pattern: &[u8], bytes: &[u8]) -> bool {
        let mut state = vec![false; pattern.len() + 1];
        state[0] = true;
        let closure = |state: &mut Vec<bool>| {
            for index in 0..pattern.len() {
                if state[index] && pattern[index] == b'*' {
                    state[index + 1] = true;
                }
            }
        };
        closure(&mut state);
        for byte in bytes {
            let mut next = vec![false; state.len()];
            next[0] = true;
            for index in 0..pattern.len() {
                if state[index] {
                    match pattern[index] {
                        b'*' => next[index] = true,
                        b'?' => next[index + 1] = true,
                        literal if literal == *byte => next[index + 1] = true,
                        _ => {}
                    }
                }
            }
            closure(&mut next);
            state = next;
            if state[pattern.len()] {
                return true;
            }
        }
        state[pattern.len()]
    }

    #[test]
    fn bit_parallel_matcher_matches_reference_and_split_streams() {
        for length in 1..=5u32 {
            for mut encoded in 0..4usize.pow(length) {
                let mut pattern = Vec::new();
                for _ in 0..length {
                    pattern.push(b"ab*?"[encoded % 4]);
                    encoded /= 4;
                }
                let compiled = Pattern::new(&pattern, true).unwrap();
                for mut encoded in 0..32usize {
                    let mut bytes = Vec::new();
                    for _ in 0..5 {
                        bytes.push(b"ab"[encoded % 2]);
                        encoded /= 2;
                    }
                    let expected = scalar_match(&pattern, &bytes);
                    assert_eq!(compiled.feed(&mut [0; WORDS], &bytes), expected);
                    for split in 0..=bytes.len() {
                        let mut states = [0; WORDS];
                        let actual = compiled.feed(&mut states, &bytes[..split])
                            || compiled.feed(&mut states, &bytes[split..]);
                        // Empty prefixes are not observations in the output API.
                        if split > 0 || !scalar_match(&pattern, b"") {
                            assert_eq!(actual, expected);
                        }
                    }
                }
            }
        }
        for length in [63, 64, 65, 127, 128, 191, 192, 255, 256] {
            let pattern = vec![b'x'; length];
            let compiled = Pattern::new(&pattern, false).unwrap();
            let mut states = [0; WORDS];
            assert!(!compiled.feed(&mut states, &pattern[..length - 1]));
            assert!(compiled.feed(&mut states, b"x"));
            assert!(compiled.valid_states(&states));
        }
        let compiled = Pattern::new(b"a*?", false).unwrap();
        assert!(!compiled.feed(&mut [0; WORDS], b"abc"));
        assert!(compiled.feed(&mut [0; WORDS], b"a*?"));
    }

    fn wait(condition: TerminalReturnCondition, ceiling: u64) -> TerminalWaitState {
        TerminalWaitState::new(
            TerminalWaitRequest {
                condition,
                safety_ceiling_ms: ceiling,
            },
            &context(0),
            0,
            false,
        )
        .unwrap()
    }
    #[test]
    fn waits_preserve_exit_cancel_condition_and_ceiling_precedence() {
        let mut started = wait(TerminalReturnCondition::Started, 10);
        assert_eq!(started.started_at_ms(), 0);
        assert_eq!(
            started.poll(&context(0), None, false).unwrap(),
            Some(TerminalWaitOutcome::Started)
        );
        let mut exit = wait(TerminalReturnCondition::Exit, 10);
        assert_eq!(exit.poll(&context(9), None, false).unwrap(), None);
        assert_eq!(
            exit.poll(&context(10), None, false).unwrap(),
            Some(TerminalWaitOutcome::SafetyCeiling)
        );
        let mut quiet = wait(TerminalReturnCondition::Quiet { duration_ms: 10 }, 10);
        assert_eq!(
            quiet.poll(&context(10), None, false).unwrap(),
            Some(TerminalWaitOutcome::ConditionMet)
        );
        let mut terminated = wait(TerminalReturnCondition::Started, 10);
        assert_eq!(
            terminated
                .poll(
                    &context(10),
                    Some(TerminalProcessOutcome::Signaled(6)),
                    false
                )
                .unwrap(),
            Some(TerminalWaitOutcome::Signaled(6))
        );
        let mut cancelled = wait(TerminalReturnCondition::Started, 10);
        assert_eq!(
            cancelled
                .poll(&context(10), Some(TerminalProcessOutcome::Exited(0)), true)
                .unwrap(),
            Some(TerminalWaitOutcome::Cancelled)
        );
        assert_eq!(cancelled.next_deadline(), None);
        let mut lost = wait(TerminalReturnCondition::Exit, 10);
        let mut ctx = context(1);
        ctx.lifecycle = TerminalLifecycle::Lost;
        assert_eq!(
            lost.poll(&ctx, None, false).unwrap(),
            Some(TerminalWaitOutcome::Lost)
        );
    }

    #[test]
    fn quiet_wait_timing_starting_ceiling_and_attention_cancellation_are_local() {
        let mut attention = wait(TerminalReturnCondition::Quiet { duration_ms: 10 }, 100);
        attention.output(b"x", 9).unwrap();
        assert_eq!(attention.next_deadline(), Some(19));
        assert_eq!(attention.poll(&context(18), None, false).unwrap(), None);
        attention.output(b"", 18).unwrap();
        assert_eq!(
            attention.poll(&context(19), None, false).unwrap(),
            Some(TerminalWaitOutcome::ConditionMet)
        );
        let mut ctx = context(0);
        ctx.lifecycle = TerminalLifecycle::Starting;
        let mut attention = TerminalWaitState::new(
            TerminalWaitRequest {
                condition: TerminalReturnCondition::Quiet { duration_ms: 10 },
                safety_ceiling_ms: 100,
            },
            &ctx,
            0,
            false,
        )
        .unwrap();
        assert_eq!(attention.next_deadline(), Some(100));
        ctx.now_ms = 20;
        assert_eq!(attention.poll(&ctx, None, false).unwrap(), None);
        assert_eq!(attention.next_deadline(), Some(100));
        let mut monitors = set();
        let id = add(&mut monitors, definition(contains("x")));
        let before = monitors.snapshot().unwrap();
        assert_eq!(
            attention.poll(&ctx, None, true).unwrap(),
            Some(TerminalWaitOutcome::Cancelled)
        );
        drop(attention);
        assert_eq!(monitors.snapshot().unwrap(), before);
        assert_eq!(monitors.state(&id), Some(State::Active));
    }

    #[test]
    fn literal_wait_handles_max_pattern_split_utf8_nul_and_history_seed() {
        let pattern = format!("{}界*?", "x".repeat(4091));
        assert_eq!(pattern.len(), 4096);
        let mut attention = wait(
            TerminalReturnCondition::Match {
                pattern: pattern.clone(),
            },
            100,
        );
        attention.output(&pattern.as_bytes()[..4092], 1).unwrap();
        assert_eq!(attention.poll(&context(1), None, false).unwrap(), None);
        attention.output(&pattern.as_bytes()[4092..], 2).unwrap();
        assert_eq!(
            attention.poll(&context(2), None, false).unwrap(),
            Some(TerminalWaitOutcome::ConditionMet)
        );
        let mut attention = wait(
            TerminalReturnCondition::Match {
                pattern: "ab".into(),
            },
            100,
        );
        attention.output(b"a\0a", 1).unwrap();
        attention.output(b"b", 2).unwrap();
        assert_eq!(
            attention.poll(&context(2), None, false).unwrap(),
            Some(TerminalWaitOutcome::ConditionMet)
        );
        let mut attention = TerminalWaitState::new(
            TerminalWaitRequest {
                condition: TerminalReturnCondition::Match {
                    pattern: "done".into(),
                },
                safety_ceiling_ms: 100,
            },
            &context(0),
            0,
            true,
        )
        .unwrap();
        assert_eq!(
            attention.poll(&context(0), None, false).unwrap(),
            Some(TerminalWaitOutcome::ConditionMet)
        );
        assert!(
            TerminalWaitState::new(
                TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit,
                    safety_ceiling_ms: 0
                },
                &context(0),
                0,
                false
            )
            .is_err()
        );
        let mut attention = wait(TerminalReturnCondition::Exit, 100);
        attention.output(b"x", 10).unwrap();
        assert_eq!(
            attention.output(b"x", 9).err(),
            Some(TerminalMonitorError::Clock)
        );
        assert_eq!(
            attention.poll(&context(9), None, false).err(),
            Some(TerminalMonitorError::Clock)
        );
    }
}

fn screen_text(screen: &TerminalScreen) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for row in screen
        .cells
        .chunks(usize::from(screen.dimensions.columns()))
    {
        let start = bytes.len();
        for cell in row {
            match cell.kind {
                TerminalCellKind::Blank => bytes.push(b' '),
                TerminalCellKind::Continuation => {}
                _ => bytes.extend_from_slice(cell.text.as_bytes()),
            }
        }
        while bytes.len() > start && bytes.last() == Some(&b' ') {
            bytes.pop();
        }
        bytes.push(b'\n');
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(TerminalMonitorError::Capacity);
        }
    }
    Ok(bytes)
}

fn validate_runtime(
    runtime: &Runtime,
    definition: &Definition,
    set: &TerminalMonitorSet,
) -> Result<()> {
    require(
        runtime.generation > 0
            && runtime.created_at_ms >= 0
            && runtime.created_at_ms <= set.context.now_ms
            && runtime.last_event_id < set.next_event_id
            && (runtime.last_event_id == 0) == runtime.last_event_reason.is_none()
            && (runtime.notification_count == 0) == (runtime.last_event_id == 0)
            && !(runtime.condition_matched && runtime.state == State::Active)
            && (runtime.state != State::Matched || runtime.condition_matched),
    )?;
    let expected = match definition.lifetime {
        Lifetime::Duration { duration_ms } => Some(deadline(runtime.created_at_ms, duration_ms)?),
        _ => None,
    };
    require(runtime.lifetime_deadline_ms == expected)?;
    for value in [runtime.next_check_ms, runtime.next_notification_ms]
        .into_iter()
        .flatten()
    {
        require(value >= runtime.created_at_ms)?;
    }
    if runtime.state == State::Degraded {
        require(
            matches!(
                definition.condition,
                Condition::OutputContains { .. }
                    | Condition::OutputMatches { .. }
                    | Condition::OutputQuiet { .. }
                    | Condition::ScreenMatches { .. }
            ) && runtime.next_check_ms.is_none()
                && runtime.next_notification_ms.is_none()
                && runtime.pending.is_none()
                && runtime.matcher_states == [0; WORDS],
        )?;
    } else {
        require(
            runtime.next_check_ms.is_some()
                == (definition.condition.requires_polling()
                    || matches!(definition.condition, Condition::OutputQuiet { .. }))
                && runtime.next_notification_ms.is_some()
                    == matches!(definition.notify, Notify::Interval { .. }),
        )?;
    }
    require(
        runtime.path_baseline.is_some()
            == matches!(definition.condition, Condition::PathChanged { .. })
            && runtime.cwd_sha256.is_some()
                == matches!(definition.condition, Condition::CustomProbe { .. }),
    )?;
    if let Some(baseline) = runtime.path_baseline {
        baseline.validate()?;
    }
    if let Some(pending) = &runtime.pending {
        require(
            definition.condition.requires_polling()
                && !matches!(runtime.state, State::Paused | State::Degraded)
                && pending.sequence > 0
                && pending.sequence < set.next_probe_sequence
                && pending.started_at_ms >= runtime.created_at_ms
                && pending.started_at_ms <= set.context.now_ms
                && pending.deadline_ms == deadline(pending.started_at_ms, PROBE_TIMEOUT_MS)?,
        )?;
    }
    Ok(())
}

struct SnapshotWriter(Vec<u8>);
impl std::io::Write for SnapshotWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_SNAPSHOT_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("terminal monitor snapshot capacity"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_vec<'de, D, T, const MAX: usize>(
    deserializer: D,
) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T, const MAX: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const MAX: usize> serde::de::Visitor<'de> for Visitor<T, MAX> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("bounded terminal state")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while values.len() < MAX {
                let Some(value) = sequence.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
            if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("terminal state capacity"));
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Visitor::<T, MAX>(std::marker::PhantomData))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum TerminalWaitOutcome {
    Started,
    Exited(i32),
    Signaled(i32),
    ConditionMet,
    SafetyCeiling,
    Cancelled,
    Lost,
}

/// An attention-only wait. Dropping or cancelling this value has no monitor or
/// process side effect. The caller seeds retained-history matches explicitly.
pub(crate) struct TerminalWaitState {
    request: TerminalWaitRequest,
    #[cfg(test)]
    started_at_ms: i64,
    deadline_ms: i64,
    last_now_ms: i64,
    last_output_ms: i64,
    last_cursor: TerminalCursor,
    lifecycle: TerminalLifecycle,
    matched: bool,
    matcher: Option<LiteralMatcher>,
    outcome: Option<TerminalWaitOutcome>,
}

impl TerminalWaitState {
    pub(crate) fn new(
        request: TerminalWaitRequest,
        context: &TerminalMonitorContext,
        last_output_ms: i64,
        already_matched: bool,
    ) -> Result<Self> {
        request
            .validate()
            .map_err(|_| TerminalMonitorError::Invalid)?;
        validate_context(context)?;
        require(last_output_ms >= 0 && last_output_ms <= context.now_ms)?;
        let matcher = match &request.condition {
            TerminalReturnCondition::Match { pattern } => {
                Some(LiteralMatcher::new(pattern.as_bytes()))
            }
            _ => None,
        };
        Ok(Self {
            deadline_ms: deadline(context.now_ms, request.safety_ceiling_ms)?,
            request,
            #[cfg(test)]
            started_at_ms: context.now_ms,
            last_now_ms: context.now_ms,
            last_output_ms,
            last_cursor: context.cursor.clone(),
            lifecycle: context.lifecycle,
            matched: already_matched,
            matcher,
            outcome: None,
        })
    }
    pub(crate) fn output(&mut self, bytes: &[u8], now_ms: i64) -> Result<()> {
        require(bytes.len() <= MAX_MONITOR_FEED_BYTES)?;
        if now_ms < self.last_now_ms {
            return Err(TerminalMonitorError::Clock);
        }
        if self.outcome.is_some() {
            return Ok(());
        }
        self.last_now_ms = now_ms;
        if !bytes.is_empty() {
            self.last_output_ms = now_ms;
            if !self.matched
                && let Some(matcher) = &mut self.matcher
            {
                self.matched = matcher.feed(bytes);
            }
        }
        Ok(())
    }
    pub(crate) fn poll(
        &mut self,
        context: &TerminalMonitorContext,
        process: Option<TerminalProcessOutcome>,
        cancelled: bool,
    ) -> Result<Option<TerminalWaitOutcome>> {
        if let Some(outcome) = self.outcome {
            return Ok(Some(outcome));
        }
        validate_context(context)?;
        if context.now_ms < self.last_now_ms || context.cursor < self.last_cursor {
            return Err(TerminalMonitorError::Clock);
        }
        if let Some(process) = process {
            process.validate()?;
        }
        let outcome = if cancelled {
            Some(TerminalWaitOutcome::Cancelled)
        } else if let Some(process) = process {
            Some(match process {
                TerminalProcessOutcome::Exited(code) => TerminalWaitOutcome::Exited(code),
                TerminalProcessOutcome::Signaled(signal) => TerminalWaitOutcome::Signaled(signal),
            })
        } else if context.lifecycle == TerminalLifecycle::Lost {
            Some(TerminalWaitOutcome::Lost)
        } else {
            let met = context.lifecycle == TerminalLifecycle::Running
                && match &self.request.condition {
                    TerminalReturnCondition::Started => true,
                    TerminalReturnCondition::Exit => false,
                    TerminalReturnCondition::Quiet { duration_ms } => {
                        u64::try_from(context.now_ms - self.last_output_ms)
                            .map_err(|_| TerminalMonitorError::Clock)?
                            >= *duration_ms
                    }
                    TerminalReturnCondition::Match { .. } => self.matched,
                };
            if met {
                Some(
                    if self.request.condition == TerminalReturnCondition::Started {
                        TerminalWaitOutcome::Started
                    } else {
                        TerminalWaitOutcome::ConditionMet
                    },
                )
            } else if context.now_ms >= self.deadline_ms {
                Some(TerminalWaitOutcome::SafetyCeiling)
            } else {
                None
            }
        };
        self.last_now_ms = context.now_ms;
        self.last_cursor = context.cursor.clone();
        self.lifecycle = context.lifecycle;
        self.outcome = outcome;
        Ok(outcome)
    }
    /// Feed already committed journal bytes without replacing their actual
    /// last-output timestamp with the time a bounded owner catch-up reads them.
    /// Condition polling happens separately after reaching the observation
    /// cursor, so an earlier replay page cannot hide a later retained match.
    pub(crate) fn committed_output(
        &mut self,
        bytes: &[u8],
        now_ms: i64,
        last_output_ms: i64,
    ) -> Result<()> {
        require(bytes.len() <= MAX_MONITOR_FEED_BYTES)?;
        if now_ms < self.last_now_ms || last_output_ms < 0 || last_output_ms > now_ms {
            return Err(TerminalMonitorError::Clock);
        }
        if self.outcome.is_none() {
            self.last_now_ms = now_ms;
            self.last_output_ms = last_output_ms;
            if !self.matched
                && let Some(matcher) = &mut self.matcher
            {
                self.matched = matcher.feed(bytes);
            }
        }
        Ok(())
    }
    pub(crate) fn next_deadline(&self) -> Option<i64> {
        if self.outcome.is_some() {
            return None;
        }
        let quiet = match self.request.condition {
            TerminalReturnCondition::Quiet { duration_ms }
                if self.lifecycle == TerminalLifecycle::Running =>
            {
                deadline(self.last_output_ms, duration_ms).ok()
            }
            _ => None,
        };
        Some(quiet.map_or(self.deadline_ms, |quiet| quiet.min(self.deadline_ms)))
    }
    #[cfg(test)]
    pub(crate) fn started_at_ms(&self) -> i64 {
        self.started_at_ms
    }
}

/// Literal wait matching uses KMP to support the full 4096-byte wait pattern
/// without multiplying each input byte by pattern length.
struct LiteralMatcher {
    pattern: Vec<u8>,
    prefix: Vec<usize>,
    position: usize,
}
impl LiteralMatcher {
    fn new(pattern: &[u8]) -> Self {
        let mut prefix = vec![0; pattern.len()];
        let mut matched = 0;
        for index in 1..pattern.len() {
            while matched > 0 && pattern[index] != pattern[matched] {
                matched = prefix[matched - 1];
            }
            if pattern[index] == pattern[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self {
            pattern: pattern.to_vec(),
            prefix,
            position: 0,
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> bool {
        for byte in bytes {
            while self.position > 0 && *byte != self.pattern[self.position] {
                self.position = self.prefix[self.position - 1];
            }
            if *byte == self.pattern[self.position] {
                self.position += 1;
            }
            if self.position == self.pattern.len() {
                self.position = self.prefix[self.position - 1];
                return true;
            }
        }
        false
    }
}

impl TerminalMonitorSet {
    /// A fatal session-end transition may be unable to allocate another event
    /// ID. Native shutdown still must revoke every pending probe and timer.
    pub(crate) fn quiesce(&mut self) {
        self.monitors.clear();
    }

    pub(crate) fn needs_screen(&self) -> bool {
        self.monitors.iter().any(|monitor| {
            monitor.enabled()
                && matches!(
                    monitor.definition.condition,
                    Condition::ScreenMatches { .. }
                )
        })
    }

    pub(crate) fn output(&mut self, bytes: &[u8], context: TerminalMonitorContext) -> Result<()> {
        require(bytes.len() <= MAX_MONITOR_FEED_BYTES)?;
        self.transition(context, |set| {
            if bytes.is_empty() {
                return Ok(());
            }
            let mut index = 0;
            while index < set.monitors.len() {
                if !set.monitors[index].enabled() {
                    index += 1;
                    continue;
                }
                let monitor = &mut set.monitors[index];
                let matched = match &monitor.definition.condition {
                    Condition::OutputQuiet { .. } => {
                        monitor.runtime.next_check_ms =
                            check_deadline(&monitor.definition, set.context.now_ms)?;
                        index += 1;
                        continue;
                    }
                    Condition::OutputContains { .. } | Condition::OutputMatches { .. } => monitor
                        .pattern
                        .as_ref()
                        .ok_or(TerminalMonitorError::InvalidState)?
                        .feed(&mut monitor.runtime.matcher_states, bytes),
                    _ => {
                        index += 1;
                        continue;
                    }
                };
                if !set.observe(index, matched, false, false)? {
                    index += 1;
                }
            }
            Ok(())
        })
    }

    pub(crate) fn screen(
        &mut self,
        screen: &TerminalScreen,
        context: TerminalMonitorContext,
    ) -> Result<()> {
        screen
            .validate()
            .map_err(|_| TerminalMonitorError::Invalid)?;
        let bytes = screen_text(screen)?;
        let count = self
            .monitors
            .iter()
            .filter(|monitor| {
                monitor.enabled()
                    && matches!(
                        monitor.definition.condition,
                        Condition::ScreenMatches { .. }
                    )
            })
            .count();
        if bytes.len().saturating_mul(count) > 16 * 1024 * 1024 {
            return Err(TerminalMonitorError::Capacity);
        }
        self.transition(context, |set| {
            let mut index = 0;
            while index < set.monitors.len() {
                let monitor = &set.monitors[index];
                if !monitor.enabled()
                    || !matches!(
                        monitor.definition.condition,
                        Condition::ScreenMatches { .. }
                    )
                {
                    index += 1;
                    continue;
                }
                let matched = monitor
                    .pattern
                    .as_ref()
                    .ok_or(TerminalMonitorError::InvalidState)?
                    .feed(&mut [0; WORDS], &bytes);
                if !set.observe(index, matched, false, false)? {
                    index += 1;
                }
            }
            Ok(())
        })
    }

    /// Session end always removes persistent monitors. Only trusted process
    /// outcomes can satisfy exit-code/signal conditions; lost is not exit zero.
    pub(crate) fn end_session(
        &mut self,
        outcome: Option<TerminalProcessOutcome>,
        context: TerminalMonitorContext,
    ) -> Result<()> {
        if let Some(outcome) = outcome {
            outcome.validate()?;
        }
        require(matches!(
            context.lifecycle,
            TerminalLifecycle::Exited | TerminalLifecycle::Lost | TerminalLifecycle::Closed
        ))?;
        self.transition(context, |set| {
            while !set.monitors.is_empty() {
                let before = set.next_event_id;
                let condition = &set.monitors[0].definition.condition;
                if let Some(outcome) = outcome.filter(|_| {
                    matches!(
                        condition,
                        Condition::ProcessExit
                            | Condition::ExitCode { .. }
                            | Condition::Signal { .. }
                    )
                }) {
                    let matched = exit_matches(condition, outcome);
                    if set.observe(0, matched, false, false)? {
                        continue;
                    }
                }
                if set.monitors[0].enabled()
                    && set.next_event_id == before
                    && set.monitors[0].definition.notify == Notify::OnExit
                {
                    set.emit(0, Reason::SessionExit)?;
                }
                set.monitors.remove(0);
            }
            Ok(())
        })
    }

    /// Timers coalesce missed periods into one observation; no catch-up loops.
    /// At most one native request per monitor may remain outstanding.
    pub(crate) fn tick(
        &mut self,
        context: TerminalMonitorContext,
    ) -> Result<Vec<TerminalProbeRequest>> {
        self.transition(context, |set| {
            let mut requests = Vec::new();
            let mut index = 0;
            while index < set.monitors.len() {
                if set.expire(index)? {
                    continue;
                }
                if !set.monitors[index].enabled() {
                    index += 1;
                    continue;
                }
                let now = set.context.now_ms;
                let notify = &set.monitors[index].definition.notify;
                if let Notify::Interval { interval_ms } = notify {
                    let due = set.monitors[index]
                        .runtime
                        .next_notification_ms
                        .ok_or(TerminalMonitorError::InvalidState)?;
                    if now >= due {
                        set.monitors[index].runtime.next_notification_ms =
                            Some(advance_deadline(due, *interval_ms, now)?);
                        set.emit(index, Reason::Interval)?;
                    }
                }
                if set.monitors[index]
                    .runtime
                    .pending
                    .as_ref()
                    .is_some_and(|pending| now >= pending.deadline_ms)
                {
                    set.monitors[index].runtime.pending = None;
                    if set.observe(index, false, true, false)? {
                        continue;
                    }
                }
                let monitor = &mut set.monitors[index];
                if monitor.runtime.next_check_ms.is_some_and(|due| now >= due) {
                    if matches!(monitor.definition.condition, Condition::OutputQuiet { .. }) {
                        if set.observe(index, true, false, true)? {
                            continue;
                        }
                    } else if monitor.definition.condition.requires_polling()
                        && monitor.runtime.pending.is_none()
                    {
                        let sequence = set.next_probe_sequence;
                        set.next_probe_sequence = increment(sequence)?;
                        let deadline_ms = deadline(now, PROBE_TIMEOUT_MS)?;
                        let request = TerminalProbeRequest {
                            session_id: set.session_id.clone(),
                            monitor_id: monitor.id.clone(),
                            generation: monitor.runtime.generation,
                            request_sequence: sequence,
                            started_at_ms: now,
                            deadline_ms,
                            output_limit_bytes: PROBE_OUTPUT_BYTES,
                            target: probe_target(monitor)?,
                        };
                        monitor.runtime.pending = Some(PendingProbe {
                            sequence,
                            started_at_ms: now,
                            deadline_ms,
                        });
                        requests.push(request);
                    }
                }
                index += 1;
            }
            Ok(requests)
        })
    }

    /// Stale or wrong-session evidence is ignored before touching clocks,
    /// counters, baselines, or replacement-monitor state.
    pub(crate) fn complete_probe(
        &mut self,
        evidence: TerminalProbeEvidence,
        context: TerminalMonitorContext,
    ) -> Result<bool> {
        if evidence.session_id != self.session_id {
            return Ok(false);
        }
        let Ok(index) = self.index(&evidence.monitor_id) else {
            return Ok(false);
        };
        let monitor = &self.monitors[index];
        if !monitor.enabled()
            || monitor.runtime.generation != evidence.generation
            || monitor
                .runtime
                .pending
                .as_ref()
                .is_none_or(|pending| pending.sequence != evidence.request_sequence)
        {
            return Ok(false);
        }
        self.transition(context, |set| {
            let monitor = &mut set.monitors[index];
            let pending = monitor
                .runtime
                .pending
                .take()
                .ok_or(TerminalMonitorError::InvalidState)?;
            require(
                evidence.completed_at_ms >= pending.started_at_ms
                    && evidence.completed_at_ms <= set.context.now_ms,
            )?;
            let bounded = evidence.completed_at_ms < pending.deadline_ms
                && set.context.now_ms < pending.deadline_ms
                && !evidence.timed_out
                && !evidence.truncated
                && evidence.output_bytes <= PROBE_OUTPUT_BYTES as u64;
            if let Ok(TerminalProbeObservation::Http { response_prefix }) = &evidence.result {
                require(response_prefix.len() as u64 <= evidence.output_bytes)?;
            }
            let matched = if bounded {
                probe_matches(monitor, evidence.result)?
            } else {
                false
            };
            set.observe(index, matched, true, false)?;
            Ok(true)
        })
    }

    pub(crate) fn raw_gap(&mut self, context: TerminalMonitorContext) -> Result<()> {
        self.transition(context, |set| {
            for index in 0..set.monitors.len() {
                let monitor = &mut set.monitors[index];
                if monitor.runtime.state == State::Degraded
                    || !matches!(
                        monitor.definition.condition,
                        Condition::OutputContains { .. }
                            | Condition::OutputMatches { .. }
                            | Condition::OutputQuiet { .. }
                            | Condition::ScreenMatches { .. }
                    )
                {
                    continue;
                }
                monitor.runtime.state = State::Degraded;
                monitor.runtime.generation = increment(monitor.runtime.generation)?;
                monitor.runtime.matcher_states = [0; WORDS];
                monitor.runtime.next_check_ms = None;
                monitor.runtime.next_notification_ms = None;
                monitor.runtime.pending = None;
                set.state_event(index, Reason::StateChanged)?;
            }
            Ok(())
        })
    }

    pub(crate) fn next_deadline(&self) -> Option<i64> {
        self.monitors
            .iter()
            .flat_map(|monitor| {
                let runtime = &monitor.runtime;
                [
                    runtime.lifetime_deadline_ms,
                    monitor
                        .enabled()
                        .then_some(
                            runtime
                                .pending
                                .as_ref()
                                .map_or(runtime.next_check_ms, |pending| Some(pending.deadline_ms)),
                        )
                        .flatten(),
                    monitor
                        .enabled()
                        .then_some(runtime.next_notification_ms)
                        .flatten(),
                ]
            })
            .flatten()
            .min()
    }

    pub(crate) fn events(
        &mut self,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        query
            .validate()
            .map_err(|_| TerminalMonitorError::Invalid)?;
        if let Some(ack) = query.acknowledge_event_id {
            require(ack < self.next_event_id)?;
            self.acknowledged_event_id = self.acknowledged_event_id.max(ack);
        }
        Ok(self
            .events
            .iter()
            .filter(|event| event.event_id > query.after_event_id)
            .take(usize::from(query.max_events))
            .cloned()
            .collect())
    }
    pub(crate) fn dropped_through_event_id(&self) -> u64 {
        self.dropped_through_event_id
    }
    pub(crate) fn acknowledged_event_id(&self) -> u64 {
        self.acknowledged_event_id
    }
    #[cfg(test)]
    pub(crate) fn state(&self, id: &TerminalMonitorId) -> Option<State> {
        self.index(id)
            .ok()
            .map(|index| self.monitors[index].runtime.state)
    }
    pub(crate) fn len(&self) -> usize {
        self.monitors.len()
    }

    /// Public summaries deliberately exclude matcher tails, probe definitions,
    /// and private scheduling snapshots. At most 32 bounded identifiers.
    pub(crate) fn summaries(&self) -> Vec<machine_god_core::TerminalMonitorSummary> {
        self.monitors
            .iter()
            .map(|monitor| machine_god_core::TerminalMonitorSummary {
                monitor_id: monitor.id.clone(),
                state: monitor.runtime.state,
            })
            .collect()
    }

    pub(crate) fn next_event_id(&self) -> u64 {
        self.next_event_id
    }

    #[cfg(test)]
    pub(crate) fn inspect(&self) -> Vec<TerminalMonitorView> {
        self.monitors
            .iter()
            .map(|monitor| {
                let runtime = &monitor.runtime;
                TerminalMonitorView {
                    monitor_id: monitor.id.clone(),
                    generation: runtime.generation,
                    definition: (*monitor.definition).clone(),
                    state: runtime.state,
                    created_at_ms: runtime.created_at_ms,
                    lifetime_deadline_ms: runtime.lifetime_deadline_ms,
                    next_check_ms: runtime.next_check_ms,
                    next_notification_ms: runtime.next_notification_ms,
                    check_count: runtime.check_count,
                    notification_count: runtime.notification_count,
                    last_event_id: runtime.last_event_id,
                    last_event_reason: runtime.last_event_reason,
                    condition_matched: runtime.condition_matched,
                }
            })
            .collect()
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<u8>> {
        let value = Snapshot {
            schema: 1,
            session_id: self.session_id.clone(),
            context: self.context.clone(),
            next_monitor_id: self.next_monitor_id,
            next_event_id: self.next_event_id,
            next_probe_sequence: self.next_probe_sequence,
            acknowledged_event_id: self.acknowledged_event_id,
            dropped_through_event_id: self.dropped_through_event_id,
            monitors: self
                .monitors
                .iter()
                .map(|monitor| SavedMonitor {
                    id: monitor.id.clone(),
                    definition: (*monitor.definition).clone(),
                    runtime: monitor.runtime.clone(),
                })
                .collect(),
            events: self.events.iter().cloned().collect(),
        };
        let mut writer = SnapshotWriter(Vec::new());
        serde_json::to_writer(&mut writer, &value).map_err(|_| TerminalMonitorError::Snapshot)?;
        Ok(writer.0)
    }

    pub(crate) fn restore(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(TerminalMonitorError::Snapshot);
        }
        let saved: Snapshot =
            serde_json::from_slice(bytes).map_err(|_| TerminalMonitorError::Snapshot)?;
        require(
            saved.schema == 1
                && saved.next_monitor_id > 0
                && saved.next_event_id > 0
                && saved.next_probe_sequence > 0
                && saved.acknowledged_event_id < saved.next_event_id
                && saved.dropped_through_event_id < saved.next_event_id,
        )?;
        let mut set = Self::new(saved.session_id, saved.context)?;
        set.next_monitor_id = saved.next_monitor_id;
        set.next_event_id = saved.next_event_id;
        set.next_probe_sequence = saved.next_probe_sequence;
        set.acknowledged_event_id = saved.acknowledged_event_id;
        set.dropped_through_event_id = saved.dropped_through_event_id;
        let mut previous = 0;
        let mut pending_sequences = std::collections::BTreeSet::new();
        for saved in saved.monitors {
            let sequence = monitor_sequence(&saved.id)?;
            require(sequence > previous && sequence < set.next_monitor_id)?;
            previous = sequence;
            validate_definition(&saved.definition)?;
            validate_runtime(&saved.runtime, &saved.definition, &set)?;
            if let Some(pending) = &saved.runtime.pending {
                require(pending_sequences.insert(pending.sequence))?;
            }
            let pattern = condition_pattern(&saved.definition.condition)?;
            if let Some(pattern) = &pattern {
                require(pattern.valid_states(&saved.runtime.matcher_states))?;
            } else {
                require(saved.runtime.matcher_states == [0; WORDS])?;
            }
            set.monitors.push(Monitor {
                id: saved.id,
                definition: Arc::new(saved.definition),
                runtime: saved.runtime,
                pattern,
            });
        }
        let mut previous = set.dropped_through_event_id;
        let mut previous_time = 0;
        let mut previous_cursor =
            TerminalCursor::new(1, 0).map_err(|_| TerminalMonitorError::Snapshot)?;
        for event in saved.events {
            event
                .validate()
                .map_err(|_| TerminalMonitorError::Snapshot)?;
            require(
                event.event_id == increment(previous)?
                    && event.event_id < set.next_event_id
                    && event.created_at_ms >= previous_time
                    && event.created_at_ms <= set.context.now_ms
                    && event.cursor >= previous_cursor
                    && event.cursor <= set.context.cursor
                    && monitor_sequence(&event.monitor_id)? < set.next_monitor_id,
            )?;
            previous = event.event_id;
            previous_time = event.created_at_ms;
            previous_cursor = event.cursor.clone();
            set.events.push_back(event);
        }
        require(increment(previous)? == set.next_event_id)?;
        Ok(set)
    }
}
