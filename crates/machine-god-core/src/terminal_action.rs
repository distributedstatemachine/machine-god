//! Normalized, effect-free terminal requests and results for a trusted host.
//!
//! These serde representations are **not** the public tool argument schema.
//! The tool adapter resolves optional/raw working-directory input with separately
//! granted native authority before constructing a request. Paths here are data:
//! validation does not inspect the filesystem, resolve a shell, start a process,
//! read the environment, run a monitor probe, or grant authority. Session IDs and
//! `next_actions` are descriptive, never process handles or permission grants.
//! The host must bind caller, owner and writer identities outside these values.
//!
//! ```
//! use machine_god_core::{TerminalActionRequest, TerminalStartRequest};
//! // No filesystem or shell lookup: this nonexistent path is prepared data.
//! let request = TerminalActionRequest::Start {
//!     request: TerminalStartRequest::interactive("/prepared/nonexistent")?,
//! };
//! request.validate()?;
//! assert!(request.session_id().is_none());
//! # Ok::<(), machine_god_core::TerminalContractError>(())
//! ```

use core::{fmt, marker::PhantomData, time::Duration};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    MAX_TERMINAL_WRITE_BYTES, TerminalAttentionState, TerminalBackend, TerminalClosePolicy,
    TerminalContractError, TerminalCursor, TerminalDimensions, TerminalEventQuery, TerminalGap,
    TerminalLifecycle, TerminalMonitorDefinition, TerminalMonitorEvent, TerminalMonitorId,
    TerminalMonitorOperation, TerminalMonitorState, TerminalProfile, TerminalReturnCondition,
    TerminalScreen, TerminalScreenUnavailableReason, TerminalSessionId, TerminalSignal,
    TerminalWaitRequest, TerminalWriteRequest,
};

/// Pinned semantic command bound (distinct from legacy foreground-adapter limits).
pub const MAX_TERMINAL_ACTION_COMMAND_BYTES: usize = 64 * 1024;
/// Pinned path and catalog-filter text bound.
pub const MAX_TERMINAL_ACTION_TEXT_BYTES: usize = 4096;
/// Maximum monitor definitions attached to a start.
pub const MAX_TERMINAL_INITIAL_MONITORS: usize = 32;
/// Maximum sessions, monitor summaries, or events in a result page.
pub const MAX_TERMINAL_ACTION_RESULTS: usize = 256;
/// Maximum raw output bytes in one semantic result.
pub const MAX_TERMINAL_ACTION_OUTPUT_BYTES: usize = 1024 * 1024;
/// Maximum checkpoint payload length described by a recovery envelope.
pub const MAX_TERMINAL_CHECKPOINT_PAYLOAD_BYTES: u32 = 64 * 1024 * 1024;
/// Retained bytes per foreground stream, preserving the existing executor contract.
pub const MAX_TERMINAL_EXEC_STREAM_BYTES: usize = 32 * 1024;
/// Existing foreground executor's maximum reported execution duration.
pub const MAX_TERMINAL_EXEC_DURATION: Duration = Duration::from_secs(600);

fn require(valid: bool) -> Result<(), TerminalContractError> {
    if valid {
        Ok(())
    } else {
        Err(TerminalContractError)
    }
}

fn text(value: &str, maximum: usize) -> Result<(), TerminalContractError> {
    require(!value.is_empty() && value.len() <= maximum && !value.contains('\0'))
}

// Prepared native-terminal paths use the Unix absolute-path grammar. This is a
// lexical check, deliberately independent of the OS compiling the core crate.
fn cwd(value: &str) -> Result<(), TerminalContractError> {
    text(value, MAX_TERMINAL_ACTION_TEXT_BYTES)?;
    require(value.starts_with('/'))
}

fn ceiling(value: u64) -> Result<(), TerminalContractError> {
    require(value > 0 && i64::try_from(value).is_ok())
}

macro_rules! redacted {
    ($name:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .finish_non_exhaustive()
            }
        }
    };
}

