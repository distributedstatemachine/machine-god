//! Effect-free adapter from the pinned public terminal JSON shape to core requests.
//!
//! The host must reject duplicate object fields while decoding raw JSON, before
//! constructing `Value`: a `Value` cannot represent or detect lost duplicates.
//! String-encoded composite arguments are decoded here with duplicate rejection.
//! Cwd preparation and all authority remain the host's responsibility.

use core::fmt;
use machine_god_core::{
    MAX_TERMINAL_INITIAL_MONITORS, TerminalActionRequest, TerminalBackend, TerminalContractError,
    TerminalCursor, TerminalDimensions, TerminalEventQuery, TerminalExecRequest,
    TerminalListFilters, TerminalMonitorCondition, TerminalMonitorDefinition, TerminalMonitorId,
    TerminalMonitorLifetime, TerminalMonitorOperation, TerminalNamedKey, TerminalNotifySchedule,
    TerminalProfile, TerminalReturnCondition, TerminalSchedule, TerminalShellSpec, TerminalSignal,
    TerminalStartRequest, TerminalWaitRequest, TerminalWriteLeaseIntent, TerminalWritePayload,
    TerminalWriteRequest,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

const MAX_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_BYTES: usize = 32 * 1024;
const MAX_CWD_BYTES: usize = 4096;

/// Closed, data-free failure: never echoes model-supplied paths or payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalActionParseError;

impl fmt::Display for TerminalActionParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid terminal action arguments")
    }
}
impl std::error::Error for TerminalActionParseError {}
impl From<TerminalContractError> for TerminalActionParseError {
    fn from(_: TerminalContractError) -> Self {
        Self
    }
}

type ParseResult<T> = Result<T, TerminalActionParseError>;

const PUBLIC_FIELDS: &[&str] = &[
    "action",
    "session_id",
    "cwd",
    "command",
    "profile",
    "shell",
    "backend",
    "return_when",
    "wait_ceiling_ms",
    "dimensions",
    "initial_monitors",
    "cursor_segment",
    "cursor_offset",
    "after_event_id",
    "acknowledge_event_id",
    "max_events",
    "write",
    "lease",
    "monitor",
    "task_id",
    "workspace_root",
    "rows",
    "columns",
    "signal",
    "close_policy",
];

fn fields(action: &str) -> ParseResult<(&'static [&'static str], &'static [&'static str])> {
    Ok(match action {
        "exec" => (
            &["action", "command", "cwd", "profile"],
            &["action", "command"],
        ),
        "start" => (
            &[
                "action",
                "cwd",
                "command",
                "profile",
                "shell",
                "backend",
                "return_when",
                "wait_ceiling_ms",
                "dimensions",
                "initial_monitors",
            ],
            &["action"],
        ),
        "read" => (
            &["action", "session_id", "cursor_segment", "cursor_offset"],
            &["action", "session_id", "cursor_segment"],
        ),
        "screen" => (&["action", "session_id"], &["action", "session_id"]),
        "write" => (
            &["action", "session_id", "write", "lease"],
            &["action", "session_id"],
        ),
        "wait" => (
            &["action", "session_id", "return_when", "wait_ceiling_ms"],
            &["action", "session_id", "return_when", "wait_ceiling_ms"],
        ),
        "monitor" => (
            &["action", "session_id", "monitor"],
            &["action", "session_id", "monitor"],
        ),
        "inspect" => (
            &[
                "action",
                "session_id",
                "after_event_id",
                "acknowledge_event_id",
                "max_events",
            ],
            &["action", "session_id"],
        ),
        "list" => (
            &["action", "task_id", "workspace_root", "backend"],
            &["action"],
        ),
        "resize" => (
            &["action", "session_id", "rows", "columns"],
            &["action", "session_id", "rows", "columns"],
        ),
        "signal" => (
            &["action", "session_id", "signal"],
            &["action", "session_id", "signal"],
        ),
        "close" => (
            &["action", "session_id", "close_policy"],
            &["action", "session_id", "close_policy"],
        ),
        _ => return Err(TerminalActionParseError),
    })
}

