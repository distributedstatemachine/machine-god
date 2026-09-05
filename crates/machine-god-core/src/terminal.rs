//! Effect-free, bounded terminal session contracts.

use core::fmt;
use core::marker::PhantomData;
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum bytes in a terminal identifier.
pub const MAX_TERMINAL_ID_BYTES: usize = 255;
/// Maximum UTF-8 bytes in an opaque monitor identifier.
pub const MAX_TERMINAL_MONITOR_ID_BYTES: usize = 128;
/// Maximum bytes in text or paste input.
pub const MAX_TERMINAL_WRITE_BYTES: usize = 64 * 1024;
/// Maximum named keys or control characters in one write.
pub const MAX_TERMINAL_WRITE_ITEMS: usize = 4096;
/// Maximum bytes in a monitor's streaming pattern.
pub const MAX_TERMINAL_MONITOR_PATTERN_BYTES: usize = 256;
/// Maximum cells in one rendered screen.
pub const MAX_TERMINAL_SCREEN_CELLS: usize = 262_144;
/// Maximum aggregate cell-text bytes in one screen.
pub const MAX_TERMINAL_SCREEN_TEXT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum hyperlink table entries in one screen.
pub const MAX_TERMINAL_HYPERLINKS: usize = 65_535;
/// Maximum aggregate hyperlink URI bytes in one screen.
pub const MAX_TERMINAL_HYPERLINK_BYTES: usize = 4 * 1024 * 1024;
const MAX_DAY_MS: u64 = 86_400_000;
const MAX_YEAR_MS: u64 = 365 * MAX_DAY_MS;

/// Fixed, data-free structural validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalContractError;

impl fmt::Display for TerminalContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid terminal contract")
    }
}

impl std::error::Error for TerminalContractError {}

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