macro_rules! contract_struct {
    ($(#[$meta:meta])* $name:ident { $($(#[$field_meta:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Eq, PartialEq, Serialize)]
        pub struct $name { $(#[doc = concat!("The ", stringify!($field), " contract field.")] pub $field: $ty),* }
        redacted!($name);
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire { $($(#[$field_meta])* $field: $ty),* }
                let wire = Wire::deserialize(deserializer).map_err(|_| serde::de::Error::custom(TerminalContractError))?;
                let value = Self { $($field: wire.$field),* };
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

macro_rules! contract_enum {
    ($(#[$meta:meta])* $name:ident, $tag:literal { $($variant:ident { $($(#[$field_meta:meta])* $field:ident: $ty:ty),* $(,)? }),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Eq, PartialEq, Serialize)]
        #[serde(tag = $tag, rename_all = "snake_case")]
        pub enum $name { $(#[doc = concat!("The ", stringify!($variant), " form.")] $variant { $(#[doc = concat!("The ", stringify!($field), " field.")] $field: $ty),* }),* }
        redacted!($name);
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(tag = $tag, rename_all = "snake_case", deny_unknown_fields)]
                enum Wire { $($variant { $($(#[$field_meta])* $field: $ty),* }),* }
                let wire = Wire::deserialize(deserializer).map_err(|_| serde::de::Error::custom(TerminalContractError))?;
                let value = match wire { $(Wire::$variant { $($field),* } => Self::$variant { $($field),* }),* };
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

macro_rules! scalar_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $(#[doc = concat!("The ", stringify!($variant), " value.")] $variant),* }
    };
}

fn bounded_vec<'de, D, T, const LIMIT: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> serde::de::Visitor<'de> for Bounded<T, LIMIT> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded terminal action sequence")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element()? {
                if values.len() == LIMIT {
                    return Err(serde::de::Error::custom(TerminalContractError));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Bounded::<T, LIMIT>(PhantomData))
}

scalar_enum! {
    /// Complete terminal action vocabulary, including foreground execution.
    TerminalAction { Exec, Start, Read, Screen, Write, Wait, Monitor, Inspect, List, Resize, Signal, Close }
}

contract_enum! {
    /// Shell selection data; only the native resolver can select an executable.
    TerminalShellSpec, "kind" {
        UserLogin {},
        Executable { path: String, clean_start: bool }
    }
}
impl TerminalShellSpec {
    /// Checks bounded shell data, without looking up the path or accepting a shell as authority.
    ///
    /// # Errors
    /// Rejects empty, oversized, or NUL-containing executable paths.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::UserLogin {} => Ok(()),
            Self::Executable { path, .. } => text(path, MAX_TERMINAL_ACTION_TEXT_BYTES),
        }
    }
}

contract_struct! {
    /// Prepared foreground execution data. The host owns limits and environment resolution.
    TerminalExecRequest { command: String, cwd: String, profile: Option<TerminalProfile> }
}
impl TerminalExecRequest {
    /// Validates prepared execution data.
    ///
    /// # Errors
    /// Rejects malformed commands or a non-absolute working directory.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        text(&self.command, MAX_TERMINAL_ACTION_COMMAND_BYTES)?;
        cwd(&self.cwd)
    }
}

contract_struct! {
    /// Prepared interactive startup, with optional command and initial monitors.
    ///
    /// `cwd` is the host's resolved absolute directory, not raw model input.
    /// An absent profile and shell select the native host's user-profile default.
    /// Explicit profile and shell are mutually exclusive. Starting with a command
    /// requires a return condition; non-immediate conditions require a ceiling.
    TerminalStartRequest {
        cwd: String, command: Option<String>, profile: Option<TerminalProfile>,
        shell: Option<TerminalShellSpec>, backend: TerminalBackend,
        return_when: Option<TerminalReturnCondition>, wait_ceiling_ms: Option<u64>,
        dimensions: Option<TerminalDimensions>,
        #[serde(deserialize_with = "bounded_vec::<_, TerminalMonitorDefinition, MAX_TERMINAL_INITIAL_MONITORS>")]
        initial_monitors: Vec<TerminalMonitorDefinition>
    }
}
impl TerminalStartRequest {
    /// Constructs a default native interactive start with no command.
    ///
    /// # Errors
    /// Rejects a malformed prepared working directory.
    pub fn interactive(cwd: impl Into<String>) -> Result<Self, TerminalContractError> {
        let request = Self {
            cwd: cwd.into(),
            command: None,
            profile: None,
            shell: None,
            backend: TerminalBackend::Native,
            return_when: None,
            wait_ceiling_ms: None,
            dimensions: None,
            initial_monitors: Vec::new(),
        };
        request.validate()?;
        Ok(request)
    }

    /// Validates startup fields and their relationships without native effects.
    ///
    /// # Errors
    /// Rejects malformed fields, conflicting shell/profile selection, missing
    /// command return conditions or wait ceilings, and more than 32 monitors.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        cwd(&self.cwd)?;
        require(self.profile.is_none() || self.shell.is_none())?;
        if let Some(shell) = &self.shell {
            shell.validate()?;
        }
        if let Some(command) = &self.command {
            text(command, MAX_TERMINAL_ACTION_COMMAND_BYTES)?;
            require(self.return_when.is_some())?;
        }
        if let Some(condition) = &self.return_when {
            condition.validate()?;
            require(
                matches!(condition, TerminalReturnCondition::Started)
                    || self.wait_ceiling_ms.is_some(),
            )?;
        }
        if let Some(value) = self.wait_ceiling_ms {
            ceiling(value)?;
        }
        if let Some(dimensions) = &self.dimensions {
            dimensions.validate()?;
        }
        require(self.initial_monitors.len() <= MAX_TERMINAL_INITIAL_MONITORS)?;
        for monitor in &self.initial_monitors {
            monitor.validate()?;
        }
        Ok(())
    }
}

contract_struct! {
    /// Non-authoritative catalog predicates; owner catalog authority is injected separately.
    #[derive(Default)]
    TerminalListFilters {
        task_id: Option<String>, workspace_root: Option<String>,
        lifecycle: Option<TerminalLifecycle>, backend: Option<TerminalBackend>
    }
}
impl TerminalListFilters {
    /// Checks bounded filter data.
    ///
    /// # Errors
    /// Rejects empty, oversized, or NUL-containing filters.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        for value in [&self.task_id, &self.workspace_root].into_iter().flatten() {
            text(value, MAX_TERMINAL_ACTION_TEXT_BYTES)?;
        }
        Ok(())
    }
}

contract_enum! {
    /// Complete normalized action request. Identity and process authority are external.
    TerminalActionRequest, "action" {
        Exec { request: TerminalExecRequest }, Start { request: TerminalStartRequest },
        Read { session_id: TerminalSessionId, cursor: TerminalCursor },
        Screen { session_id: TerminalSessionId },
        Write { session_id: TerminalSessionId, request: TerminalWriteRequest },
        Wait { session_id: TerminalSessionId, request: TerminalWaitRequest },
        Monitor { session_id: TerminalSessionId, operation: TerminalMonitorOperation },
        Inspect { session_id: TerminalSessionId, events: TerminalEventQuery },
        List { filters: TerminalListFilters },
        Resize { session_id: TerminalSessionId, dimensions: TerminalDimensions },
        Signal { session_id: TerminalSessionId, signal: TerminalSignal },
        Close { session_id: TerminalSessionId, policy: TerminalClosePolicy }
    }
}

macro_rules! action_method {
    () => {
        /// Returns this envelope's action discriminator.
        #[must_use]
        pub const fn action(&self) -> TerminalAction {
            match self {
                Self::Exec { .. } => TerminalAction::Exec,
                Self::Start { .. } => TerminalAction::Start,
                Self::Read { .. } => TerminalAction::Read,
                Self::Screen { .. } => TerminalAction::Screen,
                Self::Write { .. } => TerminalAction::Write,
                Self::Wait { .. } => TerminalAction::Wait,
                Self::Monitor { .. } => TerminalAction::Monitor,
                Self::Inspect { .. } => TerminalAction::Inspect,
                Self::List { .. } => TerminalAction::List,
                Self::Resize { .. } => TerminalAction::Resize,
                Self::Signal { .. } => TerminalAction::Signal,
                Self::Close { .. } => TerminalAction::Close,
            }
        }
    };
}

impl TerminalActionRequest {
    action_method!();

    /// Returns the exact requested session, if this action targets an existing session.
    #[must_use]
    pub const fn session_id(&self) -> Option<&TerminalSessionId> {
        match self {
            Self::Exec { .. } | Self::Start { .. } | Self::List { .. } => None,
            Self::Read { session_id, .. }
            | Self::Screen { session_id }
            | Self::Write { session_id, .. }
            | Self::Wait { session_id, .. }
            | Self::Monitor { session_id, .. }
            | Self::Inspect { session_id, .. }
            | Self::Resize { session_id, .. }
            | Self::Signal { session_id, .. }
            | Self::Close { session_id, .. } => Some(session_id),
        }
    }

    /// Validates a complete request without granting authority or performing effects.
    ///
    /// # Errors
    /// Rejects invalid nested contracts and acknowledgements preceding the query cursor.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Exec { request } => request.validate(),
            Self::Start { request } => request.validate(),
            Self::Read { cursor, .. } => cursor.validate(),
            Self::Write { request, .. } => request.validate(),
            Self::Wait { request, .. } => request.validate(),
            Self::Monitor { operation, .. } => operation.validate(),
            Self::Inspect { events, .. } => {
                events.validate()?;
                require(
                    events
                        .acknowledge_event_id
                        .is_none_or(|id| id >= events.after_event_id),
                )
            }
            Self::List { filters } => filters.validate(),
            Self::Resize { dimensions, .. } => dimensions.validate(),
            Self::Screen { .. } | Self::Signal { .. } | Self::Close { .. } => Ok(()),
        }
    }
}

contract_struct! {
    /// Inclusive start and exclusive end in the raw journal; empty ranges are valid.
    TerminalRawRange { start: TerminalCursor, end: TerminalCursor }
}
impl TerminalRawRange {
    /// Checks cursor order; actual byte availability belongs to the store.
    ///
    /// # Errors
    /// Rejects invalid cursors, different segments, or reversed endpoints.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.start.validate()?;
        self.end.validate()?;
        require(self.start.segment() == self.end.segment() && self.start <= self.end)
    }
}

contract_struct! {
    /// A checkpoint descriptor, not its payload or proof that storage is authentic.
    TerminalCheckpointEnvelope {
        engine_schema_revision: u16, applied_cursor: TerminalCursor,
        payload_len: u32, checksum: [u8; 32]
    }
}
impl TerminalCheckpointEnvelope {
    /// Checks the bounded checkpoint header; the native journal verifies payload/checksum.
    ///
    /// # Errors
    /// Rejects zero schema/length, oversized payloads, or invalid cursors.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.applied_cursor.validate()?;
        require(
            self.engine_schema_revision > 0
                && self.payload_len > 0
                && self.payload_len <= MAX_TERMINAL_CHECKPOINT_PAYLOAD_BYTES,
        )
    }
}

contract_enum! {
    /// Recovery facts for the most recent screen checkpoint.
    TerminalScreenRecovery, "kind" {
        Available { checkpoint: TerminalCheckpointEnvelope },
        Unavailable { reason: TerminalScreenUnavailableReason }
    }
}
impl TerminalScreenRecovery {
    /// Checks a recovery descriptor without reading checkpoint storage.
    ///
    /// # Errors
    /// Rejects a malformed available checkpoint header.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Available { checkpoint } => checkpoint.validate(),
            Self::Unavailable { .. } => Ok(()),
        }
    }
}

scalar_enum! {
    /// The pinned runtime's persistence vocabulary.
    TerminalPersistenceLevel { Durable }
}

contract_struct! {
    /// Descriptive UI hints. These booleans are never capabilities or authority grants.
    #[derive(Default)]
    TerminalAllowedControls {
        read: bool, screen: bool, write: bool, wait: bool, monitor: bool,
        inspect: bool, list: bool, resize: bool, signal: bool, close: bool
    }
}
impl TerminalAllowedControls {
    /// Checks descriptive controls; authority is deliberately not validated here.
    ///
    /// # Errors
    /// This closed boolean vocabulary has no invalid combinations.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        Ok(())
    }
}

contract_struct! {
    /// Bounded descriptive session facts returned by the separately authorized host.
    TerminalSessionFacts {
        session_id: TerminalSessionId, lifecycle: TerminalLifecycle, attention: TerminalAttentionState,
        backend: TerminalBackend, persistence: TerminalPersistenceLevel,
        output_cursor: TerminalCursor, unread_range: Option<TerminalRawRange>, raw_gap: Option<TerminalGap>,
        screen_recovery: TerminalScreenRecovery, active_monitor_count: u16, next_actions: TerminalAllowedControls
    }
}
impl TerminalSessionFacts {
    /// Checks facts, retained ranges and checkpoint anchoring against the output cursor.
    ///
    /// # Errors
    /// Rejects invalid nested data or ranges/checkpoints extending beyond available output.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.attention.validate()?;
        self.output_cursor.validate()?;
        self.screen_recovery.validate()?;
        self.next_actions.validate()?;
        if let Some(range) = &self.unread_range {
            range.validate()?;
            require(range.end <= self.output_cursor)?;
        }
        if let Some(gap) = &self.raw_gap {
            gap.validate()?;
            require(gap.available_from <= self.output_cursor)?;
        }
        if let TerminalScreenRecovery::Available { checkpoint } = &self.screen_recovery {
            require(
                checkpoint.applied_cursor.segment() == self.output_cursor.segment()
                    && checkpoint.applied_cursor <= self.output_cursor,
            )?;
        }
        Ok(())
    }
}

contract_enum! {
    /// Observed return status; cancellation does not imply session termination.
    TerminalReturnOutcome, "kind" {
        Started {}, ConditionMet {}, SafetyCeiling {}, Cancelled {},
        Exited { exit_code: i32 }, Signal { signal: u32 }
    }
}
impl TerminalReturnOutcome {
    /// Checks the portable exit/signal range used by the pinned terminal contract.
    ///
    /// # Errors
    /// Rejects exit codes outside 0..=255 and signals outside 1..=255.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Exited { exit_code } => require((0..=255).contains(exit_code)),
            Self::Signal { signal } => require((1..=255).contains(signal)),
            _ => Ok(()),
        }
    }
}

