//! Read-only union of legacy background records and committed terminal histories.

use machine_god_core::{BoxFuture, TerminalCursor, TerminalLifecycle, TerminalSessionId};
use std::{fmt, path::PathBuf};

use crate::{NativeBackgroundDetail, NativeBackgroundInspectionError, NativeEnvironment};

/// Both sources retain their independent bounds: 100 legacy and 128 terminal rows.
pub const MAX_BACKGROUND_HISTORY_RECORDS: usize = crate::MAX_BACKGROUND_RECORDS + 128;

/// Separate numeric legacy and opaque terminal identity domains.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum NativeBackgroundHistoryId {
    Legacy(u64),
    Terminal(TerminalSessionId),
}
impl fmt::Debug for NativeBackgroundHistoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeBackgroundHistoryId { .. }")
    }
}

/// Inert selection; identifiers never confer process or workspace authority.
#[derive(Clone, Eq, PartialEq)]
pub enum NativeBackgroundHistoryQuery {
    List,
    Last,
    Legacy(u64),
    Terminal(TerminalSessionId),
}
impl fmt::Debug for NativeBackgroundHistoryQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeBackgroundHistoryQuery { .. }")
    }
}

/// Bounded recorded-state list row. The timestamp is losslessly widened from
/// legacy unsigned update time or terminal signed last-output time.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeBackgroundHistorySummary {
    id: NativeBackgroundHistoryId,
    state: &'static str,
    updated_at_ms: i128,
    command_preview: String,
    preview_truncated: bool,
}
impl NativeBackgroundHistorySummary {
    #[must_use]
    pub const fn id(&self) -> &NativeBackgroundHistoryId {
        &self.id
    }
    #[must_use]
    pub const fn state(&self) -> &'static str {
        self.state
    }
    #[must_use]
    pub const fn updated_at_ms(&self) -> i128 {
        self.updated_at_ms
    }
    #[must_use]
    pub fn command_preview(&self) -> &str {
        &self.command_preview
    }
    #[must_use]
    pub const fn preview_truncated(&self) -> bool {
        self.preview_truncated
    }
}
impl fmt::Debug for NativeBackgroundHistorySummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeBackgroundHistorySummary { .. }")
    }
}

/// Newest timestamp first; ties put terminal IDs before legacy IDs, then sort
/// each identity domain descending. No atomic cross-store snapshot is promised.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeBackgroundHistoryList {
    records: Vec<NativeBackgroundHistorySummary>,
    truncated: bool,
}
impl NativeBackgroundHistoryList {
    #[must_use]
    pub fn records(&self) -> &[NativeBackgroundHistorySummary] {
        &self.records
    }
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}
impl fmt::Debug for NativeBackgroundHistoryList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeBackgroundHistoryList { .. }")
    }
}

/// Data from one committed terminal history, never recovered live authority.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeBackgroundTerminalDetail {
    id: TerminalSessionId,
    owner: machine_god_core::BackgroundOutputOwner,
    workspace: String,
    created_at_ms: i64,
    last_output_ms: i64,
    lifecycle: TerminalLifecycle,
    command: Option<String>,
    cwd: String,
    exit_code: Option<i32>,
    signal: Option<i32>,
    earliest: TerminalCursor,
    latest: TerminalCursor,
    facts_cursor: TerminalCursor,
}
impl NativeBackgroundTerminalDetail {
    #[must_use]
    pub const fn owner(&self) -> &machine_god_core::BackgroundOutputOwner {
        &self.owner
    }
    #[must_use]
    pub fn workspace(&self) -> &str {
        &self.workspace
    }
    #[must_use]
    pub const fn id(&self) -> &TerminalSessionId {
        &self.id
    }
    #[must_use]
    pub const fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }
    #[must_use]
    pub const fn last_output_ms(&self) -> i64 {
        self.last_output_ms
    }
    #[must_use]
    pub const fn lifecycle(&self) -> TerminalLifecycle {
        self.lifecycle
    }
    #[must_use]
    pub const fn state(&self) -> &'static str {
        match self.lifecycle {
            TerminalLifecycle::Starting => "starting",
            TerminalLifecycle::Running => "running",
            TerminalLifecycle::Exited => "exited",
            TerminalLifecycle::Lost => "lost",
            TerminalLifecycle::Closed => "closed",
        }
    }
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        self.command.as_deref()
    }
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    #[must_use]
    pub const fn signal(&self) -> Option<i32> {
        self.signal
    }
    #[must_use]
    pub const fn earliest(&self) -> &TerminalCursor {
        &self.earliest
    }
    #[must_use]
    pub const fn latest(&self) -> &TerminalCursor {
        &self.latest
    }
    #[must_use]
    pub const fn facts_cursor(&self) -> &TerminalCursor {
        &self.facts_cursor
    }
}
impl fmt::Debug for NativeBackgroundTerminalDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeBackgroundTerminalDetail { .. }")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeBackgroundHistoryDetail {
    Legacy(NativeBackgroundDetail),
    Terminal(NativeBackgroundTerminalDetail),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeBackgroundHistoryInspection {
    List(NativeBackgroundHistoryList),
    Detail(NativeBackgroundHistoryDetail),
}

