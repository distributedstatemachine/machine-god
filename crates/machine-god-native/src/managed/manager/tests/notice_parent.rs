use super::*;
use crate::managed::{
    notices::{ManagedNotice, NoticeEvent, NoticePrincipal, NoticeTerminal},
    prompt_context::ParentNoticeContext,
};
use std::num::NonZeroU64;

fn settle(fixture: &mut Fixture) {
    fixture.drive(|f| {
        f.manager.active.is_none()
            && f.manager.replay.done
            && f.manager.retiring.is_empty()
            && f.manager.children.iter().all(|child| !child.busy())
    });
}

fn relationship(fixture: &mut Fixture, action: &str) {
    let mut command = serde_json::json!({"id":"child-2", "action":action});
    if action != "detach" {
        command["parent_id"] = "child-1".into();
    }
    assert!(
        fixture
            .command(serde_json::json!({"relationship":command}))
            .ok
    );
}

fn send(fixture: &mut Fixture, id: &str) {
    assert!(
        fixture
            .command(serde_json::json!({
                "message":{"send":{"id":id,"content":"explicit work"}}
            }))
            .ok
    );
}

fn paired(steps: Vec<ModelProviderStep>) -> Fixture {
    let mut fixture = Fixture::new(steps);
    fixture.manager.limits.residents = 2;
    for name in ["parent", "source"] {
        assert!(
            fixture
                .command(serde_json::json!({
                    "create":{"name":name,"mode":"persistent"}
                }))
                .ok
        );
    }
    relationship(&mut fixture, "reparent");
    settle(&mut fixture);
    fixture
}

fn evicted(steps: Vec<ModelProviderStep>) -> Fixture {
    let mut fixture = paired(steps);
    let selection = fixture.manager.children()[0].selection.clone();
    assert!(
        fixture
            .command(serde_json::json!({
                "create":{"name":"pressure","mode":"persistent"}
            }))
            .ok
    );
    settle(&mut fixture);
    // Exercise real resident pressure and cleanup, not a removed weak route.
    assert!(fixture.manager.selected_runtime(&selection).is_none());
    assert_eq!(fixture.manager.children.len(), 2);
    assert!(
        fixture
            .manager
            .children
            .iter()
            .all(|c| c.snapshot.head.id != "child-1")
    );
    let parent = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(parent.head.status, ManagedAgentState::Idle);
    assert_eq!(parent.head.generation, 1);
    fixture
}

fn originals(fixture: &Fixture) -> Vec<ManagedNotice> {
    let snapshot = block_on(fixture.journal.inspect("child-2".into())).unwrap();
    let mut cursor = None;
    let mut originals = Vec::new();
    loop {
        let page = block_on(fixture.journal.history(snapshot.clone(), cursor, 100)).unwrap();
        originals.extend(page.records.into_iter().filter_map(|record| match record {
            JournalRecord::Notice(notice) => Some(notice),
            _ => None,
        }));
        let Some(next) = page.next else {
            return originals;
        };
        cursor = Some(next);
    }
}

fn terminal(notice: &ManagedNotice, generation: u64, outcome: NoticeTerminal) -> bool {
    notice.target.parent.id == "child-1"
        && notice.target.parent.generation.get() == generation
        && notice.event == NoticeEvent::Terminal { outcome }
}

#[test]
fn absent_parent_terminal_is_durable_and_replays_after_restart() {
    let mut fixture = evicted(vec![completed()]);
    send(&mut fixture, "child-2");
    settle(&mut fixture);
    let original = originals(&fixture)
        .into_iter()
        .find(|notice| terminal(notice, 1, NoticeTerminal::Completed))
        .unwrap();
    assert!(
        fixture
            .manager
            .children
            .iter()
            .all(|c| c.snapshot.head.id != "child-1")
    );
    fixture.restart_manager();
    let session = fixture.child_session("child-1");
    let context = Arc::new(ParentNoticeContext::new(
        &session,
        original.target.parent.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&context).unwrap();
    settle(&mut fixture);
    let batch = fixture
        .manager
        .notices
        .snapshot(&original.target.parent, 64, 64 * 1024)
        .unwrap();
    assert!(
        batch
            .entries()
            .iter()
            .any(|entry| entry.notice() == &original)
    );
    assert_eq!(fixture.manager.notices.usage().trackers, 0);
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn parent_restored_before_child_terminal_keeps_original_target() {
    let mut fixture = evicted(vec![ModelProviderStep::pending(), completed()]);
    send(&mut fixture, "child-2");
    fixture.drive(|f| {
        f.factory.provider.requests().len() == 1
            && f.manager
                .children
                .iter()
                .any(|c| c.snapshot.head.id == "child-2" && c.notice.is_some())
    });
    // The running source cannot be evicted; restoring the parent evicts pressure.
    send(&mut fixture, "child-1");
    fixture.drive(|f| {
        f.factory.provider.requests().len() == 2
            && f.manager
                .children
                .iter()
                .any(|c| c.snapshot.head.id == "child-1" && !c.busy())
    });
    assert!(
        fixture
            .command(serde_json::json!({
                "lifecycle":{"id":"child-2","action":"cancel"}
            }))
            .ok
    );
    settle(&mut fixture);
    assert!(originals(&fixture).iter().any(|notice| terminal(
        notice,
        1,
        NoticeTerminal::Cancelled
    )));
    assert!(
        fixture
            .manager
            .children
            .iter()
            .any(|c| c.snapshot.head.id == "child-1" && c.snapshot.head.generation == 1)
    );
}

#[test]
fn reopened_parent_is_not_retargeted_without_explicit_relationship_change() {
    let mut fixture = paired(vec![completed(), completed(), completed()]);
    assert!(
        fixture
            .command(serde_json::json!({
                "lifecycle":{"id":"child-1","action":"close"}
            }))
            .ok
    );
    settle(&mut fixture);
    assert!(
        fixture
            .command(serde_json::json!({
                "lifecycle":{"id":"child-1","action":"reopen"}
            }))
            .ok
    );
    send(&mut fixture, "child-2");
    settle(&mut fixture);
    assert!(originals(&fixture).iter().any(|notice| terminal(
        notice,
        1,
        NoticeTerminal::Completed
    )));
    let target = NoticePrincipal {
        id: "child-1".into(),
        generation: NonZeroU64::new(2).unwrap(),
    };
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&target, 64, 64 * 1024)
            .unwrap()
            .entries()
            .is_empty()
    );
    relationship(&mut fixture, "reparent");
    send(&mut fixture, "child-2");
    settle(&mut fixture);
    let before_detach = originals(&fixture);
    assert!(
        before_detach
            .iter()
            .any(|notice| terminal(notice, 1, NoticeTerminal::Completed))
    );
    assert!(
        before_detach
            .iter()
            .any(|notice| terminal(notice, 2, NoticeTerminal::Completed))
    );
    relationship(&mut fixture, "detach");
    send(&mut fixture, "child-2");
    settle(&mut fixture);
    assert_eq!(originals(&fixture), before_detach);
    let head = block_on(fixture.journal.inspect("child-2".into()))
        .unwrap()
        .head;
    assert_eq!(
        (head.parent_id, head.parent_owner, head.parent_generation),
        (None, None, None)
    );
    assert_eq!(fixture.factory.provider.requests().len(), 3);
}