// The wire mirror prevents deserialization from bypassing cross-field validation.
// Error mapping never repeats malformed caller-controlled values.
macro_rules! contract_struct {
    ($(#[$meta:meta])* $name:ident { $($(#[$field_meta:meta])* $visibility:vis $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Eq, PartialEq, Serialize)]
        pub struct $name { $(#[doc = concat!("The ", stringify!($field), " contract field.")] $visibility $field: $ty),* }
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

fn bounded_vec<'de, D, T, const LIMIT: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> serde::de::Visitor<'de> for Bounded<T, LIMIT> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded terminal sequence")
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

fn hyperlink_table<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<TerminalHyperlink>, D::Error> {
    struct Table;
    impl<'de> serde::de::Visitor<'de> for Table {
        type Value = Vec<TerminalHyperlink>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded terminal hyperlink table")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            let mut bytes = 0_usize;
            let mut previous_id = 0;
            while let Some(value) = sequence.next_element::<TerminalHyperlink>()? {
                bytes = bytes
                    .checked_add(value.uri.len())
                    .ok_or_else(|| serde::de::Error::custom(TerminalContractError))?;
                if values.len() == MAX_TERMINAL_HYPERLINKS
                    || bytes > MAX_TERMINAL_HYPERLINK_BYTES
                    || value.id <= previous_id
                {
                    return Err(serde::de::Error::custom(TerminalContractError));
                }
                previous_id = value.id;
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Table)
}

macro_rules! contract_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident $( { $($field:ident: $ty:ty),* $(,)? } )?),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Eq, PartialEq, Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum $name { $(#[doc = concat!("The ", stringify!($variant), " form.")] $variant $( { $(#[doc = concat!("The ", stringify!($field), " field.")] $field: $ty),* } )?),* }
        redacted!($name);
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
                enum Wire { $($variant { $($($field: $ty),*)? }),* }
                let wire = Wire::deserialize(deserializer).map_err(|_| serde::de::Error::custom(TerminalContractError))?;
                let value = match wire { $(Wire::$variant { $($($field),*)? } => Self::$variant $( { $($field),* } )?),* };
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

macro_rules! identifier {
    ($name:ident, $maximum:expr, $valid:expr) => {
        #[doc = concat!("A validated, non-authoritative ", stringify!($name), ".")]
        #[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);
        redacted!($name);
        impl $name {
            /// Validates a portable identifier without exercising authority.
            ///
            /// # Errors
            /// Rejects identifiers outside this type's length and alphabet contract.
            pub fn new(value: impl Into<String>) -> Result<Self, TerminalContractError> {
                let value = value.into();
                require(!value.is_empty() && value.len() <= $maximum && ($valid)(&value))?;
                Ok(Self(value))
            }
            /// Returns the validated identifier spelling, never authority.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)
                    .map_err(|_| serde::de::Error::custom(TerminalContractError))?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier!(
    TerminalSessionId,
    MAX_TERMINAL_ID_BYTES,
    |value: &str| value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
);
identifier!(
    TerminalMonitorId,
    MAX_TERMINAL_MONITOR_ID_BYTES,
    |value: &str| !value.contains('\0')
);

contract_struct! {
    /// Positive terminal dimensions with an independent aggregate cell bound.
    TerminalDimensions { rows: u16, columns: u16 }
}
impl TerminalDimensions {
    /// Constructs dimensions.
    ///
    /// # Errors
    /// Rejects zero, dimensions over 4096, or excessive aggregate cells.
    pub fn new(rows: u16, columns: u16) -> Result<Self, TerminalContractError> {
        let value = Self { rows, columns };
        value.validate()?;
        Ok(value)
    }
    /// Returns rows.
    #[must_use]
    pub const fn rows(&self) -> u16 {
        self.rows
    }
    /// Returns columns.
    #[must_use]
    pub const fn columns(&self) -> u16 {
        self.columns
    }
    /// Validates dimensions.
    ///
    /// # Errors
    /// Rejects an excessive or empty grid.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require(
            self.rows > 0
                && self.columns > 0
                && self.rows <= 4096
                && self.columns <= 4096
                && usize::from(self.rows) * usize::from(self.columns) <= MAX_TERMINAL_SCREEN_CELLS,
        )
    }
}

contract_struct! {
    /// Position in a durable segmented byte journal; not a live process handle.
    TerminalCursor { segment: u64, offset: u64 }
}
impl TerminalCursor {
    /// Constructs a cursor.
    ///
    /// # Errors
    /// Rejects reserved segment zero.
    pub fn new(segment: u64, offset: u64) -> Result<Self, TerminalContractError> {
        let value = Self { segment, offset };
        value.validate()?;
        Ok(value)
    }
    /// Returns the segment.
    #[must_use]
    pub const fn segment(&self) -> u64 {
        self.segment
    }
    /// Returns the byte offset.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }
    /// Validates the cursor.
    ///
    /// # Errors
    /// Rejects segment zero; the store separately validates availability.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require(self.segment != 0)
    }
}
impl Ord for TerminalCursor {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (self.segment, self.offset).cmp(&(other.segment, other.offset))
    }
}
impl PartialOrd for TerminalCursor {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

contract_struct! {
    /// Explicit unavailable interval in the raw journal.
    TerminalGap { pub missing_from: TerminalCursor, pub available_from: TerminalCursor }
}
impl TerminalGap {
    /// Constructs a gap.
    ///
    /// # Errors
    /// Rejects empty or reversed intervals.
    pub fn new(
        missing_from: TerminalCursor,
        available_from: TerminalCursor,
    ) -> Result<Self, TerminalContractError> {
        let value = Self {
            missing_from,
            available_from,
        };
        value.validate()?;
        Ok(value)
    }
    /// Validates the interval.
    ///
    /// # Errors
    /// Rejects malformed cursors and non-increasing endpoints.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.missing_from.validate()?;
        self.available_from.validate()?;
        require(self.missing_from < self.available_from)
    }
}

macro_rules! scalar_enum {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[doc = concat!("Closed ", stringify!($name), " vocabulary.")]
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $(#[doc = concat!("The ", stringify!($variant), " value.")] $variant),* }
    };
}
scalar_enum!(TerminalLifecycle {
    Starting,
    Running,
    Exited,
    Lost,
    Closed
});
scalar_enum!(TerminalBackend { Native, Tmux });
scalar_enum!(TerminalProfile { User, Clean });
scalar_enum!(TerminalWriteLeaseIntent {
    Acquire,
    Use,
    Release,
    Revoke
});
scalar_enum!(TerminalSignal {
    Hangup,
    Interrupt,
    Quit,
    Terminate,
    Kill
});
scalar_enum!(TerminalClosePolicy { Graceful, Force });
scalar_enum!(TerminalNamedKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown
});

contract_enum! {
    /// Semantic input; encoding and terminal effects belong to native.
    TerminalWritePayload {
        Text { text: String }, Paste { text: String },
        Keys { keys: Vec<TerminalNamedKey> }, Controls { controls: Vec<u8> }
    }
}
impl TerminalWritePayload {
    /// Validates bounded input.
    ///
    /// # Errors
    /// Rejects empty/oversized payloads or invalid control-character spellings.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Text { text } | Self::Paste { text } => {
                require(!text.is_empty() && text.len() <= MAX_TERMINAL_WRITE_BYTES)
            }
            Self::Keys { keys } => {
                require(!keys.is_empty() && keys.len() <= MAX_TERMINAL_WRITE_ITEMS)
            }
            Self::Controls { controls } => require(
                !controls.is_empty()
                    && controls.len() <= MAX_TERMINAL_WRITE_ITEMS
                    && controls
                        .iter()
                        .all(|b| (b'@'..=b'_').contains(b) || b.is_ascii_lowercase() || *b == b'?'),
            ),
        }
    }
}
contract_struct! {
    /// A payload write or a payload-free lease operation.
    TerminalWriteRequest { pub lease: TerminalWriteLeaseIntent, pub payload: Option<TerminalWritePayload> }
}
impl TerminalWriteRequest {
    /// Validates lease/payload agreement.
    ///
    /// # Errors
    /// Only `Use` accepts and requires a payload.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require((self.lease == TerminalWriteLeaseIntent::Use) == self.payload.is_some())?;
        if let Some(payload) = &self.payload {
            payload.validate()?;
        }
        Ok(())
    }
}

contract_enum! {
    /// An immediate or observed return condition, independent of process ownership.
    TerminalReturnCondition { Started, Exit, Quiet { duration_ms: u64 }, Match { pattern: String } }
}
impl TerminalReturnCondition {
    /// Validates a return condition.
    ///
    /// # Errors
    /// Rejects zero quiet duration or empty/oversized/NUL-containing patterns.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Started | Self::Exit => Ok(()),
            Self::Quiet { duration_ms } => require(*duration_ms > 0),
            Self::Match { pattern } => text(pattern, 4096),
        }
    }
}
contract_struct! {
    /// A return condition and positive safety ceiling, measured by an injected clock.
    TerminalWaitRequest { pub condition: TerminalReturnCondition, pub safety_ceiling_ms: u64 }
}
impl TerminalWaitRequest {
    /// Validates a wait.
    ///
    /// # Errors
    /// Rejects invalid conditions or zero/nonrepresentable millisecond ceilings.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.condition.validate()?;
        require(self.safety_ceiling_ms > 0 && i64::try_from(self.safety_ceiling_ms).is_ok())
    }
}