/// All canonicalization and descriptor/store work starts on first poll and
/// completes synchronously. No runtime, recovery, worker or process is created.
#[must_use]
pub fn inspect_native_background_history(
    environment: NativeEnvironment,
    workspace: PathBuf,
    query: NativeBackgroundHistoryQuery,
) -> BoxFuture<'static, Result<NativeBackgroundHistoryInspection, NativeBackgroundInspectionError>>
{
    Box::pin(async move {
        if let NativeBackgroundHistoryQuery::Legacy(id) = query {
            return crate::inspect_native_background(
                environment,
                workspace,
                crate::NativeBackgroundQuery::Id(id),
            )
            .await
            .and_then(legacy_detail);
        }
        #[cfg(all(
            any(test, feature = "ai-gateway-http"),
            any(target_os = "linux", target_os = "macos")
        ))]
        {
            supported::inspect(&environment, &workspace, &query)
        }
        #[cfg(not(all(
            any(test, feature = "ai-gateway-http"),
            any(target_os = "linux", target_os = "macos")
        )))]
        {
            let _ = (environment, workspace, query);
            Err(NativeBackgroundInspectionError::new(
                crate::NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
            ))
        }
    })
}

/// Captures only state-base inputs and the current workspace on first poll.
#[must_use]
pub fn inspect_process_background_history(
    query: NativeBackgroundHistoryQuery,
) -> BoxFuture<'static, Result<NativeBackgroundHistoryInspection, NativeBackgroundInspectionError>>
{
    Box::pin(async move {
        if let NativeBackgroundHistoryQuery::Legacy(id) = query {
            return crate::inspect_process_background(crate::NativeBackgroundQuery::Id(id))
                .await
                .and_then(legacy_detail);
        }
        #[cfg(all(
            any(test, feature = "ai-gateway-http"),
            any(target_os = "linux", target_os = "macos")
        ))]
        {
            use crate::state_environment::{
                ProcessStateEnvironmentReader, capture_state_environment,
            };
            let environment = capture_state_environment(&mut ProcessStateEnvironmentReader);
            let workspace = std::env::current_dir().map_err(|_| {
                NativeBackgroundInspectionError::new(
                    crate::NativeBackgroundInspectionErrorKind::Unavailable,
                )
            })?;
            supported::inspect(&environment, &workspace, &query)
        }
        #[cfg(not(all(
            any(test, feature = "ai-gateway-http"),
            any(target_os = "linux", target_os = "macos")
        )))]
        {
            let _ = query;
            Err(NativeBackgroundInspectionError::new(
                crate::NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
            ))
        }
    })
}

fn legacy_detail(
    inspection: crate::NativeBackgroundInspection,
) -> Result<NativeBackgroundHistoryInspection, NativeBackgroundInspectionError> {
    match inspection {
        crate::NativeBackgroundInspection::Detail(detail) => {
            Ok(NativeBackgroundHistoryInspection::Detail(
                NativeBackgroundHistoryDetail::Legacy(detail),
            ))
        }
        crate::NativeBackgroundInspection::List(_) => Err(NativeBackgroundInspectionError::new(
            crate::NativeBackgroundInspectionErrorKind::Corrupt,
        )),
    }
}

#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
mod supported {
    use super::{
        MAX_BACKGROUND_HISTORY_RECORDS, NativeBackgroundDetail, NativeBackgroundHistoryDetail,
        NativeBackgroundHistoryId, NativeBackgroundHistoryInspection, NativeBackgroundHistoryList,
        NativeBackgroundHistoryQuery, NativeBackgroundHistorySummary,
        NativeBackgroundInspectionError, NativeBackgroundTerminalDetail, NativeEnvironment,
    };
    use crate::background_inspection::supported as legacy;
    use crate::background_terminal_inspection::{
        NativeTerminalBackgroundHistoryError, NativeTerminalBackgroundHistoryRecord,
        inspect_terminal_background_history,
    };
    use crate::{NativeBackgroundInspectionErrorKind as Kind, NativeBackgroundList};
    use machine_god_core::CancellationToken;
    use std::path::Path;

    pub(super) fn inspect(
        environment: &NativeEnvironment,
        workspace: &Path,
        query: &NativeBackgroundHistoryQuery,
    ) -> Result<NativeBackgroundHistoryInspection, NativeBackgroundInspectionError> {
        let workspace = legacy::canonical_workspace(workspace)?;
        let Some(root) = legacy::open_state_hierarchy(environment)? else {
            return if matches!(query, NativeBackgroundHistoryQuery::List) {
                Ok(NativeBackgroundHistoryInspection::List(
                    NativeBackgroundHistoryList {
                        records: Vec::new(),
                        truncated: false,
                    },
                ))
            } else {
                Err(error(Kind::NotFound))
            };
        };
        let cancellation = CancellationToken::new();
        let exact = match query {
            NativeBackgroundHistoryQuery::Terminal(id) => Some(id),
            _ => None,
        };
        let terminal = inspect_terminal_background_history(&root, &workspace, exact, &cancellation)
            .map_err(terminal_error)?;
        if exact.is_some() {
            if terminal.len() != 1 {
                return Err(error(Kind::Corrupt));
            }
            return Ok(NativeBackgroundHistoryInspection::Detail(
                NativeBackgroundHistoryDetail::Terminal(terminal_detail(
                    terminal
                        .into_iter()
                        .next()
                        .ok_or_else(|| error(Kind::NotFound))?,
                )),
            ));
        }
        let (list, latest) = legacy::list_retained(&root, &workspace, &cancellation)?;
        compose(
            &list,
            latest,
            terminal,
            matches!(query, NativeBackgroundHistoryQuery::Last),
        )
    }

