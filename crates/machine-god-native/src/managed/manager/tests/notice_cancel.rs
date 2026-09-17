use super::*;
use crate::managed::notices::{ManagedNotice, NoticeEvent, NoticeTerminal};

fn originals(fixture: &mut Fixture) -> Vec<ManagedNotice> {
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let mut cursor = None;
    let mut notices = Vec::new();
    loop {
        let page = block_on(fixture.journal.history(snapshot.clone(), cursor, 100)).unwrap();
        notices.extend(page.records.into_iter().filter_map(|record| match record {
            JournalRecord::Notice(notice) => Some(notice),
            _ => None,
        }));
        let Some(next) = page.next else {
            return notices;
        };
        cursor = Some(next);
    }
}

fn cancellation(notices: &[ManagedNotice]) -> &ManagedNotice {
    let selected: Vec<_> = notices
        .iter()
        .filter(|notice| {
            notice.event
                == NoticeEvent::Terminal {
                    outcome: NoticeTerminal::Cancelled,
                }
        })
        .collect();
    assert_eq!(
        selected.len(),
        1,
        "accepted cancellation needs one original notice"
    );
    selected[0]
}

fn queued_commands(fixture: &mut Fixture, commands: Vec<serde_json::Value>) {
    let mut admissions = Vec::new();
    let mut responses = Vec::new();
    for command in commands {
        let (admission, invocation) = fixture.invocation(command);
        admissions.push(admission);
        let requester = fixture.requester.clone();
        let future: machine_god_core::BoxFuture<'static, _> = Box::pin(async move {
            requester
                .execute(invocation, CancellationToken::new())
                .await
        });
        responses.push(Some(future));
    }
    let mut cx = Context::from_waker(Waker::noop());
    for response in &mut responses {
        assert!(
            response
                .as_mut()
                .unwrap()
                .as_mut()
                .poll(&mut cx)
                .is_pending()
        );
    }
    block_on(std::future::poll_fn(|cx| {
        for response in &mut responses {
            if let Some(future) = response
                && let Poll::Ready(result) = future.as_mut().poll(cx)
            {
                assert!(result.unwrap().ok);
                *response = None;
            }
        }
        if responses.iter().all(Option::is_none) {
            return Poll::Ready(());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    drop(admissions);
}

fn cancel() -> serde_json::Value {
    serde_json::json!({"lifecycle": {"id": "child-1", "action": "cancel"}})
}

#[test]
fn cancellation_policy_is_frozen_and_detached_work_stays_silent() {
    for detached in [false, true] {
        let mut fixture = Fixture::new(vec![]);
        let mut commands = vec![serde_json::json!({"create": {
            "name": "silent", "mode": "persistent", "prompt": "head",
            "notifications": {"terminal": {"cancelled": detached}}
        }})];
        if detached {
            commands.push(serde_json::json!({"relationship": {
                "id": "child-1", "action": "detach"
            }}));
        } else {
            commands.push(serde_json::json!({"configure": {
                "id": "child-1", "notifications": {"terminal": {"cancelled": true}}
            }}));
        }
        commands.push(cancel());
        queued_commands(&mut fixture, commands);
        assert!(originals(&mut fixture).is_empty());
        assert!(fixture.factory.provider.requests().is_empty());
        assert!(fixture.manager.children[0].snapshot.head.queue.is_empty());
        assert_eq!(
            fixture.manager.children[0].snapshot.head.status,
            ManagedAgentState::Idle
        );
    }
}

#[test]
fn cancellation_preserves_later_fifo_work_and_does_not_repeat_on_idle_cancel() {
    let mut fixture = Fixture::new(vec![]);
    queued_commands(
        &mut fixture,
        vec![
            serde_json::json!({"create": {
                "name": "fifo", "mode": "persistent", "prompt": "first"
            }}),
            serde_json::json!({"message": {"send": {"id": "child-1", "content": "second"}}}),
            cancel(),
        ],
    );
    let first_notices = originals(&mut fixture);
    assert_eq!(cancellation(&first_notices).source.work_id, "work-1");
    let first = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(first.head.queue.len(), 1);
    assert_eq!(first.head.queue[0].status, ManagedQueueStatus::Interrupted);
    let next_id = first.head.queue[0].id.clone();
    assert_eq!(
        block_on(fixture.journal.read_work(first.head.queue[0].page.clone()))
            .unwrap()
            .content,
        "second"
    );
    assert!(fixture.command(cancel()).ok);
    let two = originals(&mut fixture);
    assert_eq!(two.len(), 2);
    assert!(two.iter().any(|notice| notice.source.work_id == next_id));
    assert!(two.contains(cancellation(&first_notices)));
    assert!(fixture.command(cancel()).ok);
    assert_eq!(originals(&mut fixture), two);
    assert!(fixture.factory.provider.requests().is_empty());
}

#[test]
fn interrupted_started_work_cancellation_keeps_actual_attempt_and_replays_once() {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending()]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "started", "mode": "one_off", "prompt": "work",
                "notifications": {"started": true}
            }}))
            .ok
    );
    fixture.drive(|f| {
        f.factory.provider.requests().len() == 1 && !f.manager.children[0].notice_started
    });
    let before = originals(&mut fixture);
    let started = before
        .iter()
        .find(|notice| notice.event == NoticeEvent::Started)
        .unwrap()
        .clone();
    fixture.restart_manager();
    let preparations = fixture.factory.prepared.load(Ordering::Relaxed);
    assert!(fixture.command(cancel()).ok);
    let notices = originals(&mut fixture);
    let cancelled = cancellation(&notices);
    assert_eq!(cancelled.source, started.source);
    assert_eq!(cancelled.target.parent, started.target.parent);
    assert_eq!(
        cancelled.target.parent_incarnation,
        started.target.parent_incarnation
    );
    assert_eq!(
        fixture.factory.prepared.load(Ordering::Relaxed),
        preparations
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    assert!(fixture.command(cancel()).ok);
    assert_eq!(originals(&mut fixture), notices);
    fixture.restart_manager();
    assert_eq!(originals(&mut fixture), notices);
}