contract_enum! {
    /// Complete terminal-monitor observation vocabulary; this value grants no probe authority.
    TerminalMonitorCondition {
        ProcessExit, ExitCode { exit_code: i32 }, Signal { signal: TerminalSignal },
        OutputContains { pattern: String }, OutputMatches { pattern: String },
        OutputQuiet { duration_ms: u64 }, ScreenMatches { pattern: String },
        TcpReady { host: String, port: u16 }, HttpReady { url: String },
        PathExists { path: String }, PathChanged { path: String },
        PathSize { path: String, minimum_bytes: u64 },
        CustomProbe { command: String, cwd: String }
    }
}
impl TerminalMonitorCondition {
    /// Whether the condition needs a separately authorized periodic native probe.
    #[must_use]
    pub const fn requires_polling(&self) -> bool {
        matches!(
            self,
            Self::TcpReady { .. }
                | Self::HttpReady { .. }
                | Self::PathExists { .. }
                | Self::PathChanged { .. }
                | Self::PathSize { .. }
                | Self::CustomProbe { .. }
        )
    }
    /// Validates only structural bounds, not path/network/command authority.
    ///
    /// # Errors
    /// Rejects out-of-range codes, durations, ports and text bounds.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::ProcessExit | Self::Signal { .. } => Ok(()),
            Self::ExitCode { exit_code } => require((0..=255).contains(exit_code)),
            Self::OutputContains { pattern }
            | Self::OutputMatches { pattern }
            | Self::ScreenMatches { pattern } => text(pattern, MAX_TERMINAL_MONITOR_PATTERN_BYTES),
            Self::OutputQuiet { duration_ms } => {
                require(*duration_ms > 0 && *duration_ms <= MAX_DAY_MS)
            }
            Self::TcpReady { host, port } => {
                text(host, 4096)?;
                require(*port != 0)
            }
            Self::HttpReady { url } => text(url, 4096),
            Self::PathExists { path }
            | Self::PathChanged { path }
            | Self::PathSize { path, .. } => text(path, 4096),
            Self::CustomProbe { command, cwd } => {
                text(command, 64 * 1024)?;
                text(cwd, 4096)
            }
        }
    }
}
contract_struct! {
    /// Positive bounded polling schedule in milliseconds.
    TerminalSchedule { pub interval_ms: u64 }
}
impl TerminalSchedule {
    /// Validates a schedule.
    ///
    /// # Errors
    /// Rejects intervals outside 10 milliseconds through one day.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require((10..=MAX_DAY_MS).contains(&self.interval_ms))
    }
}
contract_enum! {
    /// Monitor notification scheduling, separate from check scheduling.
    TerminalNotifySchedule { OnMatch, OnStateChange, OnExit, EveryCheck, EveryNChecks { count: u32 }, Interval { interval_ms: u64 } }
}
impl TerminalNotifySchedule {
    /// Validates the schedule.
    ///
    /// # Errors
    /// Rejects zero counts or out-of-range intervals.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::EveryNChecks { count } => require(*count > 0),
            Self::Interval { interval_ms } => TerminalSchedule {
                interval_ms: *interval_ms,
            }
            .validate(),
            _ => Ok(()),
        }
    }
}
contract_enum! {
    /// Monitor lifetime; ending attention does not imply ending the process.
    TerminalMonitorLifetime { UntilMatch, UntilSessionEnd, Duration { duration_ms: u64 } }
}
impl TerminalMonitorLifetime {
    /// Validates lifetime bounds.
    ///
    /// # Errors
    /// Rejects durations outside one millisecond through 365 days.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Duration { duration_ms } => require((1..=MAX_YEAR_MS).contains(duration_ms)),
            _ => Ok(()),
        }
    }
}
contract_struct! {
    /// One bounded monitor definition.
    TerminalMonitorDefinition {
        pub condition: TerminalMonitorCondition, pub check_schedule: Option<TerminalSchedule>,
        pub notify: TerminalNotifySchedule, pub lifetime: TerminalMonitorLifetime
    }
}
impl TerminalMonitorDefinition {
    /// Validates the definition.
    ///
    /// # Errors
    /// Probe conditions require a schedule; event-driven conditions forbid one.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.condition.validate()?;
        self.notify.validate()?;
        self.lifetime.validate()?;
        require(self.condition.requires_polling() == self.check_schedule.is_some())?;
        if let Some(schedule) = &self.check_schedule {
            schedule.validate()?;
        }
        Ok(())
    }
}
contract_enum! {
    /// Complete monitor mutation vocabulary.
    TerminalMonitorOperation {
        Add { definition: TerminalMonitorDefinition }, Update { monitor_id: TerminalMonitorId, definition: TerminalMonitorDefinition },
        Pause { monitor_id: TerminalMonitorId }, Resume { monitor_id: TerminalMonitorId }, Remove { monitor_id: TerminalMonitorId }
    }
}
impl TerminalMonitorOperation {
    /// Validates a monitor operation.
    ///
    /// # Errors
    /// Rejects malformed definitions.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        match self {
            Self::Add { definition } | Self::Update { definition, .. } => definition.validate(),
            _ => Ok(()),
        }
    }
}
scalar_enum!(TerminalMonitorState {
    Active,
    Paused,
    Matched,
    Degraded
});
scalar_enum!(TerminalMonitorEventReason {
    Matched,
    StateChanged,
    SessionExit,
    Check,
    Interval,
    Expired,
    Removed,
    Paused,
    Resumed,
    Updated
});
contract_struct! {
    /// One durable monitor event with a nonzero monotonic display sequence.
    TerminalMonitorEvent {
        pub event_id: u64, pub monitor_id: TerminalMonitorId, pub reason: TerminalMonitorEventReason,
        pub lifecycle: TerminalLifecycle, pub cursor: TerminalCursor, pub created_at_ms: i64
    }
}
impl TerminalMonitorEvent {
    /// Validates event structure.
    ///
    /// # Errors
    /// Rejects event zero or malformed cursors; storage validates monotonic history.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require(self.event_id > 0)?;
        self.cursor.validate()
    }
}
contract_struct! {
    /// Bounded event replay with an optional acknowledgement of previously seen events.
    TerminalEventQuery { pub after_event_id: u64, pub acknowledge_event_id: Option<u64>, pub max_events: u16 }
}
impl TerminalEventQuery {
    /// Validates query bounds.
    ///
    /// # Errors
    /// Rejects zero acknowledgements and event limits outside 1 through 256.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require((1..=256).contains(&self.max_events) && self.acknowledge_event_id != Some(0))
    }
}