contract_struct! {
    /// One retained monitor's identity and descriptive state.
    TerminalMonitorSummary { monitor_id: TerminalMonitorId, state: TerminalMonitorState }
}
impl TerminalMonitorSummary {
    /// Validates the summary (its identifier and state are already closed types).
    ///
    /// # Errors
    /// No additional cross-field constraints apply.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        Ok(())
    }
}

contract_enum! {
    /// Foreground termination reasons, preserving timeout versus output-limit termination.
    TerminalExecStatus, "kind" {
        Exited { exit_code: i32 }, Signaled { signal: i32 }, TimedOut {}, OutputLimit {}
    }
}
impl TerminalExecStatus {
    /// Validates the existing portable foreground process status ranges.
    ///
    /// # Errors
    /// Rejects exit codes outside 0..=255 or signals outside 1..=255.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Exited { exit_code } => require((0..=255).contains(exit_code)),
            Self::Signaled { signal } => require((1..=255).contains(signal)),
            Self::TimedOut {} | Self::OutputLimit {} => Ok(()),
        }
    }
}

contract_struct! {
    /// Bounded foreground head/tail output and its exact observed produced-byte count.
    TerminalExecCapturedOutput {
        #[serde(deserialize_with = "bounded_vec::<_, u8, MAX_TERMINAL_EXEC_STREAM_BYTES>")]
        bytes: Vec<u8>, total_bytes: u64
    }
}
impl TerminalExecCapturedOutput {
    /// Validates retained and produced byte counts.
    ///
    /// # Errors
    /// Rejects oversized retained data or a total below the retained length.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require(
            self.bytes.len() <= MAX_TERMINAL_EXEC_STREAM_BYTES
                && self.total_bytes >= self.bytes.len() as u64,
        )
    }

    /// Whether bytes are omitted from this retained stream.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.total_bytes > self.bytes.len() as u64
    }
}

