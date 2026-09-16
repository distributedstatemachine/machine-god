use super::*;
use machine_god_core::{ManagedEvent, ManagedEventKind, ManagedReceipt};

fn receipt(result: machine_god_core::ManagedSubagentResult) -> ManagedReceipt {
    assert!(result.ok, "{result:?}");
    let Some(ManagedRequested::Receipt(receipt)) = result.requested else {
        panic!("mutation receipt required");
    };
    receipt
}

fn events(fixture: &mut Fixture) -> Vec<ManagedEvent> {
    let mut cursor = None;
    let mut events = Vec::new();
    for _ in 0..100 {
        let mut request = serde_json::json!({"id":"child-1","sections":["events"],"limit":1});
        if let Some(cursor) = cursor {
            request["cursor"] = serde_json::Value::String(cursor);
        }
        let result = fixture.command(serde_json::json!({"inspect":request}));
        assert!(result.ok, "{result:?}");
        let Some(ManagedRequested::Inspection(page)) = result.requested else {
            panic!("inspection required");
        };
        assert!(!page.restart_required);
        assert!(page.events.len() <= 1);
        events.extend(page.events);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return events;
        }
    }
    panic!("bounded history failed to terminate");
}

#[test]
#[allow(clippy::too_many_lines)] // One real command history across paging, archive and restart.
fn real_create_message_and_lifecycle_receipts_resolve_to_paged_events() {
    let mut fixture = Fixture::new(vec![super::completed()]);
    let created = receipt(
        fixture.command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}})),
    );
    let initial = events(&mut fixture);
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].sequence, created.event_sequence);
    assert_eq!(initial[0].kind, ManagedEventKind::Created);

    let queued = receipt(
        fixture.command(serde_json::json!({"message":{"send":{"id":"child-1","content":"first"}}})),
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Idle
            && !f.manager.children[0].busy()
    });
    let configured = receipt(
        fixture.command(serde_json::json!({"configure":{"id":"child-1","name":"renamed"}})),
    );
    let detached = receipt(
        fixture.command(serde_json::json!({"relationship":{"id":"child-1","action":"detach"}})),
    );
    let cancelled = receipt(
        fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"cancel"}})),
    );
    let closed = receipt(
        fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"close"}})),
    );
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.is_empty());
    let archived = events(&mut fixture);
    let kind = |receipt: &ManagedReceipt| {
        &archived
            .iter()
            .find(|event| event.sequence == receipt.event_sequence)
            .unwrap()
            .kind
    };
    assert!(matches!(
        kind(&queued),
        ManagedEventKind::MessageQueued { .. }
    ));
    assert_eq!(kind(&configured), &ManagedEventKind::Configured);
    assert!(matches!(
        kind(&detached),
        ManagedEventKind::RelationshipChanged {
            parent_id: None,
            ..
        }
    ));
    assert!(matches!(
        kind(&cancelled),
        ManagedEventKind::LifecycleChanged {
            current: ManagedAgentState::Idle,
            ..
        }
    ));
    assert!(matches!(
        kind(&closed),
        ManagedEventKind::LifecycleChanged {
            current: ManagedAgentState::Archived,
            ..
        }
    ));
    assert!(archived.iter().any(|event| matches!(
        event.kind,
        ManagedEventKind::WorkTransition {
            current: ManagedQueueStatus::Running,
            ..
        }
    )));
    assert!(archived.iter().any(|event| matches!(
        event.kind,
        ManagedEventKind::WorkTransition {
            current: ManagedQueueStatus::Completed,
            ..
        }
    )));
    assert!(
        archived
            .windows(2)
            .all(|pair| pair[0].sequence > pair[1].sequence)
    );
    fixture.restart_manager();
    assert_eq!(events(&mut fixture), archived);
    let reopened = receipt(
        fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"reopen"}})),
    );
    let retained = events(&mut fixture);
    assert_eq!(retained[0].sequence, reopened.event_sequence);
    assert!(matches!(
        retained[0].kind,
        ManagedEventKind::LifecycleChanged {
            previous: ManagedAgentState::Archived,
            current: ManagedAgentState::Idle
        }
    ));
    assert_eq!(&retained[1..], archived.as_slice());
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn event_cursor_rejects_a_changed_head_instead_of_skipping_a_new_mutation() {
    let mut fixture = Fixture::new(vec![]);
    receipt(fixture.command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}})));
    let first = fixture
        .command(serde_json::json!({"inspect":{"id":"child-1","sections":["events"],"limit":1}}));
    let cursor = first
        .cursor
        .expect("control record remains on the next page");
    receipt(fixture.command(serde_json::json!({"configure":{"id":"child-1","name":"changed"}})));
    let next = fixture.command(serde_json::json!({"inspect":{"id":"child-1","sections":["events"],"limit":1,"cursor":cursor}}));
    let Some(ManagedRequested::Inspection(page)) = next.requested else {
        panic!("inspection required");
    };
    assert!(page.restart_required);
    assert!(page.events.is_empty());
    assert_eq!(events(&mut fixture).len(), 2);
}
