use super::super::store;
use super::*;
mod catalog;
mod delivery;
mod events;
#[cfg(feature = "ai-gateway-http")]
mod execution_consent;
mod fixture;
mod foreground;
mod limits;
mod notice_deadline;
mod notice_parent;
mod observation;
mod relationship;
use fixture::Fixture;
use futures_executor::block_on;
use machine_god_core::{ManagedFailureCode, ManagedRequested, ManagedSubagentAuthority};
use machine_god_core::{ManagedResultStatus, ModelEvent, StopReason};
use machine_god_testkit::ModelProviderStep;
use std::sync::atomic::Ordering;
use std::task::Waker;

#[test]
fn text_projection_preserves_utf8_boundaries() {
    assert_eq!(projection::prefix("a🦀b", 4), ("a".into(), true));
    assert_eq!(projection::prefix("a🦀b", 5), ("a🦀".into(), true));
}

#[test]
#[allow(clippy::too_many_lines)] // One original envelope across foreign, reparented and restarted targets.
fn journal_replay_preserves_originals_and_excludes_confirmed_source_acks() {
    use super::super::{
        notices::{
            ManagedNotice, NoticeEvent, NoticePrincipal, NoticeTarget, NoticeTerminal,
            WorkNoticeIdentity,
        },
        prompt_context::{NoticeCheckpoint, ParentNoticeContext},
    };
    use std::num::NonZeroU64;
    let nz = |value| NonZeroU64::new(value).unwrap();
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}}))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let session = fixture.notice_session();
    let target = NoticePrincipal {
        id: "notice-parent".into(),
        generation: nz(1),
    };
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let store::JournalPublication::Confirmed(snapshot) = block_on(fixture.journal.mutate(
        snapshot,
        JournalMutation::Relationship {
            parent_id: Some(target.id.clone()),
            parent_owner: Some(store::JournalTranscript {
                session_id: session.id(),
                incarnation: session.incarnation_id(),
            }),
            parent_generation: Some(1),
        },
    ))
    .unwrap() else {
        panic!("confirmed parent binding");
    };
    let snapshot = *snapshot;
    let original = ManagedNotice {
        source: WorkNoticeIdentity {
            source: NoticePrincipal {
                id: "child-1".into(),
                generation: nz(1),
            },
            work_id: "original-work".into(),
            work_generation: nz(1),
        },
        source_sequence: nz(snapshot.head.next_sequence),
        target: NoticeTarget {
            parent_incarnation: session.incarnation_id(),
            parent: target.clone(),
            relationship_generation: nz(snapshot.head.revision),
        },
        event: NoticeEvent::Terminal {
            outcome: NoticeTerminal::Completed,
        },
        history: None,
    };
    let store::JournalPublication::Confirmed(snapshot) = block_on(fixture.journal.mutate(
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(original.clone())]),
    ))
    .unwrap() else {
        panic!("confirmed fixture write");
    };
    fixture.manager.children[0].snapshot = *snapshot;
    // Same display ID/generation is not the original transcript incarnation.
    let foreign_engine = machine_god_core::Engine::builder()
        .provider(machine_god_testkit::ScriptedModelProvider::new("test", []))
        .permission_handler(machine_god_testkit::ScriptedPermissionHandler::new([]))
        .session_store(machine_god_testkit::InMemorySessionStore::default())
        .build()
        .unwrap();
    let foreign_session = foreign_engine
        .create_session(
            session.id(),
            machine_god_core::SessionIncarnationId::new("foreign-incarnation").unwrap(),
        )
        .unwrap();
    let foreign = Arc::new(ParentNoticeContext::new(
        &foreign_session,
        target.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&foreign).unwrap();
    fixture.drive(|f| f.manager.replay.done && f.manager.active.is_none());
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&target, 64, 64 * 1024)
            .unwrap()
            .entries()
            .is_empty()
    );
    // Reparenting after publication must not rewrite an original's recipient.
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let store::JournalPublication::Confirmed(snapshot) = block_on(fixture.journal.mutate(
        snapshot,
        JournalMutation::Relationship {
            parent_id: Some("later-parent".into()),
            parent_owner: Some(store::JournalTranscript {
                session_id: machine_god_core::SessionId::new("later-parent").unwrap(),
                incarnation:
                    machine_god_core::SessionIncarnationId::new("later-incarnation").unwrap(),
            }),
            parent_generation: Some(1),
        },
    ))
    .unwrap() else {
        panic!("confirmed later relationship");
    };
    fixture.manager.children[0].snapshot = *snapshot;
    let context = Arc::new(ParentNoticeContext::new(
        &session,
        target.clone(),
        &fixture.manager.notices,
    ));
    assert!(matches!(
        fixture.manager.register_parent_context(&context),
        Err(ManagedRuntimeError::Invalid)
    ));
    foreign.retire();
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.drive(|f| {
        f.manager
            .notices
            .snapshot(&target, 64, 64 * 1024)
            .unwrap()
            .entries()
            .len()
            == 1
    });
    let batch = fixture
        .manager
        .notices
        .snapshot(&target, 64, 64 * 1024)
        .unwrap();
    assert_eq!(batch.entries()[0].notice(), &original);
    assert_eq!(fixture.manager.notices.usage().trackers, 0);
    fixture.drive(|f| f.manager.active.is_none());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let record = session.record();
    let checkpoint = NoticeCheckpoint {
        session_id: record.id.clone(),
        incarnation_id: record.incarnation_id.clone(),
        expected_revision: record.revision,
        turn_sequence: 1,
        first_user_message: 0,
    };
    let store::JournalPublication::Confirmed(snapshot) = block_on(fixture.journal.mutate(
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::NoticeAcknowledged {
            identity: original.identity(),
            target: original.target.clone(),
            checkpoint,
        }]),
    ))
    .unwrap() else {
        panic!("confirmed ACK");
    };
    fixture.manager.children[0].snapshot = *snapshot;
    fixture.restart_manager();
    let restored = Arc::new(ParentNoticeContext::new(
        &session,
        target.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&restored).unwrap();
    fixture.drive(|f| f.manager.replay.done && f.manager.active.is_none());
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&target, 64, 64 * 1024)
            .unwrap()
            .entries()
            .is_empty()
    );
    assert!(fixture.factory.provider.requests().is_empty());
}

