//! Composition of explicit native background selection, logs and desktop handoff.

use super::url::{MAX_BACKGROUND_URL_INPUT_BYTES, detect_server_url};
use super::{NativeBackgroundCommand, NativeBackgroundTarget};
use crate::{
    NativeBackgroundOpenError, NativeBackgroundOpenOutcome, NativeBackgroundUrlOpener,
    NativeTerminalBackgroundError, NativeTerminalBackgroundRequester,
    NativeTerminalBackgroundSnapshot, NativeTerminalBackgroundStopReceipt,
    NativeTerminalBackgroundTarget,
};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, TerminalCursor, TerminalLifecycle,
    TerminalSessionId,
};
use std::fmt;

mod requests;
pub(crate) use requests::BackgroundRequests;

const LOG_WINDOW_BYTES: usize = 16 * 1024;
const MAX_WINDOW_PAGES: usize = 1024;

#[derive(Debug)]
pub enum NativeBackgroundControlError {
    Terminal(NativeTerminalBackgroundError),
    Open(NativeBackgroundOpenError),
    ResourceLimit,
    Unavailable,
}
impl fmt::Display for NativeBackgroundControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use NativeTerminalBackgroundError as Terminal;
        formatter.write_str(match self {
            Self::Terminal(Terminal::NotFound) => "no matching background history",
            Self::Terminal(Terminal::Cancelled)
            | Self::Open(NativeBackgroundOpenError::Cancelled) => "background command cancelled",
            Self::Terminal(Terminal::Revoked) => "background access revoked",
            Self::Terminal(Terminal::Closed) => "background host closed",
            Self::Terminal(Terminal::Committed) => {
                "background effect committed; final receipt unavailable"
            }
            Self::Terminal(Terminal::Uncertain) => {
                "background outcome uncertain; effects may have occurred"
            }
            Self::Terminal(Terminal::ResourceLimit) | Self::ResourceLimit => {
                "background resource limit reached"
            }
            Self::Open(NativeBackgroundOpenError::Busy) => "previous URL launcher still owned",
            Self::Open(NativeBackgroundOpenError::TimedOut) => {
                "URL launcher deadline expired before launch"
            }
            Self::Open(_) => "background URL launcher unavailable",
            Self::Terminal(Terminal::Invalid | Terminal::Unavailable) | Self::Unavailable => {
                "background command unavailable"
            }
        })
    }
}
impl std::error::Error for NativeBackgroundControlError {}
impl From<NativeTerminalBackgroundError> for NativeBackgroundControlError {
    fn from(value: NativeTerminalBackgroundError) -> Self {
        Self::Terminal(value)
    }
}
type Result<T> = std::result::Result<T, NativeBackgroundControlError>;

/// A bounded contiguous captured window. Presentation may further select lines.
pub struct NativeBackgroundLogWindow {
    bytes: Vec<u8>,
    source: TerminalCursor,
    next: TerminalCursor,
    snapshot_end: TerminalCursor,
    gap: bool,
    truncated: bool,
}
impl NativeBackgroundLogWindow {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub fn source(&self) -> &TerminalCursor {
        &self.source
    }
    #[must_use]
    pub fn next(&self) -> &TerminalCursor {
        &self.next
    }
    #[must_use]
    pub fn snapshot_end(&self) -> &TerminalCursor {
        &self.snapshot_end
    }
    #[must_use]
    pub fn has_gap(&self) -> bool {
        self.gap
    }
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

pub struct NativeBackgroundLogSummary {
    session_id: TerminalSessionId,
    head: NativeBackgroundLogWindow,
    tail: NativeBackgroundLogWindow,
}
impl NativeBackgroundLogSummary {
    #[must_use]
    pub fn session_id(&self) -> &TerminalSessionId {
        &self.session_id
    }
    #[must_use]
    pub fn head(&self) -> &NativeBackgroundLogWindow {
        &self.head
    }
    #[must_use]
    pub fn tail(&self) -> &NativeBackgroundLogWindow {
        &self.tail
    }
}

/// Exact native outcomes; display adapters cannot reinterpret historical closure
/// or a lost launcher receipt as successful process control.
pub enum NativeBackgroundControlReceipt {
    Listed(NativeTerminalBackgroundSnapshot),
    Stopped {
        session_id: TerminalSessionId,
        receipt: NativeTerminalBackgroundStopReceipt,
    },
    Logs(NativeBackgroundLogSummary),
    Opened {
        session_id: TerminalSessionId,
        url: String,
        outcome: NativeBackgroundOpenOutcome,
    },
    NoKnownUrl {
        session_id: TerminalSessionId,
    },
    NotRunning {
        session_id: TerminalSessionId,
    },
}
impl NativeBackgroundControlReceipt {
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(
            self,
            Self::Opened {
                outcome: NativeBackgroundOpenOutcome::LauncherFailed
                    | NativeBackgroundOpenOutcome::Indeterminate,
                ..
            }
        )
    }
}