#[test]
fn active_cancel_publishes_only_one_terminal_for_the_original_attempt() {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending()]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "running", "mode": "persistent", "prompt": "work",
                "notifications": {"started": true}
            }}))
            .ok
    );
    fixture.drive(|f| {
        f.factory.provider.requests().len() == 1 && !f.manager.children[0].notice_started
    });
    let before = originals(&mut fixture);
    let source = before
        .iter()
        .find(|notice| notice.event == NoticeEvent::Started)
        .unwrap()
        .source
        .clone();
    assert!(fixture.command(cancel()).ok);
    let notices = originals(&mut fixture);
    assert_eq!(cancellation(&notices).source, source);
    assert_eq!(notices.len(), 2);
    assert!(fixture.command(cancel()).ok);
    fixture.restart_manager();
    assert_eq!(originals(&mut fixture), notices);
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn active_close_archives_without_a_cancellation_notice() {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending()]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "closing", "mode": "persistent", "prompt": "work"
            }}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle": {
                "id": "child-1", "action": "close"
            }}))
            .ok
    );
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.is_empty());
    assert!(originals(&mut fixture).is_empty());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(snapshot.head.status, ManagedAgentState::Archived);
    let parent = crate::managed::notices::NoticePrincipal {
        id: "parent".into(),
        generation: std::num::NonZeroU64::new(1).unwrap(),
    };
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&parent, 64, 64 * 1024)
            .unwrap()
            .entries()
            .is_empty()
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn cancellation_of_explicit_retry_uses_its_new_attempt_not_the_failed_attempt() {
    for started in [false, true] {
        let mut fixture = Fixture::new(vec![
            ModelProviderStep::StartError(machine_god_core::ProviderError::new(
                machine_god_core::ProviderErrorKind::Protocol,
                "fixture",
                "failed attempt",
                false,
            )),
            ModelProviderStep::pending(),
        ]);
        assert!(
            fixture
                .command(serde_json::json!({"create": {
                    "name": "retry", "mode": "persistent", "prompt": "original work"
                }}))
                .ok
        );
        fixture.drive(|f| {
            f.manager.children[0].snapshot.head.status == ManagedAgentState::Failed
                && !f.manager.children[0].busy()
        });
        let before = originals(&mut fixture);
        assert_eq!(before.len(), 1);
        assert_eq!(
            before[0].event,
            NoticeEvent::Terminal {
                outcome: NoticeTerminal::Failed
            }
        );
        let resume = serde_json::json!({"lifecycle": {"id": "child-1", "action": "resume"}});
        if started {
            assert!(fixture.command(resume).ok);
            fixture.drive(|f| f.factory.provider.requests().len() == 2);
            assert!(fixture.command(cancel()).ok);
        } else {
            queued_commands(&mut fixture, vec![resume, cancel()]);
        }
        let notices = originals(&mut fixture);
        let cancelled = cancellation(&notices);
        assert_eq!(notices.len(), 2);
        assert!(notices.contains(&before[0]));
        assert_eq!(cancelled.source.work_id, before[0].source.work_id);
        assert!(cancelled.source.work_generation > before[0].source.work_generation);
        assert_eq!(
            fixture.factory.provider.requests().len(),
            if started { 2 } else { 1 }
        );
        fixture.restart_manager();
        assert_eq!(originals(&mut fixture), notices);
    }
}

