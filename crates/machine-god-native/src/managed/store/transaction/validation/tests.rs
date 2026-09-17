use super::*;
use crate::managed::store::records::JournalControl;
use machine_god_core::{ManagedAgentState, SessionId, SessionIncarnationId};

#[test]
fn historical_control_requires_complete_parent_identity() {
    let owner = JournalTranscript {
        session_id: SessionId::new("parent").unwrap(),
        incarnation: SessionIncarnationId::new("incarnation").unwrap(),
    };
    for mask in 0..8 {
        let control = JournalControl {
            revision: 1,
            status: ManagedAgentState::Idle,
            intent: None,
            failure: None,
            controller: owner.clone(),
            parent_id: (mask & 1 != 0).then(|| "parent".into()),
            parent_owner: (mask & 2 != 0).then(|| owner.clone()),
            parent_generation: (mask & 4 != 0).then_some(1),
            notice_cursor: 0,
        };
        assert_eq!(
            records(&[JournalRecord::Control(control)]).is_ok(),
            mask == 0 || mask == 7
        );
    }
    assert!(relationship(Some("parent"), Some(&owner), Some(0)).is_err());
    assert!(relationship(Some("other"), Some(&owner), Some(1)).is_err());
}
