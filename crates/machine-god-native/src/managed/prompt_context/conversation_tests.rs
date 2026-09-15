use super::{ParentNoticeContext, saved_context};
use crate::NativeConversation;
use crate::managed::notices::{
    ManagedNotices, NoticeLimits, NoticePrincipal, NoticeRelationship, NoticeTerminal,
    PreparedNotice, WorkNoticeIdentity,
};
use crate::mcp::runtime::NativeMcpRuntimeClock;
use futures_executor::block_on;
use machine_god_core::Session;
use machine_god_core::{
    BoxFuture, Engine, ManagedNotifications, Prompt, SessionId, SessionIncarnationId,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::{num::NonZeroU64, sync::Arc, time::Instant};

struct Clock;
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
fn principal(id: &str) -> NoticePrincipal {
    NoticePrincipal {
        id: id.into(),
        generation: NonZeroU64::new(1).unwrap(),
    }
}
fn emit(notices: &ManagedNotices, work_id: &str) {
    let work = notices
        .register_work(
            &WorkNoticeIdentity {
                source: principal("child"),
                work_id: work_id.into(),
                work_generation: NonZeroU64::new(1).unwrap(),
            },
            ManagedNotifications::default(),
            &NoticeRelationship {
                generation: NonZeroU64::new(1).unwrap(),
                parent: Some(principal("parent")),
            },
            0,
        )
        .unwrap();
    let PreparedNotice::Staged(stage) = notices
        .prepare_terminal(
            &work,
            NonZeroU64::new(1).unwrap(),
            NoticeTerminal::Completed,
            None,
        )
        .unwrap()
    else {
        panic!("terminal fixture is staged");
    };
    notices.confirm_durable(&stage).unwrap();
}
fn pending(notices: &ManagedNotices) -> usize {
    notices
        .snapshot(&principal("parent"), 64, 65536)
        .unwrap()
        .entries()
        .len()
}
fn engine(provider: &ScriptedModelProvider) -> Engine {
    Engine::builder()
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .build()
        .unwrap()
}
fn session(engine: &Engine) -> Session {
    engine
        .create_session(
            SessionId::new("parent").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        )
        .unwrap()
}

#[test]
fn actual_native_prompt_acknowledges_only_its_checkpoint_and_continuation_is_inert() {
    let provider = ScriptedModelProvider::new("test", []);
    let engine = engine(&provider);
    let session = session(&engine);
    let notices = Arc::new(ManagedNotices::new(NoticeLimits::default(), Arc::new(Clock)).unwrap());
    let owner = Arc::new(ParentNoticeContext::new(
        &session,
        principal("parent"),
        &notices,
    ));
    let conversation = NativeConversation::from_session(session.clone())
        .unwrap()
        .with_notice_context(&owner)
        .unwrap();
    emit(&notices, "first");
    let future = conversation.prompt(
        Prompt {
            text: "standalone parent input".into(),
            options: Default::default(),
        },
        1,
    );
    assert_eq!(pending(&notices), 1);
    let turn = block_on(future).unwrap();
    assert!(provider.requests().is_empty());
    assert_eq!(pending(&notices), 0);
    let checkpoint = {
        let record = session.record();
        (record.next_turn_sequence - 1, 0)
    };
    let before = saved_context(&session.record(), Some((checkpoint.0, checkpoint.1)))
        .unwrap()
        .unwrap();
    emit(&notices, "second");
    assert_eq!(pending(&notices), 1);
    drop(turn);
    assert!(conversation.paused_turn().unwrap().is_some());
    let continued = block_on(conversation.continue_turn(Default::default(), 2)).unwrap();
    let checkpoint = {
        let record = session.record();
        (record.next_turn_sequence - 1, 0)
    };
    let after = saved_context(&session.record(), Some((checkpoint.0, checkpoint.1)))
        .unwrap()
        .unwrap();
    assert_eq!(before.text(), after.text());
    assert_eq!(pending(&notices), 1);
    assert!(provider.requests().is_empty());
    drop(continued);
}

#[test]
fn foreign_binding_and_dropped_owner_fail_before_native_prompt_publication() {
    let provider = ScriptedModelProvider::new("test", []);
    let engine = engine(&provider);
    let session = session(&engine);
    let foreign = self::engine(&provider);
    let foreign_session = self::session(&foreign);
    let notices = Arc::new(ManagedNotices::new(NoticeLimits::default(), Arc::new(Clock)).unwrap());
    let owner = Arc::new(ParentNoticeContext::new(
        &session,
        principal("parent"),
        &notices,
    ));
    assert!(
        NativeConversation::from_session(foreign_session)
            .unwrap()
            .with_notice_context(&owner)
            .is_err()
    );
    let conversation = NativeConversation::from_session(session.clone())
        .unwrap()
        .with_notice_context(&owner)
        .unwrap();
    let original = session.record();
    drop(owner);
    assert!(
        block_on(conversation.prompt(
            Prompt {
                text: "not published".into(),
                options: Default::default()
            },
            1
        ))
        .is_err()
    );
    assert_eq!(session.record(), original);
    assert!(provider.requests().is_empty());
}