    pub(super) fn compose(
        legacy: &NativeBackgroundList,
        legacy_latest: Option<NativeBackgroundDetail>,
        terminal: Vec<NativeTerminalBackgroundHistoryRecord>,
        last: bool,
    ) -> Result<NativeBackgroundHistoryInspection, NativeBackgroundInspectionError> {
        if terminal.len() > 128
            || legacy.records().len() + terminal.len() > MAX_BACKGROUND_HISTORY_RECORDS
        {
            return Err(error(Kind::ResourceLimit));
        }
        if last && legacy.truncated() {
            return Err(error(Kind::ResourceLimit));
        }
        let terminal: Vec<_> = terminal.into_iter().map(terminal_detail).collect();
        let mut records = Vec::with_capacity(legacy.records().len() + terminal.len());
        records.extend(
            legacy
                .records()
                .iter()
                .map(|record| NativeBackgroundHistorySummary {
                    id: NativeBackgroundHistoryId::Legacy(record.id()),
                    state: record.state().as_str(),
                    updated_at_ms: i128::from(record.updated_at_ms()),
                    command_preview: record.command_preview().to_owned(),
                    preview_truncated: record.preview_truncated(),
                }),
        );
        records.extend(terminal.iter().map(|record| {
            let command = record.command().unwrap_or_default();
            let boundary = command.floor_char_boundary(crate::MAX_BACKGROUND_COMMAND_PREVIEW_BYTES);
            NativeBackgroundHistorySummary {
                id: NativeBackgroundHistoryId::Terminal(record.id.clone()),
                state: record.state(),
                updated_at_ms: i128::from(record.last_output_ms),
                command_preview: command[..boundary].to_owned(),
                preview_truncated: boundary < command.len(),
            }
        }));
        records.sort_unstable_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        if !last {
            return Ok(NativeBackgroundHistoryInspection::List(
                NativeBackgroundHistoryList {
                    records,
                    truncated: legacy.truncated(),
                },
            ));
        }
        let first = records.first().ok_or_else(|| error(Kind::NotFound))?;
        let detail = match &first.id {
            NativeBackgroundHistoryId::Legacy(id) => {
                let record = legacy_latest
                    .filter(|record| {
                        record.id() == *id
                            && i128::from(record.updated_at_ms()) == first.updated_at_ms
                    })
                    .ok_or_else(|| error(Kind::Corrupt))?;
                NativeBackgroundHistoryDetail::Legacy(record)
            }
            NativeBackgroundHistoryId::Terminal(id) => {
                let record = terminal
                    .into_iter()
                    .find(|record| record.id() == id)
                    .ok_or_else(|| error(Kind::Corrupt))?;
                NativeBackgroundHistoryDetail::Terminal(record)
            }
        };
        Ok(NativeBackgroundHistoryInspection::Detail(detail))
    }

    fn terminal_detail(
        record: NativeTerminalBackgroundHistoryRecord,
    ) -> NativeBackgroundTerminalDetail {
        use crate::terminal_monitor::TerminalProcessOutcome;
        let (exit_code, signal) = match record.outcome {
            Some(TerminalProcessOutcome::Exited(code)) => (Some(code), None),
            Some(TerminalProcessOutcome::Signaled(signal)) => (None, Some(signal)),
            None => (None, None),
        };
        NativeBackgroundTerminalDetail {
            owner: record.owner,
            workspace: record.workspace,
            id: record.session_id,
            created_at_ms: record.created_at_ms,
            last_output_ms: record.last_output_ms,
            lifecycle: record.lifecycle,
            command: record.command,
            cwd: record.cwd,
            exit_code,
            signal,
            earliest: record.earliest,
            latest: record.latest,
            facts_cursor: record.facts_cursor,
        }
    }

    fn terminal_error(
        failure: NativeTerminalBackgroundHistoryError,
    ) -> NativeBackgroundInspectionError {
        use NativeTerminalBackgroundHistoryError as E;
        error(match failure {
            E::NotFound => Kind::NotFound,
            E::Corrupt | E::Invalid => Kind::Corrupt,
            E::ResourceLimit => Kind::ResourceLimit,
            E::Busy | E::Cancelled | E::Unavailable => Kind::Unavailable,
        })
    }
    fn error(kind: Kind) -> NativeBackgroundInspectionError {
        NativeBackgroundInspectionError::new(kind)
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;
