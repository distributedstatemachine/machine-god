//! Bounded, data-only session facts and monitor state at one committed cursor.
//! These records never contain a PID, live input lease, or process capability.

use crate::terminal_monitor::{
    TerminalMonitorContext, TerminalMonitorError, TerminalMonitorSet, TerminalProcessOutcome,
};
use machine_god_core::{
    BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalAttentionState,
    TerminalBackend, TerminalCursor, TerminalGap, TerminalLifecycle, TerminalProfile,
    TerminalSessionId,
};
use serde::{Deserialize, Serialize};
use std::fmt;

const MAGIC: &[u8; 8] = b"MGTS\0\0\0\x01";
const HEADER_BYTES: usize = MAGIC.len() + 4;
// Includes worst-case JSON escaping of a 64 KiB command and bounded paths.
const MAX_FACT_BYTES: usize = 512 * 1024;
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

/// Display and catalog facts supplied by the trusted host at launch. Neither
/// identities nor paths are capabilities, and recovery never executes them.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalSessionMetadata {
    pub(crate) host_identity: SessionIncarnationId,
    pub(crate) backend_identity: String,
    pub(crate) shell: String,
    pub(crate) workspace: String,
    pub(crate) cwd: String,
    pub(crate) command: Option<String>,
    pub(crate) backend: TerminalBackend,
    pub(crate) profile: TerminalProfile,
}
impl fmt::Debug for TerminalSessionMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalSessionMetadata")
            .finish_non_exhaustive()
    }
}
impl TerminalSessionMetadata {
    pub(crate) fn validate(&self) -> Result<()> {
        let text = |value: &str, maximum| {
            !value.is_empty() && value.len() <= maximum && !value.contains('\0')
        };
        require(
            text(&self.backend_identity, 4096)
                && text(&self.shell, 4096)
                && self.shell.starts_with('/')
                && text(&self.workspace, 4096)
                && self.workspace.starts_with('/')
                && text(&self.cwd, 4096)
                && self.cwd.starts_with('/')
                && self
                    .command
                    .as_ref()
                    .is_none_or(|command| text(command, 64 * 1024)),
        )
    }
}

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
    #[serde(default)]
    pub(crate) monitor_notifications_incomplete: bool,
    /// None is explicit legacy metadata absence, never inferred launch data.
    #[serde(default)]
    pub(crate) metadata: Option<TerminalSessionMetadata>,
    #[serde(default)]
    pub(crate) attention: TerminalAttentionState,
}
impl fmt::Debug for TerminalSessionFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalSessionFacts")
            .finish_non_exhaustive()
    }
}
impl TerminalSessionFacts {
    pub(crate) const PREFIX_HEADER_BYTES: usize = HEADER_BYTES;

    /// Framing only, for a bounded unverified retention-selection hint. The
    /// caller reads the header first, then precisely this prefix length; no
    /// monitor payload needs to be read or allocated during enumeration.
    pub(crate) fn prefix_hint_len(header: &[u8], total_bytes: usize) -> Result<usize> {
        require(header.len() == HEADER_BYTES && header.starts_with(MAGIC))?;
        let facts_len = u32::from_le_bytes(
            header[MAGIC.len()..]
                .try_into()
                .map_err(|_| TerminalSessionRecordError::Invalid)?,
        ) as usize;
        require(facts_len > 0 && facts_len <= MAX_FACT_BYTES)?;
        let prefix_len = HEADER_BYTES + facts_len;
        require(prefix_len < total_bytes && total_bytes - prefix_len <= MAX_MONITOR_BYTES)?;
        Ok(prefix_len)
    }