#[test]
fn nonresident_cancel_after_shutdown_preserves_frozen_enabled_notice() {
    let mut fixture = Fixture::new(vec![]);
    // Stop at the real create receipt, before its accepted work can execute.
    fixture.manager.limits.work_per_poll = 1;
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "interrupted", "mode": "persistent", "prompt": "accepted work"
            }}))
            .ok
    );
    assert!(fixture.factory.provider.requests().is_empty());
    fixture.restart_manager();
    assert!(fixture.manager.children.is_empty());
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    let before = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(before.head.queue[0].status, ManagedQueueStatus::Interrupted);
    // Configuration must not rewrite the already accepted work's policy.
    assert!(
        fixture
            .command(serde_json::json!({"configure": {
                "id": "child-1", "notifications": {"terminal": {"cancelled": false}}
            }}))
            .ok
    );
    let prepared = fixture.factory.prepared.load(Ordering::Relaxed);
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle": {
                "id": "child-1", "action": "cancel"
            }}))
            .ok
    );
    let notices = originals(&mut fixture);
    let notice = cancellation(&notices);
    assert_eq!(notice.source.work_id, before.head.queue[0].id);
    assert_eq!(
        notice.source.source.generation.get(),
        before.head.generation
    );
    assert_eq!(notice.target.parent.id, "parent");
    assert_eq!(notice.target.parent.generation.get(), 1);
    assert_eq!(notice.target.parent_incarnation.as_str(), "incarnation");
    assert_eq!(fixture.factory.prepared.load(Ordering::Relaxed), prepared);
    assert!(fixture.factory.provider.requests().is_empty());
    let after = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(after.head.status, ManagedAgentState::Idle);
    assert!(after.head.queue.is_empty());
    fixture.restart_manager();
    assert_eq!(originals(&mut fixture), notices);
}

#[test]
fn cancellation_before_initial_execution_publishes_original_without_a_turn() {
    let mut fixture = Fixture::new(vec![]);
    // Both actual admitted invocations enter the mailbox before manager polling.
    // This exercises cancellation of a resident accepted head before Start.
    let (_create_admission, create) = fixture.invocation(serde_json::json!({"create": {
        "name": "queued", "mode": "one_off", "prompt": "never executed"
    }}));
    let (_cancel_admission, cancel) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "cancel"
    }}));
    let requester = fixture.requester.clone();
    let mut create = requester.execute(create, CancellationToken::new());
    let mut cancel = requester.execute(cancel, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(create.as_mut().poll(&mut cx).is_pending());
    assert!(cancel.as_mut().poll(&mut cx).is_pending());
    let mut created = false;
    block_on(std::future::poll_fn(|cx| {
        if !created && let Poll::Ready(result) = create.as_mut().poll(cx) {
            assert!(result.unwrap().ok);
            created = true;
        }
        if let Poll::Ready(result) = cancel.as_mut().poll(cx) {
            assert!(result.unwrap().ok);
            assert!(created);
            return Poll::Ready(());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    let notices = originals(&mut fixture);
    let notice = cancellation(&notices);
    assert_eq!(notice.source.work_id, "work-1");
    assert_eq!(notice.target.parent.id, "parent");
    assert_eq!(
        fixture.manager.children[0].snapshot.head.status,
        ManagedAgentState::Cancelled
    );
    assert!(fixture.factory.provider.requests().is_empty());
    assert!(fixture.manager.children[0].prepared.owner.run().is_none());
}