fn present<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    object.get(key).filter(|value| !value.is_null())
}

fn checked_object(arguments: &Value) -> ParseResult<&Map<String, Value>> {
    let mut remaining = MAX_ARGUMENT_BYTES;
    check_depth(arguments, 0, &mut remaining)?;
    serde_json::to_writer(ByteBudget(MAX_ARGUMENT_BYTES), arguments)
        .map_err(|_| TerminalActionParseError)?;
    let object = arguments.as_object().ok_or(TerminalActionParseError)?;
    let action = object
        .get("action")
        .and_then(Value::as_str)
        .ok_or(TerminalActionParseError)?;
    let (allowed, required) = fields(action)?;
    if object.iter().any(|(key, value)| {
        !(allowed.contains(&key.as_str())
            || (key != "action" && value.is_null() && PUBLIC_FIELDS.contains(&key.as_str())))
    }) || required.iter().any(|key| present(object, key).is_none())
        || (action == "start"
            && present(object, "profile").is_some()
            && present(object, "shell").is_some())
    {
        return Err(TerminalActionParseError);
    }
    Ok(object)
}

fn check_depth(value: &Value, depth: usize, remaining: &mut usize) -> ParseResult<()> {
    *remaining = remaining.checked_sub(1).ok_or(TerminalActionParseError)?;
    if depth > 64 {
        return Err(TerminalActionParseError);
    }
    match value {
        Value::Array(values) => {
            if values.len() > MAX_ARGUMENT_BYTES {
                return Err(TerminalActionParseError);
            }
            for value in values {
                check_depth(value, depth + 1, remaining)?;
            }
        }
        Value::Object(values) => {
            if values.len() > MAX_ARGUMENT_BYTES {
                return Err(TerminalActionParseError);
            }
            for value in values.values() {
                check_depth(value, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct ByteBudget(usize);
impl std::io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("terminal argument bound"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Extracts the bounded raw cwd without resolving or authorizing it.
///
/// Hosts must call this before cwd preparation, and pass the prepared result to
/// `decode_terminal_action`. `None` means use the host's current default cwd.
/// Relative and absolute input are retained verbatim for the host's policy.
/// A UTF-8 byte-array cwd is returned as an owned string. The entire request
/// is validated with an inert placeholder cwd first, so malformed nested
/// arguments cannot trigger cwd-resolution effects. Then the host prepares
/// this raw cwd and calls `decode_terminal_action` with its canonical result.
///
/// # Errors
/// Rejects any invalid request or empty, oversized or NUL-containing cwd.
pub fn terminal_action_requested_cwd(
    arguments: &Value,
) -> ParseResult<Option<std::borrow::Cow<'_, str>>> {
    decode_terminal_action(arguments, "/")?;
    let object = checked_object(arguments)?;
    present(object, "cwd")
        .map(|value| {
            let cwd = if let Some(text) = value.as_str() {
                std::borrow::Cow::Borrowed(text)
            } else {
                std::borrow::Cow::Owned(decode::<String>(normalize_field("cwd", value.clone())?)?)
            };
            bounded_text(&cwd, MAX_CWD_BYTES)?;
            Ok(cwd)
        })
        .transpose()
}

fn bounded_text(value: &str, maximum: usize) -> ParseResult<()> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        Err(TerminalActionParseError)
    } else {
        Ok(())
    }
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> ParseResult<T> {
    serde_json::from_value(value).map_err(|_| TerminalActionParseError)
}

// A raw-value visitor retains duplicate detection even in nested composite JSON.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("terminal JSON value")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|value| UniqueValue(Value::Number(value)))
                    .ok_or_else(|| E::custom(TerminalActionParseError))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(TerminalActionParseError));
                    }
                    let UniqueValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

fn composite(value: &Value) -> ParseResult<Value> {
    let value = match value {
        Value::String(value) => {
            serde_json::from_str::<UniqueValue>(value)
                .map_err(|_| TerminalActionParseError)?
                .0
        }
        value => value.clone(),
    };
    if !value.is_object() && !value.is_array() {
        return Err(TerminalActionParseError);
    }
    Ok(value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReturnInput {
    kind: String,
    duration_ms: Option<u64>,
    pattern: Option<String>,
}
impl ReturnInput {
    fn build(self) -> ParseResult<TerminalReturnCondition> {
        Ok(match self.kind.as_str() {
            "started" => TerminalReturnCondition::Started,
            "exit" => TerminalReturnCondition::Exit,
            "quiet" => TerminalReturnCondition::Quiet {
                duration_ms: self.duration_ms.ok_or(TerminalActionParseError)?,
            },
            "match" => TerminalReturnCondition::Match {
                pattern: self.pattern.ok_or(TerminalActionParseError)?,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteInput {
    kind: String,
    text: Option<String>,
    #[serde(default)]
    keys: Vec<TerminalNamedKey>,
    #[serde(default, deserialize_with = "control_bytes")]
    controls: Vec<u8>,
}
fn control_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Controls {
        Text(String),
        Bytes(Vec<u8>),
    }
    Ok(match Controls::deserialize(deserializer)? {
        Controls::Text(text) => text.into_bytes(),
        Controls::Bytes(bytes) => bytes,
    })
}
impl WriteInput {
    fn build(self) -> ParseResult<TerminalWritePayload> {
        Ok(match self.kind.as_str() {
            "text" => TerminalWritePayload::Text {
                text: self.text.ok_or(TerminalActionParseError)?,
            },
            "paste" => TerminalWritePayload::Paste {
                text: self.text.ok_or(TerminalActionParseError)?,
            },
            "keys" => TerminalWritePayload::Keys { keys: self.keys },
            "controls" => TerminalWritePayload::Controls {
                controls: self.controls,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConditionInput {
    kind: String,
    pattern: Option<String>,
    duration_ms: Option<u64>,
    exit_code: Option<i32>,
    signal: Option<TerminalSignal>,
    host: Option<String>,
    port: Option<u16>,
    path: Option<String>,
    minimum_bytes: Option<u64>,
    command: Option<String>,
    cwd: Option<String>,
}
impl ConditionInput {
    fn build(self) -> ParseResult<TerminalMonitorCondition> {
        Ok(match self.kind.as_str() {
            "process_exit" => TerminalMonitorCondition::ProcessExit,
            "exit_code" => TerminalMonitorCondition::ExitCode {
                exit_code: self.exit_code.ok_or(TerminalActionParseError)?,
            },
            "signal" => TerminalMonitorCondition::Signal {
                signal: self.signal.ok_or(TerminalActionParseError)?,
            },
            "output_contains" => TerminalMonitorCondition::OutputContains {
                pattern: self.pattern.ok_or(TerminalActionParseError)?,
            },
            "output_matches" => TerminalMonitorCondition::OutputMatches {
                pattern: self.pattern.ok_or(TerminalActionParseError)?,
            },
            "output_quiet" => TerminalMonitorCondition::OutputQuiet {
                duration_ms: self.duration_ms.ok_or(TerminalActionParseError)?,
            },
            "screen_matches" => TerminalMonitorCondition::ScreenMatches {
                pattern: self.pattern.ok_or(TerminalActionParseError)?,
            },
            "tcp_ready" => TerminalMonitorCondition::TcpReady {
                host: self.host.ok_or(TerminalActionParseError)?,
                port: self.port.ok_or(TerminalActionParseError)?,
            },
            "http_ready" => TerminalMonitorCondition::HttpReady {
                url: self.pattern.ok_or(TerminalActionParseError)?,
            },
            "path_exists" => TerminalMonitorCondition::PathExists {
                path: self.path.ok_or(TerminalActionParseError)?,
            },
            "path_changed" => TerminalMonitorCondition::PathChanged {
                path: self.path.ok_or(TerminalActionParseError)?,
            },
            "path_size" => TerminalMonitorCondition::PathSize {
                path: self.path.ok_or(TerminalActionParseError)?,
                minimum_bytes: self.minimum_bytes.ok_or(TerminalActionParseError)?,
            },
            "custom_probe" => TerminalMonitorCondition::CustomProbe {
                command: self.command.ok_or(TerminalActionParseError)?,
                cwd: self.cwd.ok_or(TerminalActionParseError)?,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotifyInput {
    kind: String,
    count: Option<u32>,
    interval_ms: Option<u64>,
}
impl NotifyInput {
    fn build(self) -> ParseResult<TerminalNotifySchedule> {
        Ok(match self.kind.as_str() {
            "on_match" => TerminalNotifySchedule::OnMatch,
            "on_state_change" => TerminalNotifySchedule::OnStateChange,
            "on_exit" => TerminalNotifySchedule::OnExit,
            "every_check" => TerminalNotifySchedule::EveryCheck,
            "every_n_checks" => TerminalNotifySchedule::EveryNChecks {
                count: self.count.ok_or(TerminalActionParseError)?,
            },
            "interval" => TerminalNotifySchedule::Interval {
                interval_ms: self.interval_ms.ok_or(TerminalActionParseError)?,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifetimeInput {
    kind: String,
    duration_ms: Option<u64>,
}
impl LifetimeInput {
    fn build(self) -> ParseResult<TerminalMonitorLifetime> {
        Ok(match self.kind.as_str() {
            "until_match" => TerminalMonitorLifetime::UntilMatch,
            "until_session_end" => TerminalMonitorLifetime::UntilSessionEnd,
            "duration" => TerminalMonitorLifetime::Duration {
                duration_ms: self.duration_ms.ok_or(TerminalActionParseError)?,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionInput {
    condition: ConditionInput,
    check_interval_ms: Option<u64>,
    notify: NotifyInput,
    lifetime: LifetimeInput,
}
impl DefinitionInput {
    fn build(self) -> ParseResult<TerminalMonitorDefinition> {
        let condition = self.condition.build()?;
        let check_schedule = if condition.requires_polling() {
            Some(TerminalSchedule {
                interval_ms: self.check_interval_ms.ok_or(TerminalActionParseError)?,
            })
        } else {
            None
        };
        Ok(TerminalMonitorDefinition {
            condition,
            check_schedule,
            notify: self.notify.build()?,
            lifetime: self.lifetime.build()?,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationInput {
    kind: String,
    monitor_id: Option<String>,
    definition: Option<DefinitionInput>,
}
impl OperationInput {
    fn build(self) -> ParseResult<TerminalMonitorOperation> {
        Ok(match self.kind.as_str() {
            "add" => TerminalMonitorOperation::Add {
                definition: self.definition.ok_or(TerminalActionParseError)?.build()?,
            },
            "update" => TerminalMonitorOperation::Update {
                monitor_id: TerminalMonitorId::new(
                    self.monitor_id.ok_or(TerminalActionParseError)?,
                )?,
                definition: self.definition.ok_or(TerminalActionParseError)?.build()?,
            },
            "pause" => TerminalMonitorOperation::Pause {
                monitor_id: TerminalMonitorId::new(
                    self.monitor_id.ok_or(TerminalActionParseError)?,
                )?,
            },
            "resume" => TerminalMonitorOperation::Resume {
                monitor_id: TerminalMonitorId::new(
                    self.monitor_id.ok_or(TerminalActionParseError)?,
                )?,
            },
            "remove" => TerminalMonitorOperation::Remove {
                monitor_id: TerminalMonitorId::new(
                    self.monitor_id.ok_or(TerminalActionParseError)?,
                )?,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellInput {
    #[serde(default = "user_login")]
    kind: String,
    path: Option<String>,
    #[serde(default)]
    clean_start: bool,
}
fn user_login() -> String {
    "user_login".to_owned()
}
impl ShellInput {
    fn build(self) -> ParseResult<TerminalShellSpec> {
        Ok(match self.kind.as_str() {
            "user_login" => TerminalShellSpec::UserLogin {},
            "executable" => TerminalShellSpec::Executable {
                path: self.path.ok_or(TerminalActionParseError)?,
                clean_start: self.clean_start,
            },
            _ => return Err(TerminalActionParseError),
        })
    }
}

fn optional<T: for<'de> Deserialize<'de>>(
    object: &Map<String, Value>,
    key: &str,
) -> ParseResult<Option<T>> {
    present(object, key)
        .map(|value| decode(normalize_field(key, value.clone())?))
        .transpose()
}
fn required<T: for<'de> Deserialize<'de>>(
    object: &Map<String, Value>,
    key: &str,
) -> ParseResult<T> {
    optional(object, key)?.ok_or(TerminalActionParseError)
}
fn optional_composite<T: for<'de> Deserialize<'de>>(
    object: &Map<String, Value>,
    key: &str,
) -> ParseResult<Option<T>> {
    present(object, key)
        .map(|value| decode(normalize_field(key, composite(value)?)?))
        .transpose()
}

fn initial_monitors(object: &Map<String, Value>) -> ParseResult<Vec<TerminalMonitorDefinition>> {
    let Some(value) = present(object, "initial_monitors") else {
        return Ok(Vec::new());
    };
    let mut value = composite(value)?;
    let values = value.as_array_mut().ok_or(TerminalActionParseError)?;
    if values.len() > MAX_TERMINAL_INITIAL_MONITORS {
        return Err(TerminalActionParseError);
    }
    // Pinned compatibility promotion is restricted to initial monitor entries.
    // An explicitly present outer field (even null) takes precedence.
    for value in values {
        if let Some(monitor) = value.as_object_mut() {
            let interval = monitor
                .get_mut("condition")
                .and_then(Value::as_object_mut)
                .and_then(|condition| condition.remove("check_interval_ms"));
            if let Some(interval) = interval {
                monitor.entry("check_interval_ms").or_insert(interval);
            }
        }
    }
    decode::<Vec<DefinitionInput>>(normalize_field("initial_monitors", value)?)?
        .into_iter()
        .map(DefinitionInput::build)
        .collect()
}

const SIGNALS: &[&str] = &["hangup", "interrupt", "quit", "terminate", "kill"];
const KEYS: &[&str] = &[
    "enter",
    "tab",
    "escape",
    "backspace",
    "delete",
    "insert",
    "arrow_up",
    "arrow_down",
    "arrow_left",
    "arrow_right",
    "home",
    "end",
    "page_up",
    "page_down",
];

// Zig 0.16's static JSON parser accepts integer spellings and enum ordinals.
// Normalize only fields whose pinned public type calls for that conversion.
fn normalize_field(context: &str, mut value: Value) -> ParseResult<Value> {
    if value.is_null() {
        return Ok(value);
    }
    if matches!(
        context,
        "session_id"
            | "cwd"
            | "command"
            | "task_id"
            | "workspace_root"
            | "path"
            | "pattern"
            | "host"
            | "text"
            | "monitor_id"
    ) {
        if let Value::Array(values) = value {
            let bytes = values
                .into_iter()
                .map(|value| decode::<u8>(integer_value(value)?))
                .collect::<ParseResult<Vec<_>>>()?;
            return String::from_utf8(bytes)
                .map(Value::String)
                .map_err(|_| TerminalActionParseError);
        }
        return Ok(value);
    }
    let enumeration: Option<&[&str]> = match context {
        "profile" => Some(&["clean", "user"]),
        "backend" => Some(&["native", "tmux"]),
        "lease" => Some(&["acquire", "use", "release", "revoke"]),
        "signal" => Some(SIGNALS),
        "close_policy" => Some(&["graceful", "force"]),
        "key" => Some(KEYS),
        _ => None,
    };
    if let Some(names) = enumeration {
        return normalize_enum(value, names);
    }
    if matches!(
        context,
        "wait_ceiling_ms"
            | "cursor_segment"
            | "cursor_offset"
            | "after_event_id"
            | "acknowledge_event_id"
            | "max_events"
            | "rows"
            | "columns"
            | "duration_ms"
            | "exit_code"
            | "port"
            | "minimum_bytes"
            | "count"
            | "interval_ms"
            | "check_interval_ms"
            | "control"
    ) {
        return integer_value(value);
    }
    if let Value::Array(values) = &mut value {
        let child = match context {
            "initial_monitors" => "definition",
            "keys" => "key",
            "controls" => "control",
            _ => return Ok(value),
        };
        for value in values {
            *value = normalize_field(child, value.take())?;
        }
    }
    if let Value::Object(object) = &mut value {
        let kinds = composite_kind_names(context);
        for (key, value) in object {
            *value = if key == "kind" {
                if let Some(names) = kinds {
                    normalize_enum(value.take(), names)?
                } else {
                    value.take()
                }
            } else {
                normalize_field(key, value.take())?
            };
        }
    }
    Ok(value)
}

fn composite_kind_names(context: &str) -> Option<&'static [&'static str]> {
    match context {
        "shell" => Some(&["user_login", "executable"]),
        "return_when" => Some(&["started", "exit", "quiet", "match"]),
        "write" => Some(&["text", "keys", "controls", "paste"]),
        "condition" => Some(&[
            "process_exit",
            "exit_code",
            "signal",
            "output_contains",
            "output_matches",
            "output_quiet",
            "screen_matches",
            "tcp_ready",
            "http_ready",
            "path_exists",
            "path_changed",
            "path_size",
            "custom_probe",
        ]),
        "notify" => Some(&[
            "on_match",
            "on_state_change",
            "on_exit",
            "every_check",
            "every_n_checks",
            "interval",
        ]),
        "lifetime" => Some(&["until_match", "until_session_end", "duration"]),
        "monitor" => Some(&["add", "update", "pause", "resume", "remove"]),
        _ => None,
    }
}

fn normalize_enum(value: Value, names: &[&str]) -> ParseResult<Value> {
    if value.as_str().is_some_and(|value| names.contains(&value)) {
        return Ok(value);
    }
    let spelling = match &value {
        Value::String(value) => value.clone(),
        Value::Number(value) if value.is_u64() || value.is_i64() => value.to_string(),
        _ => return Err(TerminalActionParseError),
    };
    if spelling == "-0" || spelling.contains(['.', 'e', 'E']) {
        return Err(TerminalActionParseError);
    }
    let ordinal = integer_spelling(&spelling)?;
    let index = usize::try_from(ordinal).map_err(|_| TerminalActionParseError)?;
    Ok(Value::String(
        (*names.get(index).ok_or(TerminalActionParseError)?).to_owned(),
    ))
}

fn integer_value(value: Value) -> ParseResult<Value> {
    let integer = match value {
        Value::String(value) => integer_spelling(&value)?,
        Value::Number(value) if value.is_i64() => {
            i128::from(value.as_i64().ok_or(TerminalActionParseError)?)
        }
        Value::Number(value) if value.is_u64() => {
            i128::from(value.as_u64().ok_or(TerminalActionParseError)?)
        }
        Value::Number(value) => {
            let number = value.as_f64().ok_or(TerminalActionParseError)?;
            // The pinned decoder also passes finite floating JSON through
            // dynamic Value (f64), then stringify, before typed integer parsing.
            if !number.is_finite() {
                return Err(TerminalActionParseError);
            }
            integer_spelling(&value.to_string())?
        }
        _ => return Err(TerminalActionParseError),
    };
    if integer < 0 {
        Ok(Value::from(
            i64::try_from(integer).map_err(|_| TerminalActionParseError)?,
        ))
    } else {
        Ok(Value::from(
            u64::try_from(integer).map_err(|_| TerminalActionParseError)?,
        ))
    }
}

fn integer_spelling(spelling: &str) -> ParseResult<i128> {
    // Exact bounded decimal arithmetic avoids float rounding or lossy casts.
    if spelling.is_empty() || spelling.len() > MAX_ARGUMENT_BYTES {
        return Err(TerminalActionParseError);
    }
    let (negative, unsigned) = match spelling.as_bytes()[0] {
        b'-' => (true, &spelling[1..]),
        b'+' => (false, &spelling[1..]),
        _ => (false, spelling),
    };
    if !unsigned.contains(['.', 'e', 'E']) {
        if unsigned.is_empty() || unsigned.starts_with('_') || unsigned.ends_with('_') {
            return Err(TerminalActionParseError);
        }
        let mut integer = 0_i128;
        for byte in unsigned.bytes().filter(|byte| *byte != b'_') {
            if !byte.is_ascii_digit() {
                return Err(TerminalActionParseError);
            }
            integer = integer
                .checked_mul(10)
                .and_then(|value| value.checked_add(i128::from(byte - b'0')))
                .ok_or(TerminalActionParseError)?;
        }
        return Ok(if negative { -integer } else { integer });
    }
    let mut parts = unsigned.split(['e', 'E']);
    let mantissa = parts.next().ok_or(TerminalActionParseError)?;
    let exponent = parts
        .next()
        .map(str::parse::<i32>)
        .transpose()
        .map_err(|_| TerminalActionParseError)?
        .unwrap_or(0);
    if parts.next().is_some() || !(-65_536..=65_536).contains(&exponent) {
        return Err(TerminalActionParseError);
    }
    let mut fraction = None;
    let mut digits = String::new();
    for byte in mantissa.bytes() {
        match byte {
            b'0'..=b'9' => {
                digits.push(char::from(byte));
                if let Some(count) = &mut fraction {
                    *count += 1;
                }
            }
            b'.' if fraction.is_none() => fraction = Some(0_i32),
            _ => return Err(TerminalActionParseError),
        }
    }
    if digits.is_empty() {
        return Err(TerminalActionParseError);
    }
    let scale = exponent - fraction.unwrap_or(0);
    if scale < 0 {
        for _ in 0..scale.unsigned_abs() {
            match digits.pop() {
                Some('0') | None => {}
                _ => return Err(TerminalActionParseError),
            }
        }
    }
    let mut integer = 0_i128;
    for byte in digits.bytes() {
        integer = integer
            .checked_mul(10)
            .and_then(|value| value.checked_add(i128::from(byte - b'0')))
            .ok_or(TerminalActionParseError)?;
    }
    if scale > 0 {
        for _ in 0..scale {
            integer = integer.checked_mul(10).ok_or(TerminalActionParseError)?;
        }
    }
    Ok(if negative { -integer } else { integer })
}

/// Decodes all twelve public actions into validated, non-authoritative requests.
///
/// `resolved_cwd` must be the trusted canonical cwd prepared by the host from
/// `terminal_action_requested_cwd`; it is used only by `exec` and `start`.
/// The decoder validates the raw cwd too, but cannot establish that the host
/// prepared the right directory. It performs no filesystem, process, environment,
/// network, shell-resolution, or authorization effects.
///
/// Foreground `exec` retains the existing clean profile default and 32 KiB
/// command bound. Execution timeout remains a host limit, not a public argument.
/// Other actions use pinned semantic defaults and the bounded core contracts.
///
/// # Errors
/// Rejects malformed/oversized arguments, action-field violations, invalid
/// composites, or requests that fail core structural validation. Raw JSON
/// duplicate-field rejection must precede this `Value` API.
pub fn decode_terminal_action(
    arguments: &Value,
    resolved_cwd: &str,
) -> ParseResult<TerminalActionRequest> {
    let object = checked_object(arguments)?;
    if let Some(cwd) = present(object, "cwd") {
        let cwd = decode::<String>(normalize_field("cwd", cwd.clone())?)?;
        bounded_text(&cwd, MAX_CWD_BYTES)?;
    }
    let action = required::<String>(object, "action")?;
    if matches!(action.as_str(), "exec" | "start") {
        bounded_text(resolved_cwd, MAX_CWD_BYTES)?;
    }
    let request = match action.as_str() {
        "exec" => {
            let command = required::<String>(object, "command")?;
            bounded_text(&command, MAX_COMMAND_BYTES)?;
            let profile =
                optional::<TerminalProfile>(object, "profile")?.unwrap_or(TerminalProfile::Clean);
            if profile != TerminalProfile::Clean {
                return Err(TerminalActionParseError);
            }
            TerminalActionRequest::Exec {
                request: TerminalExecRequest {
                    command,
                    cwd: resolved_cwd.to_owned(),
                    profile: Some(profile),
                },
            }
        }
        "start" => TerminalActionRequest::Start {
            request: start_request(object, resolved_cwd)?,
        },
        "read" => TerminalActionRequest::Read {
            session_id: required(object, "session_id")?,
            cursor: TerminalCursor::new(
                required(object, "cursor_segment")?,
                optional(object, "cursor_offset")?.unwrap_or(0),
            )?,
        },
        "screen" => TerminalActionRequest::Screen {
            session_id: required(object, "session_id")?,
        },
        "write" => TerminalActionRequest::Write {
            session_id: required(object, "session_id")?,
            request: TerminalWriteRequest {
                lease: optional(object, "lease")?.unwrap_or(TerminalWriteLeaseIntent::Use),
                payload: optional_composite::<WriteInput>(object, "write")?
                    .map(WriteInput::build)
                    .transpose()?,
            },
        },
        "wait" => TerminalActionRequest::Wait {
            session_id: required(object, "session_id")?,
            request: TerminalWaitRequest {
                condition: optional_composite::<ReturnInput>(object, "return_when")?
                    .ok_or(TerminalActionParseError)?
                    .build()?,
                safety_ceiling_ms: required(object, "wait_ceiling_ms")?,
            },
        },
        "monitor" => TerminalActionRequest::Monitor {
            session_id: required(object, "session_id")?,
            operation: optional_composite::<OperationInput>(object, "monitor")?
                .ok_or(TerminalActionParseError)?
                .build()?,
        },
        "inspect" => TerminalActionRequest::Inspect {
            session_id: required(object, "session_id")?,
            events: TerminalEventQuery {
                after_event_id: optional(object, "after_event_id")?.unwrap_or(0),
                acknowledge_event_id: optional(object, "acknowledge_event_id")?,
                max_events: optional(object, "max_events")?.unwrap_or(64),
            },
        },
        "list" => TerminalActionRequest::List {
            filters: TerminalListFilters {
                task_id: optional::<String>(object, "task_id")?.filter(|value| !value.is_empty()),
                workspace_root: optional::<String>(object, "workspace_root")?
                    .filter(|value| !value.is_empty()),
                lifecycle: None,
                backend: optional(object, "backend")?,
            },
        },
        "resize" => TerminalActionRequest::Resize {
            session_id: required(object, "session_id")?,
            dimensions: TerminalDimensions::new(
                required(object, "rows")?,
                required(object, "columns")?,
            )?,
        },
        "signal" => TerminalActionRequest::Signal {
            session_id: required(object, "session_id")?,
            signal: required(object, "signal")?,
        },
        "close" => TerminalActionRequest::Close {
            session_id: required(object, "session_id")?,
            policy: required(object, "close_policy")?,
        },
        _ => return Err(TerminalActionParseError),
    };
    request.validate()?;
    Ok(request)
}

fn start_request(
    object: &Map<String, Value>,
    resolved_cwd: &str,
) -> ParseResult<TerminalStartRequest> {
    let command = optional::<String>(object, "command")?.filter(|value| !value.is_empty());
    let return_when = optional_composite::<ReturnInput>(object, "return_when")?
        .map(ReturnInput::build)
        .transpose()?
        .or_else(|| command.as_ref().map(|_| TerminalReturnCondition::Started));
    Ok(TerminalStartRequest {
        cwd: resolved_cwd.to_owned(),
        command,
        profile: optional(object, "profile")?,
        shell: optional_composite::<ShellInput>(object, "shell")?
            .map(ShellInput::build)
            .transpose()?,
        backend: optional(object, "backend")?.unwrap_or(TerminalBackend::Native),
        return_when,
        wait_ceiling_ms: optional(object, "wait_ceiling_ms")?,
        dimensions: optional_composite(object, "dimensions")?,
        initial_monitors: initial_monitors(object)?,
    })
}
