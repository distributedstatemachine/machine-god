//! Accepted large originals remain traversable by every history consumer.
use super::{Fixture, completed, delivery};
use crate::NativeConversation;
use crate::managed::{
    manager::ManagedForegroundSelection, notices::NoticePrincipal,
    prompt_context::ParentNoticeContext, store::JournalRecord,
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    CancellationToken, ManagedEventKind, ManagedRequested, ManagedSubagentCommand, Session,
};
use std::{num::NonZeroU64, sync::Arc, task::Context, task::Poll, task::Waker};

fn enroll(fixture: &mut Fixture, session: &Session) -> ManagedForegroundSelection {
    let prepared = fixture.notified_foreground(session.clone());
    let reservation = fixture.manager.reserve_foreground().unwrap();
    fixture.drive(|f| {
        f.manager
            .poll_foreground_reservation(&reservation, &Context::from_waker(Waker::noop()))
            .is_ready()
    });
    fixture
        .manager
        .enroll_foreground(Box::new(prepared), &reservation)
        .unwrap()
}

fn accept_escaped_message(fixture: &mut Fixture, parent: &ManagedForegroundSelection) {
    let content = "\u{1}".repeat(65_536);
    let reference: crate::NativeSkillReference = serde_json::from_value(serde_json::json!({
        "version": 1,
        "name": "s",
        "location": format!("/{}", "\u{1}".repeat(4090)),
        "revision": ([0; 32]),
    }))
    .unwrap();
    let references = vec![reference; 16];
    let command = ManagedSubagentCommand::decode(serde_json::json!({
        "command": {"message": {"send": {"id": "child-1", "content": content}}}
    }))
    .unwrap();
    // This is the same bounded human mailbox endpoint used by the public
    // interactive skill-reference submission, not a manufactured journal record.
    let mut response = fixture
        .manager
        .request_observed_human_command(
            parent,
            None,
            command,
            &references,
            CancellationToken::new(),
        )
        .unwrap();
    let result = block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    assert!(result.ok, "{result:?}");
    // The fixture has no skill catalog. Rebinding fails the accepted head
    // without a provider call, but cannot remove its already-durable original.
    fixture.drive(|f| f.manager.active.is_none() && !f.manager.children[0].busy());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let work = block_on(
        fixture
            .journal
            .read_work(snapshot.head.queue[0].page.clone()),
    )
    .unwrap();
    assert_eq!(work.content, content);
    assert_eq!(work.skills, references);
    assert!(
        serde_json::to_vec(&JournalRecord::WorkAccepted(work))
            .unwrap()
            .len()
            > 512 * 1024
    );
    assert!(fixture.factory.provider.requests().is_empty());
}

fn inspect_through_original(fixture: &mut Fixture) {
    let mut cursor = None;
    let mut created = false;
    let mut complete = false;
    for _ in 0..64 {
        let mut query = serde_json::json!({
            "id": "child-1", "sections": ["messages", "events", "tool_activity"], "limit": 100
        });
        if let Some(cursor) = cursor.take() {
            query["cursor"] = serde_json::Value::String(cursor);
        }
        let result = fixture.command(serde_json::json!({"inspect": query}));
        assert!(result.ok, "{result:?}");
        result.validate().unwrap();
        assert!(
            serde_json::to_vec(&result).unwrap().len()
                <= machine_god_core::MAX_SUBAGENT_OUTPUT_BYTES
        );
        let Some(ManagedRequested::Inspection(inspection)) = result.requested else {
            panic!("inspection receipt");
        };
        assert!(inspection.history_error.is_none());
        assert!(inspection.events_error.is_none());
        assert!(inspection.tool_activity_error.is_none());
        assert!(!inspection.restart_required);
        created |= inspection
            .events
            .iter()
            .any(|event| event.kind == ManagedEventKind::Created);
        cursor = inspection.next_cursor;
        if cursor.is_none() {
            complete = true;
            break;
        }
    }
    assert!(
        complete && created,
        "inspection did not reach older evidence"
    );
}

#[test]
fn oversized_accepted_work_preserves_inspection_replay_and_source_acknowledgement() {
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    // Retain an original before the large accepted record. Both replay and
    // delivery must cross that record to prove this older notice and lineage.
    let original = delivery::original(&mut fixture, &parent);
    let selection = enroll(&mut fixture, &parent);
    accept_escaped_message(&mut fixture, &selection);
    inspect_through_original(&mut fixture);
    fixture.restart_manager();
    let target = NoticePrincipal {
        id: parent.id().to_string(),
        generation: NonZeroU64::new(1).unwrap(),
    };
    let context = Arc::new(ParentNoticeContext::new(
        &parent,
        target.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&target, 64, 64 * 1024)
            .unwrap()
            .entries()
            .iter()
            .any(|entry| entry.notice() == &original)
    );
    assert!(fixture.factory.provider.requests().is_empty());
    let conversation = NativeConversation::from_session(parent)
        .unwrap()
        .with_notice_context(&context)
        .unwrap();
    let turn = block_on(conversation.prompt("explicit parent prompt".into(), 102)).unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let delivered = context.delivery().unwrap();
    assert!(delivered.originals().contains(&original));
    fixture.drive(|f| {
        f.manager.active.is_none()
            && f.manager
                .parents
                .iter()
                .any(|parent| parent.completed.is_some())
    });
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let newest = block_on(fixture.journal.history(snapshot, None, 100)).unwrap();
    assert!(newest.records.contains(&JournalRecord::NoticeAcknowledged {
        identity: original.identity(),
        target: original.target,
        checkpoint: delivered.checkpoint().clone(),
    }));
    block_on(conversation.clear_notice_delivery(&delivered)).unwrap();
    assert!(context.delivery().is_none());
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}
