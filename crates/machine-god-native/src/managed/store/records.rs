use super::{JournalError, Shared};
use machine_god_core::{
    ManagedAgentMode, ManagedAgentState, ManagedConfiguration, ManagedEvent, ManagedHistoryItem,
    ManagedQueueStatus, ManagedToolActivity, SessionId, SessionIncarnationId,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalTranscript {
    pub session_id: SessionId,
    pub incarnation: SessionIncarnationId,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalPageRef {
    pub child_id: String,
    pub owner: JournalTranscript,
    pub generation: u64,
    pub sequence: u64,
    pub length: usize,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalWork {
    pub id: String,
    pub source_id: String,
    pub source_owner: JournalTranscript,
    pub content: String,
    pub accepted_at_ms: i64,
    #[serde(deserialize_with = "configuration")]
    pub configuration: ManagedConfiguration,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalWorkRef {
    pub id: String,
    pub status: ManagedQueueStatus,
    pub page: JournalPageRef,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalFailure {
    pub work_id: String,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalIntent {
    Cancel,
    Archive,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalHead {
    pub version: u16,
    pub owner_epoch: u64,
    pub id: String,
    pub generation: u64,
    pub revision: u64,
    pub mode: ManagedAgentMode,
    #[serde(deserialize_with = "configuration")]
    pub configuration: ManagedConfiguration,
    pub transcript: JournalTranscript,
    pub controller: JournalTranscript,
    pub parent_id: Option<String>,
    pub parent_owner: Option<JournalTranscript>,
    pub status: ManagedAgentState,
    #[serde(deserialize_with = "queue")]
    pub queue: Vec<JournalWorkRef>,
    pub failure: Option<JournalFailure>,
    pub intent: Option<JournalIntent>,
    pub notice_cursor: u64,
    pub history_tail: Option<JournalPageRef>,
    pub next_sequence: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct JournalCreate {
    pub id: String,
    pub mode: ManagedAgentMode,
    pub configuration: ManagedConfiguration,
    pub transcript: JournalTranscript,
    pub controller: JournalTranscript,
    pub parent_id: Option<String>,
    pub parent_owner: Option<JournalTranscript>,
    pub initial_work: Option<JournalWork>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalRecord {
    Notice(crate::managed::notices::ManagedNotice),
    NoticeAcknowledged {
        identity: crate::managed::notices::NoticeIdentity,
        target: crate::managed::notices::NoticeTarget,
        checkpoint: crate::managed::prompt_context::NoticeCheckpoint,
    },
    WorkAccepted(JournalWork),
    Event(ManagedEvent),
    History(ManagedHistoryItem),
    Tool(ManagedToolActivity),
    Control(JournalControl),
    Configuration(#[serde(deserialize_with = "configuration")] ManagedConfiguration),
    WorkState {
        work_id: String,
        status: ManagedQueueStatus,
        failure: Option<String>,
    },
    WorkResolved {
        work_id: String,
        retry: bool,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalControl {
    pub revision: u64,
    pub status: ManagedAgentState,
    pub intent: Option<JournalIntent>,
    pub failure: Option<JournalFailure>,
    pub controller: JournalTranscript,
    pub parent_id: Option<String>,
    pub parent_owner: Option<JournalTranscript>,
    pub notice_cursor: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredPage {
    pub version: u16,
    pub child_id: String,
    pub owner: JournalTranscript,
    pub generation: u64,
    pub sequence: u64,
    pub previous: Option<JournalPageRef>,
    #[serde(deserialize_with = "records")]
    pub records: Vec<JournalRecord>,
}

#[derive(Clone, Debug)]
pub(crate) enum JournalMutation {
    Enqueue(JournalWork),
    HeadState {
        work_id: String,
        status: ManagedQueueStatus,
        failure: Option<String>,
    },
    Intent(JournalIntent),
    /// Finish a confirmed cancellation intent when no queued work exists.
    CancelIdle,
    ResolveHead {
        work_id: String,
        retry: bool,
    },
    Configure(ManagedConfiguration),
    Relationship {
        parent_id: Option<String>,
        parent_owner: Option<JournalTranscript>,
    },
    NoticeCursor(u64),
    AppendHistory(Vec<JournalRecord>),
    Archive,
    Reopen(JournalTranscript),
    Recover,
}

#[derive(Clone, Debug)]
pub(crate) struct JournalSnapshot {
    pub head: JournalHead,
    pub(super) digest: [u8; 32],
    pub(super) identity: Weak<Shared>,
    pub(super) source_revision: [i128; 11],
}
impl JournalSnapshot {
    pub(crate) fn recovery_required(&self) -> bool {
        self.identity
            .upgrade()
            .is_none_or(|owner| owner.epoch != self.head.owner_epoch)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct JournalReceipt {
    pub(super) identity: Weak<Shared>,
    pub(super) operation: u64,
}
pub(crate) enum JournalPublication {
    Confirmed(Box<JournalSnapshot>),
    Ambiguous(JournalReceipt),
    /// Exact reconciliation established that the referencing head did not publish.
    NotApplied,
}
impl std::fmt::Debug for JournalPublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Confirmed(_) => "Confirmed { .. }",
            Self::Ambiguous(_) => "Ambiguous { .. }",
            Self::NotApplied => "NotApplied",
        })
    }
}
#[derive(Debug)]
pub(crate) struct JournalHistoryPage {
    pub records: Vec<JournalRecord>,
    pub next: Option<JournalHistoryCursor>,
}
#[derive(Clone, Debug)]
pub(crate) struct JournalHistoryCursor {
    pub(super) identity: Weak<Shared>,
    pub(super) snapshot: [u8; 32],
    pub(super) next: JournalPageRef,
    pub(super) offset: usize,
}
#[derive(Clone, Debug)]
pub(crate) struct JournalCatalogCursor {
    pub(super) identity: Weak<Shared>,
    pub(super) after: String,
}
#[derive(Debug)]
pub(crate) struct JournalCatalogEntry {
    pub id: String,
    pub generation: u64,
    pub revision: u64,
    pub name: String,
    pub parent_id: Option<String>,
    pub status: ManagedAgentState,
    pub recovery_required: bool,
}
#[derive(Debug)]
pub(crate) struct JournalCatalogPage {
    pub entries: Vec<JournalCatalogEntry>,
    pub next: Option<JournalCatalogCursor>,
}
pub(super) fn identity_matches(
    expected: &Weak<Shared>,
    actual: &Arc<Shared>,
) -> Result<(), JournalError> {
    if expected.ptr_eq(&Arc::downgrade(actual)) {
        Ok(())
    } else {
        Err(JournalError::Conflict)
    }
}

fn bounded<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded journal list")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = seq.next_element()? {
                if values.len() == N {
                    return Err(serde::de::Error::custom("journal list limit"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
}
fn queue<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<JournalWorkRef>, D::Error> {
    bounded::<D, _, 256>(d)
}
fn records<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<JournalRecord>, D::Error> {
    bounded::<D, _, 101>(d)
}
fn milestones<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    bounded::<D, _, 32>(d)
}
fn stops<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<machine_god_core::ManagedStopCondition>, D::Error> {
    bounded::<D, _, 8>(d)
}

fn configuration<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ManagedConfiguration, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Policy {
        terminal: machine_god_core::ManagedTerminalNotifications,
        started: bool,
        #[serde(deserialize_with = "milestones")]
        milestones: Vec<String>,
        report_interval_ms: Option<u64>,
        report_duration_ms: Option<u64>,
        #[serde(deserialize_with = "stops")]
        stop_conditions: Vec<machine_god_core::ManagedStopCondition>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Config {
        name: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: machine_god_core::ManagedPermissionMode,
        notifications: Policy,
    }
    let config = Config::deserialize(d)?;
    Ok(ManagedConfiguration {
        name: config.name,
        model: config.model,
        effort: config.effort,
        permission_mode: config.permission_mode,
        notifications: machine_god_core::ManagedNotifications {
            terminal: config.notifications.terminal,
            started: config.notifications.started,
            milestones: config.notifications.milestones,
            report_interval_ms: config.notifications.report_interval_ms,
            report_duration_ms: config.notifications.report_duration_ms,
            stop_conditions: config.notifications.stop_conditions,
        },
    })
}
