//! Bounded, data-only session facts and monitor state at one committed cursor.
//! These records never contain a PID, live input lease, or process capability.

use crate::terminal_monitor::{
    TerminalMonitorContext, TerminalMonitorError, TerminalMonitorSet, TerminalProcessOutcome,
};
use machine_god_core::{
    BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalCursor, TerminalGap,
    TerminalLifecycle, TerminalSessionId,
};
use serde::{Deserialize, Serialize};
use std::fmt;

const MAGIC: &[u8; 8] = b"MGTS\0\0\0\x01";
const HEADER_BYTES: usize = MAGIC.len() + 4;
const MAX_FACT_BYTES: usize = 64 * 1024;
const MAX_MONITOR_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalSessionRecordError {
    Invalid,
    Monitor(TerminalMonitorError),
}
impl From<TerminalMonitorError> for TerminalSessionRecordError {
    fn from(error: TerminalMonitorError) -> Self {
        Self::Monitor(error)
    }
}
type Result<T> = std::result::Result<T, TerminalSessionRecordError>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalSessionFacts {
    pub(crate) session_id: TerminalSessionId,
    owner_session_id: SessionId,
    owner_incarnation_id: SessionIncarnationId,
    pub(crate) context: TerminalMonitorContext,
    pub(crate) created_at_ms: i64,
    pub(crate) last_output_ms: i64,
    pub(crate) outcome: Option<TerminalProcessOutcome>,
    pub(crate) observation_gap: Option<TerminalGap>,
}
impl fmt::Debug for TerminalSessionFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalSessionFacts")
            .finish_non_exhaustive()
    }
}
impl TerminalSessionFacts {
    pub(crate) fn new(
        session_id: TerminalSessionId,
        owner: &BackgroundOutputOwner,
        context: TerminalMonitorContext,
        created_at_ms: i64,
        last_output_ms: i64,
        outcome: Option<TerminalProcessOutcome>,
    ) -> Result<Self> {
        let facts = Self {
            session_id,
            owner_session_id: owner.session_id().clone(),
            owner_incarnation_id: owner.session_incarnation_id().clone(),
            context,
            created_at_ms,
            last_output_ms,
            outcome,
            observation_gap: None,
        };
        facts.validate()?;
        Ok(facts)
    }

    pub(crate) fn owned_by(&self, owner: &BackgroundOutputOwner) -> bool {
        &self.owner_session_id == owner.session_id()
            && &self.owner_incarnation_id == owner.session_incarnation_id()
    }

    fn validate(&self) -> Result<()> {
        let valid_outcome = match self.outcome {
            None => self.context.lifecycle != TerminalLifecycle::Exited,
            Some(outcome) => {
                matches!(
                    self.context.lifecycle,
                    TerminalLifecycle::Exited | TerminalLifecycle::Closed
                ) && match outcome {
                    TerminalProcessOutcome::Exited(code) => (0..=255).contains(&code),
                    TerminalProcessOutcome::Signaled(signal) => (1..=255).contains(&signal),
                }
            }
        };
        require(
            self.created_at_ms >= 0
                && self.last_output_ms >= self.created_at_ms
                && self.context.now_ms >= self.last_output_ms
                && self
                    .observation_gap
                    .as_ref()
                    .is_none_or(|gap| gap.available_from <= self.context.cursor)
                && valid_outcome,
        )
    }

    pub(crate) fn encode(&self, monitors: &TerminalMonitorSet) -> Result<Vec<u8>> {
        self.validate()?;
        self.validate_monitors(monitors)?;
        let facts = serde_json::to_vec(self).map_err(|_| TerminalSessionRecordError::Invalid)?;
        require(facts.len() <= MAX_FACT_BYTES)?;
        let monitor_bytes = monitors.snapshot()?;
        require(!monitor_bytes.is_empty() && monitor_bytes.len() <= MAX_MONITOR_BYTES)?;
        let mut bytes = Vec::with_capacity(HEADER_BYTES + facts.len() + monitor_bytes.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(
            &u32::try_from(facts.len())
                .map_err(|_| TerminalSessionRecordError::Invalid)?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&facts);
        bytes.extend_from_slice(&monitor_bytes);
        Ok(bytes)
    }

    /// Decode bounded facts before the potentially larger monitor set. The caller
    /// checks exact ownership before asking to restore monitor contents.
    pub(crate) fn decode<'a>(
        bytes: &'a [u8],
        session_id: &TerminalSessionId,
        source: &TerminalCursor,
    ) -> Result<(Self, &'a [u8])> {
        require(
            bytes.len() > HEADER_BYTES
                && bytes.len() <= HEADER_BYTES + MAX_FACT_BYTES + MAX_MONITOR_BYTES
                && bytes.starts_with(MAGIC),
        )?;
        let facts_len = u32::from_le_bytes(
            bytes[MAGIC.len()..HEADER_BYTES]
                .try_into()
                .map_err(|_| TerminalSessionRecordError::Invalid)?,
        ) as usize;
        require(
            facts_len > 0
                && facts_len <= MAX_FACT_BYTES
                && HEADER_BYTES + facts_len < bytes.len()
                && bytes.len() - HEADER_BYTES - facts_len <= MAX_MONITOR_BYTES,
        )?;
        let facts: Self = serde_json::from_slice(&bytes[HEADER_BYTES..HEADER_BYTES + facts_len])
            .map_err(|_| TerminalSessionRecordError::Invalid)?;
        facts.validate()?;
        require(&facts.session_id == session_id && &facts.context.cursor == source)?;
        Ok((facts, &bytes[HEADER_BYTES + facts_len..]))
    }