contract_struct! {
    /// Lossless effect-free counterpart of the native foreground executor outcome.
    TerminalExecResult {
        status: TerminalExecStatus, stdout: TerminalExecCapturedOutput,
        stderr: TerminalExecCapturedOutput, duration: Duration
    }
}
impl TerminalExecResult {
    /// Validates the existing foreground stream, duration and output-limit invariants.
    ///
    /// # Errors
    /// Rejects malformed streams/status, excessive duration, or output-limit disagreement.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.status.validate()?;
        self.stdout.validate()?;
        self.stderr.validate()?;
        require(self.duration <= MAX_TERMINAL_EXEC_DURATION)?;
        require(
            matches!(self.status, TerminalExecStatus::OutputLimit {})
                == (self
                    .stdout
                    .total_bytes
                    .saturating_add(self.stderr.total_bytes)
                    > MAX_TERMINAL_ACTION_OUTPUT_BYTES as u64),
        )
    }
}

contract_enum! {
    /// Complete bounded successful action result. See `validate_for` before delivering a reply.
    #[allow(clippy::large_enum_variant)]
    TerminalActionResult, "action" {
        Exec { result: TerminalExecResult },
        Start { session: TerminalSessionFacts, outcome: TerminalReturnOutcome },
        Read { session: TerminalSessionFacts,
            #[serde(deserialize_with = "bounded_vec::<_, u8, MAX_TERMINAL_ACTION_OUTPUT_BYTES>")]
            output: Vec<u8>, raw_range: Option<TerminalRawRange> },
        Screen { session: TerminalSessionFacts, snapshot: TerminalScreen },
        Write { session: TerminalSessionFacts, accepted_bytes: u32 },
        Wait { session: TerminalSessionFacts, outcome: TerminalReturnOutcome },
        Monitor { session: TerminalSessionFacts, monitor_id: Option<TerminalMonitorId> },
        Inspect { session: TerminalSessionFacts, shell: String, cwd: String, command: Option<String>,
            #[serde(deserialize_with = "bounded_vec::<_, TerminalMonitorSummary, MAX_TERMINAL_ACTION_RESULTS>")]
            monitors: Vec<TerminalMonitorSummary>,
            #[serde(deserialize_with = "bounded_vec::<_, TerminalMonitorEvent, MAX_TERMINAL_ACTION_RESULTS>")]
            events: Vec<TerminalMonitorEvent>, event_gap_through: u64, next_event_id: u64 },
        List {
            #[serde(deserialize_with = "bounded_vec::<_, TerminalSessionFacts, MAX_TERMINAL_ACTION_RESULTS>")]
            sessions: Vec<TerminalSessionFacts> },
        Resize { session: TerminalSessionFacts, dimensions: TerminalDimensions },
        Signal { session: TerminalSessionFacts, signal: TerminalSignal },
        Close { session: TerminalSessionFacts, policy: TerminalClosePolicy }
    }
}
impl TerminalActionResult {
    action_method!();

