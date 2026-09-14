//! Pure bounded projection of actual native session and catalog observations.
use crate::acp::session::{AcpSessionError, NativeAcpSession};
use crate::{NativeSessionCatalogPage, PermissionMode};
use serde_json::{Map, Value, json};
use std::{
    io,
    path::{Component, Path},
};

const MAX_CATALOG_ROWS: usize = 100;
const MAX_MODEL_OPTIONS: usize = crate::AI_GATEWAY_MODEL_CATALOG_MAX_MODELS;
// Includes conservative object framing, modes, metadata and a native cursor.
// Leave room for the bounded RPC ID and enclosing response envelope.
const MAX_PROJECTION_BYTES: usize =
    crate::acp::protocol::ACP_MAX_FRAME_BYTES - (6 * crate::acp::protocol::ACP_MAX_ID_BYTES + 256);
const MODES: [(PermissionMode, &str); 3] = [
    (PermissionMode::Ask, "Ask"),
    (PermissionMode::Auto, "Auto"),
    (PermissionMode::Yolo, "Yolo"),
];

pub(super) fn config_response(session: &NativeAcpSession) -> Result<Value, AcpSessionError> {
    config_for_mode(session, session.mode()?)
}

pub(super) fn selection_response(session: &NativeAcpSession) -> Result<Value, AcpSessionError> {
    // One mode observation serves both representations in this response.
    let mode = session.mode()?;
    let mut result = config_for_mode(session, mode)?;
    result["sessionId"] = Value::String(session.id().as_str().to_owned());
    let mut modes = json!({"currentModeId":mode.as_str()});
    modes["availableModes"] = Value::Array(mode_options("id"));
    result["modes"] = modes;
    Ok(result)
}

fn config_for_mode(
    session: &NativeAcpSession,
    mode: PermissionMode,
) -> Result<Value, AcpSessionError> {
    let preferences = session.runtime().model_preferences();
    let catalog = session.runtime().model_catalog();
    config_value(
        mode,
        preferences.model(),
        catalog
            .iter()
            .flat_map(|catalog| catalog.entries())
            .map(|entry| entry.model().id()),
    )
}

fn config_value<'a>(
    mode: PermissionMode,
    current: &str,
    models: impl Iterator<Item = &'a str> + Clone,
) -> Result<Value, AcpSessionError> {
    let mut budget = Budget::new();
    budget.string(current)?;
    let mut count = 0;
    let mut contains_current = false;
    for model in models.clone() {
        count += 1;
        if count > MAX_MODEL_OPTIONS {
            return Err(AcpSessionError::Limit);
        }
        budget.charge(128)?;
        budget.string(model)?;
        budget.string(model)?;
        contains_current |= model == current;
    }
    if !contains_current {
        budget.charge(128)?;
        budget.string(current)?;
        budget.string(current)?;
    }
    // No model strings/JSON entries are cloned before the entire preflight.
    let mut options = Vec::with_capacity(count + usize::from(!contains_current));
    options.extend(models.map(|model| json!({"value":model,"name":model})));
    if !contains_current {
        options.push(json!({"value":current,"name":current}));
    }
    let mut permission_option = json!({"id":"mode","name":"Permission mode","category":"mode","type":"select","currentValue":mode.as_str()});
    permission_option["options"] = Value::Array(mode_options("value"));
    let mut model_option = json!({"id":"model","name":"Model","category":"model","type":"select","currentValue":current});
    model_option["options"] = Value::Array(options);
    Ok(Value::Object(Map::from_iter([(
        "configOptions".to_owned(),
        Value::Array(vec![permission_option, model_option]),
    )])))
}

fn mode_options(field: &str) -> Vec<Value> {
    MODES
        .iter()
        .map(|(mode, name)| {
            Value::Object(Map::from_iter([
                (field.to_owned(), Value::String(mode.as_str().to_owned())),
                ("name".to_owned(), Value::String((*name).to_owned())),
            ]))
        })
        .collect()
}