    pub(crate) fn restore_monitors(&self, bytes: &[u8]) -> Result<TerminalMonitorSet> {
        let monitors = TerminalMonitorSet::restore(bytes)?;
        self.validate_monitors(&monitors)?;
        Ok(monitors)
    }

    fn validate_monitors(&self, monitors: &TerminalMonitorSet) -> Result<()> {
        let context = monitors.context();
        require(
            monitors.session_id() == &self.session_id
                && context.cursor == self.context.cursor
                && context.now_ms == self.context.now_ms
                && context.lifecycle == self.context.lifecycle
                && (matches!(
                    context.lifecycle,
                    TerminalLifecycle::Starting | TerminalLifecycle::Running
                ) || monitors.len() == 0),
        )
    }
}

fn require(valid: bool) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(TerminalSessionRecordError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (TerminalSessionFacts, TerminalMonitorSet) {
        let id = TerminalSessionId::new("terminal-facts").unwrap();
        let context = TerminalMonitorContext {
            now_ms: 10,
            cursor: TerminalCursor::new(1, 5).unwrap(),
            lifecycle: TerminalLifecycle::Running,
        };
        let facts =
            TerminalSessionFacts::new(id.clone(), &owner("one"), context.clone(), 0, 5, None)
                .unwrap();
        (facts, TerminalMonitorSet::new(id, context).unwrap())
    }
    fn owner(incarnation: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("logical-owner").unwrap(),
            SessionIncarnationId::new(incarnation).unwrap(),
        )
    }

    #[test]
    fn facts_and_monitor_state_roundtrip_at_one_exact_cursor() {
        let (facts, monitors) = fixture();
        let bytes = facts.encode(&monitors).unwrap();
        let (decoded, monitor_bytes) =
            TerminalSessionFacts::decode(&bytes, &facts.session_id, &facts.context.cursor).unwrap();
        assert!(decoded.owned_by(&owner("one")));
        assert!(!decoded.owned_by(&owner("two")));
        let restored = decoded.restore_monitors(monitor_bytes).unwrap();
        assert_eq!(decoded.encode(&restored).unwrap(), bytes);
        assert_eq!(format!("{decoded:?}"), "TerminalSessionFacts { .. }");
    }

    #[test]
    fn frames_are_bounded_and_truncation_or_trailing_bytes_are_rejected() {
        let (facts, monitors) = fixture();
        let valid = facts.encode(&monitors).unwrap();
        for bytes in [
            &valid[..0],
            &valid[..8],
            &valid[..12],
            &valid[..valid.len() - 1],
        ] {
            let result =
                TerminalSessionFacts::decode(bytes, &facts.session_id, &facts.context.cursor)
                    .and_then(|(facts, rest)| facts.restore_monitors(rest));
            assert!(result.is_err());
        }
        let mut wrong_version = valid.clone();
        wrong_version[7] = 2;
        let mut oversized_facts = valid.clone();
        oversized_facts[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut trailing = valid.clone();
        trailing.push(0);
        for bytes in [wrong_version, oversized_facts, trailing] {
            let result =
                TerminalSessionFacts::decode(&bytes, &facts.session_id, &facts.context.cursor)
                    .and_then(|(facts, rest)| facts.restore_monitors(rest));
            assert!(result.is_err());
        }
    }

    #[test]
    fn mixed_session_cursor_and_monitor_context_are_rejected() {
        let (facts, mut monitors) = fixture();
        let bytes = facts.encode(&monitors).unwrap();
        assert!(
            TerminalSessionFacts::decode(
                &bytes,
                &TerminalSessionId::new("other").unwrap(),
                &facts.context.cursor,
            )
            .is_err()
        );
        assert!(
            TerminalSessionFacts::decode(
                &bytes,
                &facts.session_id,
                &TerminalCursor::new(1, 6).unwrap(),
            )
            .is_err()
        );
        let mut context = facts.context.clone();
        context.now_ms += 1;
        monitors.checkpoint_context(context).unwrap();
        assert!(facts.encode(&monitors).is_err());
        assert!(
            facts
                .restore_monitors(&monitors.snapshot().unwrap())
                .is_err()
        );
    }

    #[test]
    fn invalid_termination_and_time_facts_are_rejected() {
        let (mut facts, monitors) = fixture();
        facts.outcome = Some(TerminalProcessOutcome::Exited(0));
        assert!(facts.encode(&monitors).is_err());
        facts.outcome = None;
        facts.last_output_ms = 11;
        assert!(facts.encode(&monitors).is_err());
        facts.last_output_ms = 5;
        facts.created_at_ms = 6;
        assert!(facts.encode(&monitors).is_err());
        facts.created_at_ms = -1;
        assert!(facts.encode(&monitors).is_err());
    }

    #[test]
    fn unknown_fact_fields_cannot_smuggle_process_authority() {
        let (facts, monitors) = fixture();
        let mut value = serde_json::to_value(&facts).unwrap();
        value["pid"] = serde_json::json!(1234);
        let facts_bytes = serde_json::to_vec(&value).unwrap();
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&u32::try_from(facts_bytes.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&facts_bytes);
        bytes.extend_from_slice(&monitors.snapshot().unwrap());
        assert!(
            TerminalSessionFacts::decode(&bytes, &facts.session_id, &facts.context.cursor,)
                .is_err()
        );
    }
}