macro_rules! redacted {
    ($($name:ty),+ $(,)?) => {$(
        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted!(
    NativeBackgroundLogWindow,
    NativeBackgroundLogSummary,
    NativeBackgroundControlReceipt
);

pub(crate) fn execute(
    requester: NativeTerminalBackgroundRequester,
    owner: BackgroundOutputOwner,
    command: NativeBackgroundCommand,
    opener: Option<NativeBackgroundUrlOpener>,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeBackgroundControlReceipt>> {
    execute_with(requester, owner, command, opener, cancellation)
}

pub(crate) fn execute_with(
    requester: impl BackgroundRequests,
    owner: BackgroundOutputOwner,
    command: NativeBackgroundCommand,
    opener: Option<NativeBackgroundUrlOpener>,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeBackgroundControlReceipt>> {
    Box::pin(async move {
        use NativeBackgroundControlReceipt as Receipt;
        let selection = match command {
            NativeBackgroundCommand::List => {
                return requester
                    .snapshot(owner, cancellation)
                    .await
                    .map(Receipt::Listed)
                    .map_err(Into::into);
            }
            NativeBackgroundCommand::Stop(ref target)
            | NativeBackgroundCommand::Open(ref target)
            | NativeBackgroundCommand::Logs(ref target) => match target {
                NativeBackgroundTarget::Last => None,
                NativeBackgroundTarget::Session(id) => Some(id.clone()),
            },
        };
        let target = requester
            .select(owner, selection, cancellation.clone())
            .await?;
        let session_id = target.id().clone();
        match command {
            NativeBackgroundCommand::List => unreachable!("listing returned before selection"),
            NativeBackgroundCommand::Stop(_) => {
                let receipt = requester.stop(&target, cancellation).await?;
                Ok(Receipt::Stopped {
                    session_id,
                    receipt,
                })
            }
            NativeBackgroundCommand::Logs(_) => {
                let head = read_window(
                    &requester,
                    &target,
                    initial_cursor(),
                    LOG_WINDOW_BYTES,
                    &cancellation,
                )
                .await?;
                let tail_cursor = requester
                    .tail_start(&target, LOG_WINDOW_BYTES, cancellation.clone())
                    .await?;
                let tail = read_window(
                    &requester,
                    &target,
                    tail_cursor,
                    LOG_WINDOW_BYTES,
                    &cancellation,
                )
                .await?;
                Ok(Receipt::Logs(NativeBackgroundLogSummary {
                    session_id,
                    head,
                    tail,
                }))
            }
            NativeBackgroundCommand::Open(_) => {
                let inspection = requester.inspect(&target, cancellation.clone()).await?;
                if !is_running(&inspection) {
                    return Ok(Receipt::NotRunning { session_id });
                }
                let opener = opener.ok_or(NativeBackgroundControlError::Open(
                    NativeBackgroundOpenError::Unavailable,
                ))?;
                let evidence = read_window(
                    &requester,
                    &target,
                    initial_cursor(),
                    MAX_BACKGROUND_URL_INPUT_BYTES,
                    &cancellation,
                )
                .await?;
                let Some(url) = detect_server_url(evidence.bytes())
                    .map_err(|_| NativeBackgroundControlError::ResourceLimit)?
                else {
                    return Ok(Receipt::NoKnownUrl { session_id });
                };
                // Reading output is asynchronous: re-observe this exact selected
                // target, never resolve `last` again, before launch admission.
                if !is_running(&requester.inspect(&target, cancellation.clone()).await?) {
                    return Ok(Receipt::NotRunning { session_id });
                }
                let displayed_url = url.as_str().to_owned();
                let outcome = opener
                    .open(url, cancellation, target.revocation_token())
                    .await
                    .map_err(NativeBackgroundControlError::Open)?;
                Ok(Receipt::Opened {
                    session_id,
                    url: displayed_url,
                    outcome,
                })
            }
        }
    })
}

fn initial_cursor() -> TerminalCursor {
    TerminalCursor::new(1, 0).expect("fixed nonzero initial segment")
}

fn is_running(inspection: &crate::NativeTerminalBackgroundInspection) -> bool {
    inspection.owns_backend()
        && inspection
            .result()
            .session()
            .is_some_and(|facts| facts.lifecycle == TerminalLifecycle::Running)
}

async fn read_window(
    requester: &impl BackgroundRequests,
    target: &NativeTerminalBackgroundTarget,
    mut cursor: TerminalCursor,
    maximum: usize,
    cancellation: &CancellationToken,
) -> Result<NativeBackgroundLogWindow> {
    let mut bytes = Vec::new();
    let mut source = cursor.clone();
    let mut end: Option<TerminalCursor> = None;
    let mut gap = false;
    for _ in 0..MAX_WINDOW_PAGES {
        if end.as_ref().is_some_and(|end| &cursor >= end) || bytes.len() == maximum {
            break;
        }
        let mut remaining = maximum - bytes.len();
        if let Some(end) = &end
            && cursor.segment() == end.segment()
        {
            remaining = remaining.min(
                usize::try_from(end.offset().saturating_sub(cursor.offset())).unwrap_or(usize::MAX),
            );
        }
        if remaining == 0 {
            break;
        }
        let page = requester
            .read(target, cursor.clone(), remaining, cancellation.clone())
            .await?;
        let captured_end = end.get_or_insert_with(|| page.latest().clone());
        if page.gap().is_some() {
            gap = true;
            // Do not join two discontinuous spans into synthetic log/URL text.
            if !bytes.is_empty() {
                break;
            }
        }
        // A read at the end of a segment can return the next segment without
        // a retention gap. Native pages are single-segment contiguous spans;
        // derive their start from the returned end, not the requested cursor.
        let actual_start = TerminalCursor::new(
            page.next().segment(),
            page.next()
                .offset()
                .checked_sub(page.bytes().len() as u64)
                .ok_or(NativeBackgroundControlError::Unavailable)?,
        )
        .map_err(|_| NativeBackgroundControlError::Unavailable)?;
        if bytes.is_empty() {
            source = actual_start.clone();
        }
        if &actual_start >= captured_end {
            cursor = actual_start.clone();
            break;
        }
        let mut accepted = page.bytes().len().min(remaining);
        if actual_start.segment() == captured_end.segment() {
            accepted = accepted.min(
                usize::try_from(captured_end.offset().saturating_sub(actual_start.offset()))
                    .unwrap_or(usize::MAX),
            );
        }
        bytes.extend_from_slice(&page.bytes()[..accepted]);
        if page.next() <= &cursor {
            if page.bytes().is_empty() {
                break;
            }
            return Err(NativeBackgroundControlError::Unavailable);
        }
        cursor = if accepted < page.bytes().len() {
            captured_end.clone()
        } else {
            page.next().clone()
        };
    }
    let snapshot_end = end.ok_or(NativeBackgroundControlError::Unavailable)?;
    let truncated = cursor < snapshot_end;
    Ok(NativeBackgroundLogWindow {
        bytes,
        source,
        next: cursor,
        snapshot_end,
        gap,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_failure_and_uncertainty_are_not_success() {
        for outcome in [
            NativeBackgroundOpenOutcome::Opened,
            NativeBackgroundOpenOutcome::LauncherFailed,
            NativeBackgroundOpenOutcome::Indeterminate,
        ] {
            let receipt = NativeBackgroundControlReceipt::Opened {
                session_id: TerminalSessionId::new("terminal-00000000000000000000000000000001")
                    .unwrap(),
                url: "http://localhost:3000/private-url".to_owned(),
                outcome,
            };
            assert_eq!(
                receipt.failed(),
                outcome != NativeBackgroundOpenOutcome::Opened
            );
            assert!(!format!("{receipt:?}").contains("private-url"));
        }
    }

    #[test]
    fn log_debug_hides_bytes_and_identity() {
        let window = NativeBackgroundLogWindow {
            bytes: b"private-output".to_vec(),
            source: initial_cursor(),
            next: initial_cursor(),
            snapshot_end: initial_cursor(),
            gap: true,
            truncated: true,
        };
        assert!(!format!("{window:?}").contains("private-output"));
        assert!(window.has_gap());
        assert!(window.truncated());
        assert_eq!(window.bytes(), b"private-output");
    }

    #[test]
    fn errors_distinguish_no_match_from_committed_or_uncertain_effects() {
        let error = |error| NativeBackgroundControlError::Terminal(error).to_string();
        assert!(error(NativeTerminalBackgroundError::NotFound).contains("no matching"));
        assert!(error(NativeTerminalBackgroundError::Committed).contains("committed"));
        assert!(error(NativeTerminalBackgroundError::Uncertain).contains("uncertain"));
    }
}