fn completed() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "answer".into(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}
#[test]
fn accepted_child_runs_without_any_ui_observer() {
    let mut fixture = Fixture::new(vec![completed()]);
    let receipt = fixture.command(
        serde_json::json!({"create":{"name":"worker","mode":"one_off","prompt":"standalone"}}),
    );
    assert_eq!(receipt.status, ManagedResultStatus::Created);
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Completed
            && !f.manager.children[0].busy()
    });
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    let inspected = fixture
        .command(serde_json::json!({"inspect":{"id":"child-1","sections":["status","messages"]}}));
    assert!(inspected.ok, "{inspected:?}");
}
#[test]
fn empty_persistent_create_does_not_execute_and_messages_remain_fifo() {
    let mut fixture = Fixture::new(vec![completed(), completed()]);
    assert!(
        fixture
            .command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}}))
            .ok
    );
    assert!(fixture.factory.provider.requests().is_empty());
    assert!(
        fixture
            .command(serde_json::json!({"message":{"send":{"id":"child-1","content":"first"}}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"message":{"send":{"id":"child-1","content":"second"}}}))
            .ok
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Idle
            && !f.manager.children[0].busy()
    });
    assert_eq!(fixture.factory.provider.requests().len(), 2);
}
#[test]
fn actual_cleanup_not_terminal_status_releases_child_capacity() {
    let mut fixture = Fixture::new(vec![completed()]);
    fixture.factory.cleanup.store(false, Ordering::Release);
    assert!(
        fixture
            .command(
                serde_json::json!({"create":{"name":"worker","mode":"one_off","prompt":"task"}})
            )
            .ok
    );
    fixture.drive(|f| f.manager.children[0].snapshot.head.status == ManagedAgentState::Completed);
    assert!(!fixture.manager.children[0].actual_settled);
    assert_eq!(fixture.factory.scheduler.snapshot().residents, 1);
    fixture.factory.cleanup.store(true, Ordering::Release);
    fixture.drive(|f| !f.manager.children[0].busy() && f.manager.children[0].actual_settled);
}
#[test]
fn idle_cancel_and_close_retain_history_without_execution() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"cancel"}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"close"}}))
            .ok
    );
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.is_empty());
    // Restore fixture's transcript exists only after a prompt checkpoint; this
    // empty case verifies close/retention and never invents transcript creation.
    assert!(fixture.factory.provider.requests().is_empty());
    assert!(
        fixture
            .command(serde_json::json!({"inspect":{"id":"child-1","sections":["status"]}}))
            .ok
    );
}