pub(super) fn catalog_response(page: &NativeSessionCatalogPage) -> Result<Value, AcpSessionError> {
    check_catalog_count(page.entries().len())?;
    let mut budget = Budget::new();
    let mut omitted_workspace = 0;
    for entry in page.entries() {
        let Some(cwd) = eligible_workspace(entry.native_metadata().workspace()) else {
            omitted_workspace += 1;
            continue;
        };
        budget.charge(128)?;
        budget.string(entry.id().as_str())?;
        budget.string(cwd)?;
        if let Some(title) = entry.native_metadata().title() {
            budget.string(title)?;
        }
        budget.charge(22)?; // optional quoted UTC timestamp
    }
    let mut omitted_updated_at = 0;
    let mut sessions = Vec::with_capacity(page.entries().len() - omitted_workspace);
    for entry in page.entries() {
        let metadata = entry.native_metadata();
        let Some(cwd) = eligible_workspace(metadata.workspace()) else {
            continue;
        };
        let mut value = json!({"sessionId":entry.id().as_str(),"cwd":cwd});
        if let Some(title) = metadata.title() {
            value["title"] = Value::String(title.to_owned());
        }
        if let Some(time) = metadata.updated_at_ms() {
            if let Some(formatted) = format_utc(time) {
                value["updatedAt"] = Value::String(formatted);
            } else {
                omitted_updated_at += 1;
            }
        }
        sessions.push(value);
    }
    let mut result = json!({"_meta":{"machineGod":{
        "scanComplete":page.scan_complete(),"resultsTruncated":page.results_truncated(),
        "skippedInvalid":page.skipped_invalid(),"omittedWorkspace":omitted_workspace,
        "omittedUpdatedAt":omitted_updated_at
    }}});
    result["sessions"] = Value::Array(sessions);
    // The cursor follows the original native scan, including unrenderable rows.
    // Filtering never silently skips a renderable row or invents a new boundary.
    if let Some(cursor) = page.next_cursor() {
        result["nextCursor"] = Value::String(cursor.to_string());
    }
    Ok(result)
}

fn check_catalog_count(count: usize) -> Result<(), AcpSessionError> {
    if count > MAX_CATALOG_ROWS {
        Err(AcpSessionError::Limit)
    } else {
        Ok(())
    }
}

fn eligible_workspace(path: Option<&Path>) -> Option<&str> {
    let path = path?;
    let text = path.to_str()?;
    if !path.is_absolute()
        || text.len() > crate::MAX_NATIVE_SESSION_WORKSPACE_BYTES
        || text.chars().any(char::is_control)
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return None;
    }
    Some(text)
}

struct Budget(usize);
impl Budget {
    const fn new() -> Self {
        Self(MAX_PROJECTION_BYTES - 2048)
    }
    fn charge(&mut self, amount: usize) -> Result<(), AcpSessionError> {
        self.0 = self.0.checked_sub(amount).ok_or(AcpSessionError::Limit)?;
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), AcpSessionError> {
        // Count serialized escaping directly from the borrowed string.
        serde_json::to_writer(self, value).map_err(|_| AcpSessionError::Limit)
    }
}
impl io::Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.charge(bytes.len())
            .map_err(|_| io::Error::other("ACP projection limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Proleptic Gregorian UTC, four-digit years and whole seconds, matching the
// pinned sessions.zig output shape. Native negative times use floor division.
// All arithmetic is bounded before the at-most-14-step year and 12-month search.
fn format_utc(milliseconds: i64) -> Option<String> {
    const SECONDS_PER_DAY: i64 = 86_400;
    let seconds = milliseconds.div_euclid(1000);
    let day = seconds.div_euclid(SECONDS_PER_DAY) + days_before_year(1970);
    if !(0..days_before_year(10_000)).contains(&day) {
        return None;
    }
    let (mut low, mut high) = (0, 10_000);
    while high - low > 1 {
        let mid = i64::midpoint(low, high);
        if days_before_year(mid) <= day {
            low = mid;
        } else {
            high = mid;
        }
    }
    let year = low;
    let mut month_day = day - days_before_year(year);
    let month_lengths = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for length in month_lengths {
        if month_day < length {
            break;
        }
        month_day -= length;
        month += 1;
    }
    let within_day = seconds.rem_euclid(SECONDS_PER_DAY);
    Some(format!(
        "{year:04}-{month:02}-{:02}T{:02}:{:02}:{:02}Z",
        month_day + 1,
        within_day / 3600,
        (within_day / 60) % 60,
        within_day % 60
    ))
}
const fn days_before_year(year: i64) -> i64 {
    365 * year + (year + 3) / 4 - (year + 99) / 100 + (year + 399) / 400
}
const fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests;