    /// Returns single-session facts, absent for foreground execution and catalog pages.
    #[must_use]
    pub const fn session(&self) -> Option<&TerminalSessionFacts> {
        match self {
            Self::Exec { .. } | Self::List { .. } => None,
            Self::Start { session, .. }
            | Self::Read { session, .. }
            | Self::Screen { session, .. }
            | Self::Write { session, .. }
            | Self::Wait { session, .. }
            | Self::Monitor { session, .. }
            | Self::Inspect { session, .. }
            | Self::Resize { session, .. }
            | Self::Signal { session, .. }
            | Self::Close { session, .. } => Some(session),
        }
    }

    /// Validates bounded result structure and intra-result cursor/event relationships.
    ///
    /// # Errors
    /// Rejects malformed facts, output, screens, monitor/event pages or return outcomes.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        if let Some(session) = self.session() {
            session.validate()?;
        }
        match self {
            Self::Exec { result } => result.validate(),
            Self::Start { outcome, .. } | Self::Wait { outcome, .. } => outcome.validate(),
            Self::Read {
                session,
                output,
                raw_range,
            } => {
                require(output.len() <= MAX_TERMINAL_ACTION_OUTPUT_BYTES)?;
                if let Some(range) = raw_range {
                    range.validate()?;
                    require(range.end <= session.output_cursor)?;
                }
                Ok(())
            }
            Self::Screen { snapshot, .. } => snapshot.validate(),
            Self::Write { accepted_bytes, .. } => {
                require(u64::from(*accepted_bytes) <= MAX_TERMINAL_WRITE_BYTES as u64)
            }
            Self::Inspect {
                shell,
                cwd,
                command,
                monitors,
                events,
                event_gap_through,
                next_event_id,
                ..
            } => {
                text(shell, MAX_TERMINAL_ACTION_TEXT_BYTES)?;
                text(cwd, MAX_TERMINAL_ACTION_TEXT_BYTES)?;
                if let Some(command) = command {
                    text(command, MAX_TERMINAL_ACTION_COMMAND_BYTES)?;
                }
                require(
                    monitors.len() <= MAX_TERMINAL_ACTION_RESULTS
                        && events.len() <= MAX_TERMINAL_ACTION_RESULTS
                        && *next_event_id > 0
                        && event_gap_through < next_event_id,
                )?;
                let mut previous = 0;
                for event in events {
                    event.validate()?;
                    require(event.event_id > previous && event.event_id < *next_event_id)?;
                    previous = event.event_id;
                }
                Ok(())
            }
            Self::List { sessions } => {
                require(sessions.len() <= MAX_TERMINAL_ACTION_RESULTS)?;
                for session in sessions {
                    session.validate()?;
                }
                Ok(())
            }
            Self::Resize { dimensions, .. } => dimensions.validate(),
            Self::Monitor { .. } | Self::Signal { .. } | Self::Close { .. } => Ok(()),
        }
    }

    /// Checks a reply against its request as well as its internal structure.
    ///
    /// This does not establish caller ownership, incarnation, process identity,
    /// successful persistence, or authority; the native host must establish those.
    ///
    /// # Errors
    /// Rejects action/session mismatches, differing resize/signal/close receipts,
    /// payload-free lease receipts reporting bytes, and inconsistent event pages.
    pub fn validate_for(
        &self,
        request: &TerminalActionRequest,
    ) -> Result<(), TerminalContractError> {
        request.validate()?;
        self.validate()?;
        require(self.action() == request.action())?;
        if let Some(id) = request.session_id() {
            require(
                self.session()
                    .is_some_and(|session| &session.session_id == id),
            )?;
        }
        match (self, request) {
            (Self::Start { session, .. }, TerminalActionRequest::Start { request }) => {
                require(session.backend == request.backend)
            }
            (
                Self::Resize { dimensions, .. },
                TerminalActionRequest::Resize {
                    dimensions: requested,
                    ..
                },
            ) => require(dimensions == requested),
            (
                Self::Signal { signal, .. },
                TerminalActionRequest::Signal {
                    signal: requested, ..
                },
            ) => require(signal == requested),
            (
                Self::Close { policy, .. },
                TerminalActionRequest::Close {
                    policy: requested, ..
                },
            ) => require(policy == requested),
            (Self::Write { accepted_bytes, .. }, TerminalActionRequest::Write { request, .. }) => {
                require(request.payload.is_some() || *accepted_bytes == 0)
            }
            (
                Self::Inspect { events, .. },
                TerminalActionRequest::Inspect { events: query, .. },
            ) => require(
                events.len() <= usize::from(query.max_events)
                    && events
                        .iter()
                        .all(|event| event.event_id > query.after_event_id),
            ),
            _ => Ok(()),
        }
    }
}