#[test]
fn reopen_settles_old_runtime_and_requires_an_explicit_new_message() {
    let mut fixture = Fixture::new(vec![completed(), completed()]);
    assert!(
        fixture
            .command(
                serde_json::json!({"create":{"name":"worker","mode":"persistent","prompt":"first"}})
            )
            .ok
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Idle
            && !f.manager.children[0].busy()
    });
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"close"}}))
            .ok
    );
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.is_empty());
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"reopen"}}))
            .ok
    );
    assert_eq!(fixture.manager.children[0].snapshot.head.generation, 2);
    assert!(fixture.manager.children[0].snapshot.head.queue.is_empty());
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    assert!(
        fixture
            .command(
                serde_json::json!({"message":{"send":{"id":"child-1","content":"explicit next"}}})
            )
            .ok
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Idle
            && !f.manager.children[0].busy()
    });
    assert_eq!(fixture.factory.provider.requests().len(), 2);
}

#[test]
fn missing_run_does_not_release_unsettled_admission() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}}))
            .ok
    );
    fixture.factory.cleanup.store(false, Ordering::Release);
    let child = &mut fixture.manager.children[0];
    assert!(child.prepared.owner.run().is_none());
    child.admission_pending = true;
    child.actual_settled = false;
    let mut cx = Context::from_waker(Waker::noop());
    let _ = fixture.manager.poll_progress(&mut cx, 0);
    assert!(fixture.manager.children[0].admission_pending);
    assert!(!fixture.manager.children[0].actual_settled);
    fixture.factory.cleanup.store(true, Ordering::Release);
    fixture.drive(|f| {
        f.manager.children[0].actual_settled && !f.manager.children[0].admission_pending
    });
}

#[test]
#[cfg(feature = "ai-gateway-http")]
fn full_size_queued_message_has_bounded_inspection() {
    let mut fixture = Fixture::new(vec![completed()]);
    fixture.factory.cleanup.store(false, Ordering::Release);
    assert!(
        fixture
            .command(
                serde_json::json!({"create":{"name":"worker","mode":"persistent","prompt":"first"}})
            )
            .ok
    );
    fixture.drive(|f| f.manager.children[0].snapshot.head.status == ManagedAgentState::Idle);
    let content = "\u{1}".repeat(64 * 1024);
    assert!(
        fixture
            .command(serde_json::json!({"message":{"send":{"id":"child-1","content":content}}}))
            .ok
    );
    let result = fixture.command(serde_json::json!({"inspect":{"id":"child-1","sections":["status","messages","events","tool_activity"],"limit":100}}));
    assert!(result.ok, "{result:?}");
    assert!(result.validate().is_ok());
    assert!(serde_json::to_vec(&result).unwrap().len() <= 512 * 1024);
    fixture.factory.cleanup.store(true, Ordering::Release);
}