scalar_enum!(TerminalCursorShape {
    Block,
    Underline,
    Bar
});
scalar_enum!(TerminalCellKind {
    Blank,
    Single,
    Wide,
    Continuation
});
scalar_enum!(TerminalScreenUnavailableReason {
    Missing,
    Corrupt,
    UnsupportedSchema,
    RetentionEvicted,
    RawGap,
    ResizeUncheckpointed
});
contract_enum! {
    /// A default, palette-indexed or RGB cell color.
    TerminalColor { Default, Indexed { index: u8 }, Rgb { red: u8, green: u8, blue: u8 } }
}
impl TerminalColor {
    /// Validates a color.
    ///
    /// # Errors
    /// All representable color values are valid.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        Ok(())
    }
}
impl Copy for TerminalColor {}
contract_struct! {
    /// Zero-based cursor projection. Its containing screen validates bounds.
    TerminalScreenCursor { pub row: u16, pub column: u16, pub visible: bool, pub shape: TerminalCursorShape, pub blinking: bool }
}
impl TerminalScreenCursor {
    /// Validates intrinsic cursor bounds.
    ///
    /// # Errors
    /// Rejects positions outside the maximum supported grid.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require(self.row < 4096 && self.column < 4096)
    }
}
contract_struct! {
    /// Independent terminal display and input modes.
    #[allow(clippy::struct_excessive_bools)]
    TerminalModes {
        pub alternate_screen: bool, pub origin: bool, pub autowrap: bool, pub insert: bool,
        pub bracketed_paste: bool, pub mouse_tracking: bool, pub focus_tracking: bool,
        pub application_cursor_keys: bool, pub application_keypad: bool, pub keyboard_protocol: bool, pub synchronized_updates: bool
    }
}
impl TerminalModes {
    /// Validates modes.
    ///
    /// # Errors
    /// All mode combinations are representable.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        Ok(())
    }
}
impl Default for TerminalModes {
    fn default() -> Self {
        Self {
            alternate_screen: false,
            origin: false,
            autowrap: true,
            insert: false,
            bracketed_paste: false,
            mouse_tracking: false,
            focus_tracking: false,
            application_cursor_keys: false,
            application_keypad: false,
            keyboard_protocol: false,
            synchronized_updates: false,
        }
    }
}
contract_struct! {
    /// Presentation attributes for one terminal cell.
    #[allow(clippy::struct_excessive_bools)]
    TerminalCellStyle {
        pub foreground: TerminalColor, pub background: TerminalColor,
        pub bold: bool, pub faint: bool, pub italic: bool, pub underline: bool, pub inverse: bool, pub strikethrough: bool
    }
}
impl TerminalCellStyle {
    /// Validates attributes.
    ///
    /// # Errors
    /// Rejects invalid colors if the color contract is extended.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.foreground.validate()?;
        self.background.validate()
    }
}
impl Copy for TerminalCellStyle {}
impl Default for TerminalCellStyle {
    fn default() -> Self {
        Self {
            foreground: TerminalColor::Default,
            background: TerminalColor::Default,
            bold: false,
            faint: false,
            italic: false,
            underline: false,
            inverse: false,
            strikethrough: false,
        }
    }
}
contract_struct! {
    /// One UTF-8 cell with a maximum of 64 bytes of text.
    TerminalCell { pub kind: TerminalCellKind, pub text: String, pub style: TerminalCellStyle, pub hyperlink_id: Option<u32> }
}
impl TerminalCell {
    /// Validates text/kind agreement.
    ///
    /// # Errors
    /// Rejects oversized text, nonempty blank/continuation cells or empty visible cells.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.style.validate()?;
        require(self.hyperlink_id != Some(0))?;
        require(
            self.text.len() <= 64
                && match self.kind {
                    TerminalCellKind::Blank | TerminalCellKind::Continuation => {
                        self.text.is_empty()
                    }
                    _ => !self.text.is_empty(),
                },
        )
    }
}
contract_struct! {
    /// Data-only hyperlink table entry. No consumer may infer open/fetch authority.
    TerminalHyperlink { pub id: u32, #[serde(deserialize_with = "bounded_vec::<_, u8, 4096>")] pub uri: Vec<u8> }
}
impl TerminalHyperlink {
    /// Validates the ID and bounded opaque URI.
    ///
    /// # Errors
    /// Rejects ID zero and empty or oversized URIs. Raw bytes are preserved.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        require(self.id != 0)?;
        require(!self.uri.is_empty() && self.uri.len() <= 4096)
    }
}
contract_struct! {
    /// Structured row-major immutable screen projection, not an ANSI or debug-text dump.
    TerminalScreen {
        pub dimensions: TerminalDimensions, pub cursor: TerminalScreenCursor, pub modes: TerminalModes,
        #[serde(deserialize_with = "bounded_vec::<_, TerminalCell, MAX_TERMINAL_SCREEN_CELLS>")]
        pub cells: Vec<TerminalCell>,
        #[serde(deserialize_with = "hyperlink_table")]
        pub hyperlinks: Vec<TerminalHyperlink>
    }
}
impl TerminalScreen {
    /// Validates the complete screen without native effects.
    ///
    /// # Errors
    /// Rejects count/cursor mismatch, broken wide cells, or aggregate text overflow.
    pub fn validate(&self) -> Result<(), TerminalContractError> {
        self.dimensions.validate()?;
        self.cursor.validate()?;
        self.modes.validate()?;
        require(self.hyperlinks.len() <= MAX_TERMINAL_HYPERLINKS)?;
        let mut previous_id = 0;
        let mut hyperlink_bytes = 0_usize;
        for link in &self.hyperlinks {
            link.validate()?;
            require(link.id > previous_id)?;
            previous_id = link.id;
            hyperlink_bytes = hyperlink_bytes
                .checked_add(link.uri.len())
                .ok_or(TerminalContractError)?;
            require(hyperlink_bytes <= MAX_TERMINAL_HYPERLINK_BYTES)?;
        }
        let columns = usize::from(self.dimensions.columns());
        require(
            self.cells.len() == usize::from(self.dimensions.rows()) * columns
                && self.cursor.row < self.dimensions.rows()
                && self.cursor.column < self.dimensions.columns(),
        )?;
        let mut bytes = 0_usize;
        for (index, cell) in self.cells.iter().enumerate() {
            cell.validate()?;
            if let Some(id) = cell.hyperlink_id {
                require(
                    self.hyperlinks
                        .binary_search_by_key(&id, |link| link.id)
                        .is_ok(),
                )?;
            }
            bytes = bytes
                .checked_add(cell.text.len())
                .ok_or(TerminalContractError)?;
            require(bytes <= MAX_TERMINAL_SCREEN_TEXT_BYTES)?;
            match cell.kind {
                TerminalCellKind::Wide => require(
                    index % columns + 1 < columns
                        && self
                            .cells
                            .get(index + 1)
                            .is_some_and(|next| next.kind == TerminalCellKind::Continuation),
                )?,
                TerminalCellKind::Continuation => require(
                    index % columns > 0 && self.cells[index - 1].kind == TerminalCellKind::Wide,
                )?,
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + Eq + fmt::Debug>(value: &T) {
        let wire = serde_json::to_value(value).unwrap();
        assert_eq!(&serde_json::from_value::<T>(wire).unwrap(), value);
    }

    #[test]
    fn identifiers_and_dimensions_validate_constructor_and_wire_boundaries() {
        for value in ["", ".", "..", "a/b", "a:b", "é", &"x".repeat(256)] {
            assert!(TerminalSessionId::new(value).is_err());
            assert!(serde_json::from_value::<TerminalSessionId>(json!(value)).is_err());
        }
        let id = TerminalSessionId::new("session-012_A.z").unwrap();
        assert_eq!(id.as_str(), "session-012_A.z");
        roundtrip(&id);
        assert!(TerminalSessionId::new("x".repeat(255)).is_ok());
        roundtrip(&TerminalMonitorId::new("opaque:监视器").unwrap());
        assert!(TerminalMonitorId::new("x".repeat(129)).is_err());
        assert!(TerminalMonitorId::new("a\0b").is_err());
        for (rows, columns) in [(0, 80), (24, 0), (4097, 1), (4096, 4096)] {
            assert!(TerminalDimensions::new(rows, columns).is_err());
            assert!(
                serde_json::from_value::<TerminalDimensions>(
                    json!({"rows":rows,"columns":columns})
                )
                .is_err()
            );
        }
        roundtrip(&TerminalDimensions::new(64, 4096).unwrap());
        assert!(
            serde_json::from_value::<TerminalDimensions>(
                json!({"rows":24,"columns":80,"extra":true})
            )
            .is_err()
        );
    }

    #[test]
    fn cursors_and_gaps_preserve_segments_and_order() {
        assert!(TerminalCursor::new(0, 0).is_err());
        let end = TerminalCursor::new(1, u64::MAX).unwrap();
        let next = TerminalCursor::new(2, 0).unwrap();
        assert!(end < next);
        assert_eq!(next.segment(), 2);
        assert_eq!(next.offset(), 0);
        roundtrip(&TerminalGap::new(end.clone(), next.clone()).unwrap());
        assert!(TerminalGap::new(next.clone(), end).is_err());
        assert!(TerminalGap::new(next.clone(), next).is_err());
        assert!(serde_json::from_value::<TerminalCursor>(json!({"segment":0,"offset":1})).is_err());
    }

    #[test]
    fn write_forms_are_closed_and_lease_operations_do_not_carry_payloads() {
        let payloads = [
            TerminalWritePayload::Text {
                text: "hello\0世界".into(),
            },
            TerminalWritePayload::Paste {
                text: "x\ny".into(),
            },
            TerminalWritePayload::Keys {
                keys: vec![TerminalNamedKey::ArrowUp, TerminalNamedKey::Enter],
            },
            TerminalWritePayload::Controls {
                controls: vec![b'c', b'D', b'?', b'@', b'_'],
            },
        ];
        for payload in payloads {
            payload.validate().unwrap();
            roundtrip(&payload);
            roundtrip(&TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Use,
                payload: Some(payload.clone()),
            });
            assert!(
                TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Acquire,
                    payload: Some(payload)
                }
                .validate()
                .is_err()
            );
        }
        for lease in [
            TerminalWriteLeaseIntent::Acquire,
            TerminalWriteLeaseIntent::Release,
            TerminalWriteLeaseIntent::Revoke,
        ] {
            roundtrip(&TerminalWriteRequest {
                lease,
                payload: None,
            });
        }
        assert!(
            TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Use,
                payload: None
            }
            .validate()
            .is_err()
        );
        for wire in [
            json!({"kind":"text","text":""}),
            json!({"kind":"text","text":"x","extra":true}),
            json!({"kind":"controls","controls":[0]}),
            json!({"kind":"keys","keys":[]}),
        ] {
            assert!(serde_json::from_value::<TerminalWritePayload>(wire).is_err());
        }
        assert!(
            TerminalWritePayload::Text {
                text: "x".repeat(MAX_TERMINAL_WRITE_BYTES)
            }
            .validate()
            .is_ok()
        );
        assert!(
            TerminalWritePayload::Text {
                text: "x".repeat(MAX_TERMINAL_WRITE_BYTES + 1)
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn wait_conditions_and_ceilings_are_validated() {
        for condition in [
            TerminalReturnCondition::Started,
            TerminalReturnCondition::Exit,
            TerminalReturnCondition::Quiet { duration_ms: 1 },
            TerminalReturnCondition::Match {
                pattern: "ready".into(),
            },
        ] {
            roundtrip(&TerminalWaitRequest {
                condition: condition.clone(),
                safety_ceiling_ms: 1,
            });
            assert!(
                TerminalWaitRequest {
                    condition,
                    safety_ceiling_ms: 0
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<TerminalReturnCondition>(
                json!({"kind":"started","unexpected":"PRIVATE"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<TerminalReturnCondition>(
                json!({"kind":"quiet","duration_ms":0})
            )
            .is_err()
        );
    }

    fn definition(condition: TerminalMonitorCondition) -> TerminalMonitorDefinition {
        let check_schedule = condition
            .requires_polling()
            .then_some(TerminalSchedule { interval_ms: 10 });
        TerminalMonitorDefinition {
            condition,
            check_schedule,
            notify: TerminalNotifySchedule::OnMatch,
            lifetime: TerminalMonitorLifetime::UntilMatch,
        }
    }

    #[test]
    fn all_thirteen_monitor_conditions_roundtrip_with_exact_schedule_requirements() {
        let conditions = [
            TerminalMonitorCondition::ProcessExit,
            TerminalMonitorCondition::ExitCode { exit_code: 255 },
            TerminalMonitorCondition::Signal {
                signal: TerminalSignal::Terminate,
            },
            TerminalMonitorCondition::OutputContains {
                pattern: "ready".into(),
            },
            TerminalMonitorCondition::OutputMatches {
                pattern: "*ready?".into(),
            },
            TerminalMonitorCondition::OutputQuiet { duration_ms: 10 },
            TerminalMonitorCondition::ScreenMatches {
                pattern: "prompt*".into(),
            },
            TerminalMonitorCondition::TcpReady {
                host: "localhost".into(),
                port: 80,
            },
            TerminalMonitorCondition::HttpReady {
                url: "https://localhost/health".into(),
            },
            TerminalMonitorCondition::PathExists {
                path: "ready".into(),
            },
            TerminalMonitorCondition::PathChanged {
                path: "state".into(),
            },
            TerminalMonitorCondition::PathSize {
                path: "data".into(),
                minimum_bytes: 42,
            },
            TerminalMonitorCondition::CustomProbe {
                command: "true".into(),
                cwd: "/workspace".into(),
            },
        ];
        for condition in conditions {
            let mut definition = definition(condition);
            roundtrip(&definition);
            definition.check_schedule = if definition.check_schedule.is_some() {
                None
            } else {
                Some(TerminalSchedule { interval_ms: 10 })
            };
            assert!(definition.validate().is_err());
            assert!(
                serde_json::from_value::<TerminalMonitorDefinition>(
                    serde_json::to_value(&definition).unwrap()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn monitor_operations_notifications_lifetimes_and_events_roundtrip() {
        let id = TerminalMonitorId::new("monitor-1").unwrap();
        for operation in [
            TerminalMonitorOperation::Add {
                definition: definition(TerminalMonitorCondition::ProcessExit),
            },
            TerminalMonitorOperation::Update {
                monitor_id: id.clone(),
                definition: definition(TerminalMonitorCondition::ProcessExit),
            },
            TerminalMonitorOperation::Pause {
                monitor_id: id.clone(),
            },
            TerminalMonitorOperation::Resume {
                monitor_id: id.clone(),
            },
            TerminalMonitorOperation::Remove {
                monitor_id: id.clone(),
            },
        ] {
            roundtrip(&operation);
        }
        for notify in [
            TerminalNotifySchedule::OnMatch,
            TerminalNotifySchedule::OnStateChange,
            TerminalNotifySchedule::OnExit,
            TerminalNotifySchedule::EveryCheck,
            TerminalNotifySchedule::EveryNChecks { count: 1 },
            TerminalNotifySchedule::Interval {
                interval_ms: MAX_DAY_MS,
            },
        ] {
            roundtrip(&notify);
        }
        for lifetime in [
            TerminalMonitorLifetime::UntilMatch,
            TerminalMonitorLifetime::UntilSessionEnd,
            TerminalMonitorLifetime::Duration {
                duration_ms: MAX_YEAR_MS,
            },
        ] {
            roundtrip(&lifetime);
        }
        assert!(TerminalSchedule { interval_ms: 9 }.validate().is_err());
        assert!(
            TerminalSchedule {
                interval_ms: MAX_DAY_MS + 1
            }
            .validate()
            .is_err()
        );
        assert!(
            TerminalNotifySchedule::EveryNChecks { count: 0 }
                .validate()
                .is_err()
        );
        assert!(
            TerminalMonitorLifetime::Duration {
                duration_ms: MAX_YEAR_MS + 1
            }
            .validate()
            .is_err()
        );
        roundtrip(&TerminalMonitorEvent {
            event_id: 1,
            monitor_id: id,
            reason: TerminalMonitorEventReason::Matched,
            lifecycle: TerminalLifecycle::Running,
            cursor: TerminalCursor::new(1, 0).unwrap(),
            created_at_ms: 123,
        });
        roundtrip(&TerminalEventQuery {
            after_event_id: 10,
            acknowledge_event_id: Some(9),
            max_events: 256,
        });
        assert!(
            TerminalEventQuery {
                after_event_id: 0,
                acknowledge_event_id: Some(0),
                max_events: 1
            }
            .validate()
            .is_err()
        );
    }

    fn screen() -> TerminalScreen {
        TerminalScreen {
            dimensions: TerminalDimensions::new(1, 2).unwrap(),
            cursor: TerminalScreenCursor {
                row: 0,
                column: 0,
                visible: true,
                shape: TerminalCursorShape::Block,
                blinking: true,
            },
            modes: TerminalModes::default(),
            cells: vec![
                TerminalCell {
                    kind: TerminalCellKind::Wide,
                    text: "界".into(),
                    style: TerminalCellStyle::default(),
                    hyperlink_id: None,
                },
                TerminalCell {
                    kind: TerminalCellKind::Continuation,
                    text: String::new(),
                    style: TerminalCellStyle::default(),
                    hyperlink_id: None,
                },
            ],
            hyperlinks: Vec::new(),
        }
    }

    #[test]
    fn structured_screen_enforces_counts_cursor_and_wide_cell_pairs() {
        let valid = screen();
        roundtrip(&valid);
        let mut bad = valid.clone();
        bad.cells.pop();
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.cursor.column = 2;
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.cells.swap(0, 1);
        assert!(bad.validate().is_err());
        let mut bad = valid;
        bad.cells[1].text = "unexpected".into();
        assert!(bad.validate().is_err());
        assert!(
            serde_json::from_value::<TerminalScreen>(serde_json::to_value(bad).unwrap()).is_err()
        );
    }

    #[test]
    fn debug_and_validation_errors_do_not_reflect_payloads() {
        let payload = TerminalWritePayload::Text {
            text: "PRIVATE".into(),
        };
        assert_eq!(format!("{payload:?}"), "TerminalWritePayload { .. }");
        let monitor = definition(TerminalMonitorCondition::CustomProbe {
            command: "PRIVATE".into(),
            cwd: "/PRIVATE".into(),
        });
        assert_eq!(format!("{monitor:?}"), "TerminalMonitorDefinition { .. }");
        let invalid =
            serde_json::from_value::<TerminalDimensions>(json!({"rows":"PRIVATE","columns":1}))
                .unwrap_err();
        assert!(!invalid.to_string().contains("PRIVATE"));
    }

    #[test]
    fn hyperlink_table_is_unique_bounded_and_all_cell_references_resolve() {
        let mut value = screen();
        value.hyperlinks.push(TerminalHyperlink {
            id: 1,
            uri: b"https://example.test/PRIVATE".to_vec(),
        });
        value.cells[0].hyperlink_id = Some(1);
        roundtrip(&value);
        assert_eq!(
            format!("{:?}", value.hyperlinks[0]),
            "TerminalHyperlink { .. }"
        );
        value.cells[0].hyperlink_id = Some(2);
        assert!(value.validate().is_err());
        value.cells[0].hyperlink_id = Some(1);
        value.hyperlinks.push(value.hyperlinks[0].clone());
        assert!(value.validate().is_err());
        assert!(
            TerminalHyperlink {
                id: 0,
                uri: b"x".to_vec()
            }
            .validate()
            .is_err()
        );
        assert!(
            TerminalHyperlink {
                id: 1,
                uri: vec![b'x'; 4097]
            }
            .validate()
            .is_err()
        );
        let raw = TerminalHyperlink {
            id: 2,
            uri: vec![0, 255, 128],
        };
        roundtrip(&raw);
        assert!(
            serde_json::from_value::<TerminalHyperlink>(json!({"id":1,"uri":vec![0; 4097]}))
                .is_err()
        );
    }
}