scalar_enum! {
    /// Closed, data-free host failure reasons from the pinned terminal protocol.
    TerminalActionErrorCode {
        InvalidRequest, PathOutsideWorkspace, UnsupportedHost, ShellUnavailable, PtyUnavailable,
        StartupFailed, ProcessIdentityUnavailable, SessionLost, SessionNotFound, InvalidLifecycle,
        AuthorityDenied, LeaseConflict, CursorGap, ScreenUnavailable, MonitorUnavailable,
        ProtocolIncompatible, CapacityExceeded, Cancelled
    }
}

contract_enum! {
    /// A successful semantic result or a structured, non-echoing host failure.
    TerminalActionResponse, "status" {
        Success { result: Box<TerminalActionResult> },
        Failure { action: TerminalAction, code: TerminalActionErrorCode, session_id: Option<TerminalSessionId>, retryable: bool }
    }
}
impl TerminalActionResponse {
    /// Validates the successful result or closed failure fields.
    ///
    /// # Errors
    /// Rejects a malformed successful result.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Success { result } => result.validate(),
            Self::Failure { .. } => Ok(()),
        }
    }

    /// Validates response-to-request action/session consistency without granting authority.
    ///
    /// # Errors
    /// Rejects inconsistent success or failure identities and malformed requests.
    pub fn validate_for(
        &self,
        request: &TerminalActionRequest,
    ) -> Result<(), TerminalContractError> {
        request.validate()?;
        match self {
            Self::Success { result } => result.validate_for(request),
            Self::Failure {
                action, session_id, ..
            } => {
                require(*action == request.action())?;
                // Pinned native_session.start can fail after allocating a durable
                // session and returns that newly created ID with the failure.
                require(session_id.as_ref().is_none_or(|id| {
                    *action == TerminalAction::Start || Some(id) == request.session_id()
                }))
            }
        }
    }
}