#[test]
fn preparation_ambiguity_retains_one_candidate_and_executes_nothing() {
    let mut fixture = Fixture::new(vec![completed()]);
    fixture.factory.ambiguous.store(true, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(
        serde_json::json!({"create":{"name":"worker","mode":"one_off","prompt":"task"}}),
    );
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    fixture.drive(|f| f.factory.prepared.load(Ordering::Relaxed) == 1);
    assert!(fixture.factory.provider.requests().is_empty());
    assert!(matches!(
        block_on(fixture.journal.inspect("child-1".into())),
        Err(store::JournalError::Missing)
    ));
    fixture.factory.reconcile.store(true, Ordering::Release);
    fixture.drive(|f| f.manager.children.len() == 1);
    assert!(block_on(response).unwrap().ok);
    assert_eq!(fixture.factory.prepared.load(Ordering::Relaxed), 1);
}

#[test]
fn dropped_mutation_observer_does_not_drop_manager_owned_preparation() {
    let mut fixture = Fixture::new(vec![completed()]);
    fixture.factory.ambiguous.store(true, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(
        serde_json::json!({"create":{"name":"worker","mode":"one_off","prompt":"task"}}),
    );
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    fixture.drive(|f| f.factory.prepared.load(Ordering::Relaxed) == 1);
    drop(response);
    fixture.factory.reconcile.store(true, Ordering::Release);
    fixture.drive(|f| {
        !f.manager.children.is_empty()
            && f.manager.children[0].snapshot.head.status == ManagedAgentState::Completed
    });
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn retired_actual_caller_cannot_publish_after_async_preparation() {
    let mut fixture = Fixture::new(vec![]);
    fixture.factory.ambiguous.store(true, Ordering::Release);
    let (admission, invocation) =
        fixture.invocation(serde_json::json!({"create":{"name":"worker","mode":"persistent"}}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    fixture.drive(|f| f.factory.prepared.load(Ordering::Relaxed) == 1);
    drop(admission);
    fixture.factory.reconcile.store(true, Ordering::Release);
    fixture.drive(|f| f.manager.active.is_none());
    assert_eq!(
        block_on(response).unwrap().error_code,
        Some(ManagedFailureCode::CallerUnavailable)
    );
    assert!(fixture.manager.children.is_empty());
    assert!(fixture.factory.provider.requests().is_empty());
}

#[test]
fn cancel_interrupts_remaining_fifo_until_explicit_resume() {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending(), completed()]);
    assert!(
        fixture
            .command(
                serde_json::json!({"create":{"name":"worker","mode":"persistent","prompt":"first"}})
            )
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    assert!(
        fixture
            .command(serde_json::json!({"message":{"send":{"id":"child-1","content":"second"}}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"cancel"}}))
            .ok
    );
    assert_eq!(
        fixture.manager.children[0].snapshot.head.queue[0].status,
        ManagedQueueStatus::Interrupted
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"resume"}}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 2 && !f.manager.children[0].busy());
}

#[test]
fn configured_future_work_does_not_rewrite_accepted_configuration() {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending()]);
    assert!(
        fixture
            .command(
                serde_json::json!({"create":{"name":"worker","mode":"persistent","prompt":"first"}})
            )
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    assert!(
        fixture
            .command(serde_json::json!({"configure":{"id":"child-1","model":"new-model"}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"message":{"send":{"id":"child-1","content":"next"}}}))
            .ok
    );
    // A delivered tool reply can precede the manager's next replay/read poll.
    // Wait for that owned journal operation before independent fixture reads.
    fixture.drive(|f| f.manager.active.is_none());
    let head = &fixture.manager.children[0].snapshot.head;
    let first = block_on(fixture.journal.read_work(head.queue[0].page.clone())).unwrap();
    let next = block_on(fixture.journal.read_work(head.queue[1].page.clone())).unwrap();
    assert_eq!(first.configuration.model.as_deref(), Some("model"));
    assert_eq!(next.configuration.model.as_deref(), Some("new-model"));
}