    /// UNVERIFIED selection data, never authority to reclaim output. In
    /// particular, this neither authenticates the state blob nor validates its
    /// monitor suffix. The selected journal and full record must be verified
    /// again under writer authority before effects.
    pub(crate) fn decode_prefix_hint(
        prefix: &[u8],
        total_bytes: usize,
        session_id: &TerminalSessionId,
        source: &TerminalCursor,
    ) -> Result<Self> {
        let header = prefix
            .get(..HEADER_BYTES)
            .ok_or(TerminalSessionRecordError::Invalid)?;
        require(Self::prefix_hint_len(header, total_bytes)? == prefix.len())?;
        let facts: Self = serde_json::from_slice(&prefix[HEADER_BYTES..])
            .map_err(|_| TerminalSessionRecordError::Invalid)?;
        facts.validate()?;
        require(&facts.session_id == session_id && &facts.context.cursor == source)?;
        Ok(facts)
    }

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
            monitor_notifications_incomplete: false,
            metadata: None,
            attention: TerminalAttentionState::default(),
        };
        facts.validate()?;
        Ok(facts)
    }

    pub(crate) fn owned_by(&self, owner: &BackgroundOutputOwner) -> bool {
        &self.owner_session_id == owner.session_id()
            && &self.owner_incarnation_id == owner.session_incarnation_id()
    }

    /// Bind data-only retention facts to their actual profile namespace. This
    /// reconstructs identity, never native ownership or a recovered session.
    /// Legacy facts without launch metadata cannot authorize cross-owner work.
    pub(crate) fn validate_profile_binding(&self, namespace: &str) -> Result<()> {
        let metadata = self
            .metadata
            .as_ref()
            .ok_or(TerminalSessionRecordError::Invalid)?;
        self.validate()?;
        require(crate::terminal_catalog::canonical_workspace(
            &metadata.workspace,
        ))?;
        let owner = BackgroundOutputOwner::new(
            self.owner_session_id.clone(),
            self.owner_incarnation_id.clone(),
        );
        require(crate::terminal_catalog::owner_name(&metadata.workspace, &owner) == namespace)
    }

    fn validate(&self) -> Result<()> {
        if let Some(metadata) = &self.metadata {
            metadata.validate()?;
        }
        self.attention
            .validate()
            .map_err(|_| TerminalSessionRecordError::Invalid)?;
        require(
            matches!(
                self.context.lifecycle,
                TerminalLifecycle::Starting | TerminalLifecycle::Running
            ) || self.attention == TerminalAttentionState::default(),
        )?;
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
        let header = bytes
            .get(..HEADER_BYTES)
            .ok_or(TerminalSessionRecordError::Invalid)?;
        let prefix_len = Self::prefix_hint_len(header, bytes.len())?;
        let facts =
            Self::decode_prefix_hint(&bytes[..prefix_len], bytes.len(), session_id, source)?;
        Ok((facts, &bytes[prefix_len..]))
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
pub(crate) fn test_metadata() -> TerminalSessionMetadata {
    TerminalSessionMetadata {
        host_identity: SessionIncarnationId::new("host-test").unwrap(),
        backend_identity: "backend-test".into(),
        shell: "/bin/bash".into(),
        workspace: "/workspace".into(),
        cwd: "/workspace".into(),
        command: None,
        backend: TerminalBackend::Native,
        profile: TerminalProfile::Clean,
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
    fn prefix_hint_shares_exact_framing_without_decoding_monitor_payload() {
        let (facts, monitors) = fixture();
        let mut bytes = facts.encode(&monitors).unwrap();
        let prefix_len =
            TerminalSessionFacts::prefix_hint_len(&bytes[..HEADER_BYTES], bytes.len()).unwrap();
        let hint = TerminalSessionFacts::decode_prefix_hint(
            &bytes[..prefix_len],
            bytes.len(),
            &facts.session_id,
            &facts.context.cursor,
        )
        .unwrap();
        assert_eq!(hint.created_at_ms, facts.created_at_ms);
        assert!(
            TerminalSessionFacts::decode_prefix_hint(
                &bytes[..prefix_len - 1],
                bytes.len(),
                &facts.session_id,
                &facts.context.cursor,
            )
            .is_err()
        );
        assert!(
            TerminalSessionFacts::decode_prefix_hint(
                &bytes[..prefix_len],
                bytes.len(),
                &TerminalSessionId::new("other").unwrap(),
                &facts.context.cursor,
            )
            .is_err()
        );
        bytes[prefix_len] = b'!';
        let (decoded, suffix) =
            TerminalSessionFacts::decode(&bytes, &facts.session_id, &facts.context.cursor).unwrap();
        assert!(decoded.restore_monitors(suffix).is_err());
        for length in [0, MAX_FACT_BYTES + 1] {
            let mut header = bytes[..HEADER_BYTES].to_vec();
            header[MAGIC.len()..].copy_from_slice(&u32::try_from(length).unwrap().to_le_bytes());
            assert!(TerminalSessionFacts::prefix_hint_len(&header, bytes.len()).is_err());
        }
        for total in [
            prefix_len,
            prefix_len - 1,
            prefix_len + MAX_MONITOR_BYTES + 1,
        ] {
            assert!(TerminalSessionFacts::prefix_hint_len(&bytes[..HEADER_BYTES], total).is_err());
        }
        assert!(
            TerminalSessionFacts::prefix_hint_len(&bytes[..HEADER_BYTES - 1], bytes.len()).is_err()
        );
    }

    #[test]
    fn profile_binding_requires_exact_namespace_and_canonical_launch_workspace() {
        let (mut facts, _) = fixture();
        let namespace = crate::terminal_catalog::owner_name("/workspace", &owner("one"));
        assert_eq!(
            facts.validate_profile_binding(&namespace),
            Err(TerminalSessionRecordError::Invalid)
        );
        facts.metadata = Some(test_metadata());
        facts.validate_profile_binding(&namespace).unwrap();
        let wrong_owner = crate::terminal_catalog::owner_name("/workspace", &owner("other"));
        assert_eq!(
            facts.validate_profile_binding(&wrong_owner),
            Err(TerminalSessionRecordError::Invalid)
        );
        facts.metadata.as_mut().unwrap().workspace = "/workspace/../other".into();
        assert_eq!(
            facts.validate_profile_binding(&namespace),
            Err(TerminalSessionRecordError::Invalid)
        );
    }

    #[test]
    fn launch_metadata_roundtrips_at_maximum_escaped_bounds_without_debug_secrets() {
        let (mut facts, monitors) = fixture();
        let mut metadata = test_metadata();
        metadata.backend_identity = "\u{1}".repeat(4096);
        metadata.shell = format!("/{}", "\u{1}".repeat(4095));
        metadata.workspace.clone_from(&metadata.shell);
        metadata.cwd.clone_from(&metadata.shell);
        metadata.command = Some("\u{1}".repeat(64 * 1024));
        facts.metadata = Some(metadata);
        let bytes = facts.encode(&monitors).unwrap();
        let (decoded, monitor_bytes) =
            TerminalSessionFacts::decode(&bytes, &facts.session_id, &facts.context.cursor).unwrap();
        assert_eq!(
            decoded
                .encode(&decoded.restore_monitors(monitor_bytes).unwrap())
                .unwrap(),
            bytes
        );
        assert_eq!(
            decoded
                .metadata
                .as_ref()
                .unwrap()
                .command
                .as_ref()
                .unwrap()
                .len(),
            64 * 1024
        );
        assert_eq!(
            format!("{:?}", decoded.metadata.unwrap()),
            "TerminalSessionMetadata { .. }"
        );
    }

    fn reframe(facts: &serde_json::Value, monitors: &TerminalMonitorSet) -> Vec<u8> {
        let facts = serde_json::to_vec(facts).unwrap();
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&u32::try_from(facts.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&facts);
        bytes.extend_from_slice(&monitors.snapshot().unwrap());
        bytes
    }

    #[test]
    fn malformed_metadata_is_rejected_and_legacy_absence_is_explicit() {
        let (mut facts, monitors) = fixture();
        facts.metadata = Some(test_metadata());
        let wire = serde_json::to_value(&facts).unwrap();
        for (field, value) in [
            ("shell", serde_json::json!("relative")),
            ("workspace", serde_json::json!("relative")),
            ("cwd", serde_json::json!("/bad\0path")),
            ("command", serde_json::json!("")),
            ("command", serde_json::json!("x".repeat(64 * 1024 + 1))),
            ("backend_identity", serde_json::json!("x".repeat(4097))),
            ("backend", serde_json::json!("unknown")),
            ("pid", serde_json::json!(123)),
        ] {
            let mut candidate = wire.clone();
            candidate["metadata"][field] = value;
            assert!(
                TerminalSessionFacts::decode(
                    &reframe(&candidate, &monitors),
                    &facts.session_id,
                    &facts.context.cursor
                )
                .is_err(),
                "{field}"
            );
        }
        let mut legacy = wire;
        legacy.as_object_mut().unwrap().remove("metadata");
        legacy.as_object_mut().unwrap().remove("attention");
        let bytes = reframe(&legacy, &monitors);
        let (decoded, _) =
            TerminalSessionFacts::decode(&bytes, &facts.session_id, &facts.context.cursor).unwrap();
        assert!(decoded.metadata.is_none());
        assert_eq!(decoded.attention, TerminalAttentionState::default());
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
