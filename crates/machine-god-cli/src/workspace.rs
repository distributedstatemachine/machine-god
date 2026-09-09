//! Thin top-level workspace grammar, native ownership composition, and presentation.

use crate::bounded_output::BoundedOutput;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

const MAX_PATH_BYTES: usize = 4096;
const MAX_ENTRIES: usize = 16;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceAction {
    List,
    Add(PathBuf),
    Remove(PathBuf),
    Clear,
}
impl WorkspaceAction {
    const fn name(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Add(_) => "add",
            Self::Remove(_) => "remove",
            Self::Clear => "clear",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkspaceOptions {
    pub(super) action: WorkspaceAction,
    pub(super) json: bool,
}

pub(super) fn parse_options(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<WorkspaceOptions, ()> {
    let mut json = false;
    let mut operands = Vec::with_capacity(2);
    for argument in arguments {
        if argument == "--json" {
            if json {
                return Err(());
            }
            json = true;
        } else {
            let bytes = argument.as_encoded_bytes();
            if bytes.is_empty()
                || bytes.len() > MAX_PATH_BYTES
                || bytes.contains(&0)
                || bytes.starts_with(b"-")
                || operands.len() == 2
            {
                return Err(());
            }
            operands.push(argument);
        }
    }
    let action = match operands.as_slice() {
        [] => WorkspaceAction::List,
        [verb] if verb == "list" => WorkspaceAction::List,
        [verb] if verb == "clear" => WorkspaceAction::Clear,
        [verb, path] if verb == "add" => WorkspaceAction::Add(PathBuf::from(path)),
        [verb, path] if verb == "remove" => WorkspaceAction::Remove(PathBuf::from(path)),
        _ => return Err(()),
    };
    Ok(WorkspaceOptions { action, json })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceOperationalFailure {
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Busy,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    InvalidPath,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    UnknownDirectory,
    ResourceLimit,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    DuplicateRoot,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    OverlappingState,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Conflict,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    InvalidConfiguration,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    UnsafePath,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Persistence,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Ambiguous,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Unavailable,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Unsupported,
}
impl WorkspaceOperationalFailure {
    const fn category(self) -> &'static str {
        match self {
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::Busy => "Busy",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::InvalidPath => "InvalidPath",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::UnknownDirectory => "UnknownDirectory",
            Self::ResourceLimit => "ResourceLimit",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::DuplicateRoot => "DuplicateRoot",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::OverlappingState => "OverlappingState",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::Conflict => "Conflict",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::InvalidConfiguration => "InvalidConfiguration",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::UnsafePath => "UnsafePath",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::Persistence => "Persistence",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::Ambiguous => "Ambiguous",
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            Self::Unavailable => "Unavailable",
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            Self::Unsupported => "Unsupported",
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reconciliation {
    CachedBusy,
    Refreshed,
    Confirmed,
    AmbiguousIntended,
    AmbiguousBefore,
    Indeterminate,
    ReloadFailed(WorkspaceOperationalFailure),
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
impl Reconciliation {
    const fn name(self) -> &'static str {
        match self {
            Self::CachedBusy => "cached_busy",
            Self::Refreshed => "refreshed",
            Self::Confirmed => "confirmed",
            Self::AmbiguousIntended => "ambiguous_intended",
            Self::AmbiguousBefore => "ambiguous_before",
            Self::Indeterminate => "indeterminate",
            Self::ReloadFailed(_) => "reload_failed",
        }
    }
    const fn reload_error(self) -> Option<WorkspaceOperationalFailure> {
        match self {
            Self::ReloadFailed(error) => Some(error),
            _ => None,
        }
    }
    const fn exit(self) -> u8 {
        match self {
            Self::CachedBusy | Self::Refreshed | Self::Confirmed => 0,
            _ => 1,
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", test)))]
#[derive(Clone, Copy, Debug)]
enum Reconciliation {}

// Unsupported hosts cannot produce a successful native receipt. Keeping that
// fact uninhabited avoids inventing runtime facts or native-only constructors.
#[cfg(not(any(target_os = "linux", target_os = "macos", test)))]
impl Reconciliation {
    const fn name(self) -> &'static str {
        match self {}
    }
    const fn exit(self) -> u8 {
        match self {}
    }
    const fn reload_error(self) -> Option<WorkspaceOperationalFailure> {
        match self {}
    }
}

#[derive(Clone, Debug)]
struct Entry {
    source: PathBuf,
    identity: PathBuf,
    identity_canonical: bool,
    provenance: Provenance,
    availability: Availability,
}

#[derive(Clone, Copy, Debug)]
struct Provenance {
    saved: bool,
    launch: bool,
}

#[derive(Clone, Copy, Debug)]
struct Availability {
    available: bool,
    active: bool,
}

/// Owned presentation data only; all policy and state mutation remain native.
#[derive(Clone, Debug)]
pub(super) struct WorkspaceSnapshot {
    primary: PathBuf,
    generation: u64,
    saved_suppressed: bool,
    entries: Vec<Entry>,
    saved_changed: Option<bool>,
    runtime_changed: Option<bool>,
    reconciliation: Reconciliation,
    launch_flag_can_restore: bool,
}

pub(super) trait WorkspaceCommandHost {
    fn execute_workspace(
        &self,
        action: &WorkspaceAction,
    ) -> Result<WorkspaceSnapshot, WorkspaceOperationalFailure>;
}
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ProductionWorkspaceCommandHost;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod native;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl WorkspaceCommandHost for ProductionWorkspaceCommandHost {
    fn execute_workspace(
        &self,
        _: &WorkspaceAction,
    ) -> Result<WorkspaceSnapshot, WorkspaceOperationalFailure> {
        Err(WorkspaceOperationalFailure::Unsupported)
    }
}

pub(super) fn run_workspace(
    host: &(impl WorkspaceCommandHost + ?Sized),
    options: &WorkspaceOptions,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let snapshot = match host.execute_workspace(&options.action) {
        Ok(snapshot) => snapshot,
        Err(error) => return write_failure(error, options, stdout, stderr),
    };
    let output = match render(&snapshot, options) {
        Ok(output) => output,
        Err(error) => return write_failure(error, options, stdout, stderr),
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(super::OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    snapshot.reconciliation.exit()
}

fn write_failure(
    error: WorkspaceOperationalFailure,
    options: &WorkspaceOptions,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let category = error.category();
    let output = if options.json {
        format!(
            "{{\"kind\":\"workspace\",\"action\":\"{}\",\"error\":\"workspace operation failed\",\"code\":\"{category}\"}}\n",
            options.action.name()
        )
    } else {
        format!("machine-god workspace: operation failed: {category}\n")
    };
    let failed = if options.json {
        stdout.write_all(output.as_bytes()).is_err()
    } else {
        stderr.write_all(output.as_bytes()).is_err()
    };
    if failed {
        let _ = stderr.write_all(super::OUTPUT_FAILURE.as_bytes());
    }
    1
}

fn validate_path(path: &Path, absolute: bool) -> Result<(), WorkspaceOperationalFailure> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_PATH_BYTES
        || bytes.contains(&0)
        || (absolute && !path.is_absolute())
    {
        return Err(WorkspaceOperationalFailure::ResourceLimit);
    }
    Ok(())
}

fn render(
    snapshot: &WorkspaceSnapshot,
    options: &WorkspaceOptions,
) -> Result<String, WorkspaceOperationalFailure> {
    validate_path(&snapshot.primary, true)?;
    if snapshot.entries.len() > MAX_ENTRIES {
        return Err(WorkspaceOperationalFailure::ResourceLimit);
    }
    for entry in &snapshot.entries {
        validate_path(&entry.source, false)?;
        validate_path(&entry.identity, true)?;
    }
    let mut output = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 8192);
    if options.json {
        render_json(&mut output, snapshot, &options.action)
    } else {
        render_human(&mut output, snapshot, &options.action)
    }
    .map_err(|_| WorkspaceOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn write_path(output: &mut BoundedOutput, path: &Path, json: bool) -> std::fmt::Result {
    if json {
        output.write_str("{\"text\":")?;
    }
    if let Some(text) = path.to_str() {
        super::write_json_string(output, text)?;
        if json {
            output.write_str(",\"bytes_hex\":null}")?;
        }
    } else {
        output.write_str(if json {
            "null,\"bytes_hex\":\""
        } else {
            "bytes_hex=\""
        })?;
        for byte in path.as_os_str().as_encoded_bytes() {
            write!(output, "{byte:02x}")?;
        }
        output.write_str(if json { "\"}" } else { "\"" })?;
    }
    Ok(())
}

const fn optional(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    }
}

fn render_json(
    output: &mut BoundedOutput,
    snapshot: &WorkspaceSnapshot,
    action: &WorkspaceAction,
) -> std::fmt::Result {
    write!(
        output,
        "{{\"kind\":\"workspace\",\"action\":\"{}\",\"primary_directory\":",
        action.name()
    )?;
    write_path(output, &snapshot.primary, true)?;
    write!(
        output,
        ",\"generation\":{},\"saved_suppressed\":{},\"additional_directories\":[",
        snapshot.generation, snapshot.saved_suppressed
    )?;
    for (index, entry) in snapshot.entries.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        output.write_str("{\"source\":")?;
        write_path(output, &entry.source, true)?;
        output.write_str(",\"identity\":")?;
        write_path(output, &entry.identity, true)?;
        write!(
            output,
            ",\"identity_canonical\":{},\"saved\":{},\"launch\":{},\"available\":{},\"active\":{}}}",
            entry.identity_canonical,
            entry.provenance.saved,
            entry.provenance.launch,
            entry.availability.available,
            entry.availability.active
        )?;
    }
    write!(
        output,
        "],\"saved_changed\":{},\"runtime_changed\":{},\"reconciliation\":\"{}\",\"reconciliation_error\":",
        optional(snapshot.saved_changed),
        optional(snapshot.runtime_changed),
        snapshot.reconciliation.name()
    )?;
    if let Some(error) = snapshot.reconciliation.reload_error() {
        super::write_json_string(output, error.category())?;
    } else {
        output.write_str("null")?;
    }
    writeln!(
        output,
        ",\"launch_flag_can_restore\":{}}}",
        snapshot.launch_flag_can_restore
    )
}

fn render_human(
    output: &mut BoundedOutput,
    snapshot: &WorkspaceSnapshot,
    action: &WorkspaceAction,
) -> std::fmt::Result {
    write!(output, "[workspace] action={} primary=", action.name())?;
    write_path(output, &snapshot.primary, false)?;
    writeln!(
        output,
        " generation={} saved_suppressed={}",
        snapshot.generation, snapshot.saved_suppressed
    )?;
    writeln!(
        output,
        "[workspace] additional_directories={}",
        snapshot.entries.len()
    )?;
    for entry in &snapshot.entries {
        output.write_str("[workspace] source=")?;
        write_path(output, &entry.source, false)?;
        output.write_str(" identity=")?;
        write_path(output, &entry.identity, false)?;
        writeln!(
            output,
            " identity_canonical={} saved={} launch={} available={} active={}",
            entry.identity_canonical,
            entry.provenance.saved,
            entry.provenance.launch,
            entry.availability.available,
            entry.availability.active
        )?;
    }
    write!(
        output,
        "[workspace] saved_changed={} runtime_changed={} reconciliation={}",
        optional(snapshot.saved_changed),
        optional(snapshot.runtime_changed),
        snapshot.reconciliation.name()
    )?;
    if let Some(error) = snapshot.reconciliation.reload_error() {
        write!(output, " reconciliation_error={}", error.category())?;
    }
    writeln!(
        output,
        " launch_flag_can_restore={}",
        snapshot.launch_flag_can_restore
    )
}

#[cfg(test)]
mod tests;