#[test]
fn permission_escalation_fails_before_runtime_preparation() {
    let mut fixture = Fixture::new(vec![]);
    let result = fixture.command(serde_json::json!({"create":{"name":"worker","mode":"persistent","permission_mode":"yolo"}}));
    assert_eq!(
        result.error_code,
        Some(ManagedFailureCode::PermissionDenied)
    );
    assert_eq!(fixture.factory.prepared.load(Ordering::Relaxed), 0);
}

#[test]
fn closing_nonresident_child_restricts_repair_without_changing_saved_policy() {
    use machine_god_core::{ManagedAgentState, ManagedPermissionMode};
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({
                "create": {"name": "saved-worker", "mode": "persistent"}
            }))
            .ok
    );
    fixture.restart_manager();
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let mut configuration = snapshot.head.configuration.clone();
    configuration.permission_mode = ManagedPermissionMode::Yolo;
    assert!(matches!(
        block_on(
            fixture
                .journal
                .mutate(snapshot, JournalMutation::Configure(configuration))
        ),
        Ok(store::JournalPublication::Confirmed(_))
    ));
    assert!(
        fixture
            .command(serde_json::json!({
                "lifecycle": {"id": "child-1", "action": "close"}
            }))
            .ok
    );
    assert_eq!(
        fixture.factory.prepared_modes.lock().unwrap().last(),
        Some(&ManagedPermissionMode::Ask)
    );
    // The close response can precede manager-owned replay settlement. Keep
    // this independent assertion read out of the replay's journal slot.
    fixture.drive(|f| f.manager.active.is_none());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(snapshot.head.status, ManagedAgentState::Archived);
    assert_eq!(
        snapshot.head.configuration.permission_mode,
        ManagedPermissionMode::Yolo
    );
    assert!(fixture.factory.provider.requests().is_empty());
    let reopen = fixture.command(serde_json::json!({
        "lifecycle": {"id": "child-1", "action": "reopen"}
    }));
    assert_eq!(
        reopen.error_code,
        Some(ManagedFailureCode::PermissionDenied)
    );
    assert!(
        fixture
            .command(serde_json::json!({
                "create": {"name": "subsequent-worker", "mode": "persistent"}
            }))
            .ok
    );
    fixture.manager.request_shutdown();
    block_on(std::future::poll_fn(|cx| {
        fixture.manager.poll_shutdown(cx, 101)
    }))
    .unwrap();
}

#[test]
fn native_selection_is_allocation_bound_and_idle_residency_is_reusable() {
    let mut fixture = Fixture::new(vec![]);
    fixture.manager.limits.residents = 1;
    assert!(
        fixture
            .command(serde_json::json!({"create":{"name":"first","mode":"persistent"}}))
            .ok
    );
    let selection = fixture.manager.children()[0].selection.clone();
    assert!(fixture.manager.selected_runtime(&selection).is_some());
    assert!(
        fixture
            .command(serde_json::json!({"create":{"name":"second","mode":"persistent"}}))
            .ok
    );
    assert!(fixture.manager.selected_runtime(&selection).is_none());
    assert_eq!(fixture.manager.children.len(), 1);
    assert!(
        fixture
            .command(serde_json::json!({"inspect":{"id":"child-1","sections":["status"]}}))
            .ok
    );
}

#[test]
fn inspection_cursor_binds_exact_revision_and_selected_sections() {
    let mut fixture = Fixture::new(vec![completed()]);
    assert!(
        fixture
            .command(
                serde_json::json!({"create":{"name":"worker","mode":"one_off","prompt":"task"}})
            )
            .ok
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Completed
            && !f.manager.children[0].busy()
    });
    let first = fixture
        .command(serde_json::json!({"inspect":{"id":"child-1","sections":["messages"],"limit":1}}));
    let cursor = first.cursor.unwrap();
    let stale = fixture.command(serde_json::json!({"inspect":{"id":"child-1","sections":["events"],"cursor":cursor,"limit":1}}));
    let Some(ManagedRequested::Inspection(page)) = stale.requested else {
        panic!("inspection")
    };
    assert!(page.restart_required);
}
