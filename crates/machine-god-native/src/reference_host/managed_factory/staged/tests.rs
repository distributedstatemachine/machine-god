use super::super::tests::FactoryFixture;
use super::*;
use crate::mcp::runtime::NativeMcpRuntimeClock;
use crate::{NativeSessionMetadata, NativeSessionOrigin};
use futures_executor::block_on;
use futures_util::future::poll_fn;
use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

#[derive(Default)]
struct Clock(
    AtomicUsize,
    std::sync::Mutex<Option<PendingWorker>>,
    std::sync::atomic::AtomicBool,
);
type PendingWorker = (crate::NativeOwnedWorkerScope, std::sync::mpsc::Receiver<()>);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0.fetch_add(1, Ordering::Relaxed);
        let worker = self.1.lock().unwrap().take();
        if let Some((scope, release)) = worker {
            scope
                .spawn(move || {
                    let _ = release.recv_timeout(Duration::from_secs(10));
                })
                .unwrap();
        }
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        if self.2.load(Ordering::Acquire) {
            Box::pin(async {})
        } else {
            Box::pin(std::future::pending())
        }
    }
}
fn stage(f: &FactoryFixture, configuration: NativeMcpEphemeralConfiguration) -> StagedParentMcp {
    block_on(f.factory.stage_parent_mcp(
        f.journal.owner_lease(),
        f.parent_mcp.clone(),
        configuration,
        CancellationToken::new(),
    ))
    .unwrap()
}
fn empty(f: &FactoryFixture) -> StagedParentMcp {
    stage(f, NativeMcpEphemeralConfiguration::decode(None).unwrap())
}
fn conversation(f: &FactoryFixture) -> NativeConversation {
    let session = block_on(
        f.factory
            .0
            .services
            .session_lifecycle
            .create_generated_with_metadata(
                NativeSessionMetadata::new(&f.host.workspace, 3, NativeSessionOrigin::Acp).unwrap(),
            ),
    )
    .unwrap();
    NativeConversation::from_session(session).unwrap()
}
fn selection(f: &FactoryFixture) -> ManagedRestorationAuthority {
    let selected = &f.factory.0.restoration;
    ManagedRestorationAuthority {
        workspace: selected.workspace.clone(),
        policy: selected.policy.clone(),
        preferences: selected.preferences.clone(),
    }
}
fn principal(conversation: &NativeConversation) -> NoticePrincipal {
    NoticePrincipal {
        id: conversation.core_session().id().to_string(),
        generation: NonZeroU64::MIN,
    }
}

#[test]
fn unpolled_stage_and_settlement_do_not_read_the_clock() {
    let clock = Arc::new(Clock::default());
    let f = FactoryFixture::with_mcp(clock.clone(), true);
    let before = clock.0.load(Ordering::Relaxed);
    drop(f.factory.stage_parent_mcp(
        f.journal.owner_lease(),
        f.parent_mcp.clone(),
        NativeMcpEphemeralConfiguration::decode(None).unwrap(),
        CancellationToken::new(),
    ));
    assert_eq!(clock.0.load(Ordering::Relaxed), before);
    let mut staged = empty(&f);
    staged.ready().unwrap();
    let before = clock.0.load(Ordering::Relaxed);
    drop(staged.settle());
    assert_eq!(clock.0.load(Ordering::Relaxed), before);
    staged.ready().unwrap();
    block_on(staged.settle()).unwrap();
    assert!(staged.ready().is_err());
}

#[test]
fn staged_publication_transfers_without_recomposition_or_child_inheritance() {
    let f = FactoryFixture::with_mcp(Arc::new(Clock::default()), true);
    let staged = empty(&f);
    let original = staged.mcp.as_ref().unwrap().instance.runtime.clone();
    let ephemeral = staged
        .mcp
        .as_ref()
        .unwrap()
        .instance
        .ephemeral
        .clone()
        .unwrap();
    let checkpoint = original.publication_checkpoint().unwrap();
    let conversation = conversation(&f);
    let principal = principal(&conversation);
    let mut prepared =
        block_on(
            f.factory
                .prepare_staged_parent(staged, conversation, selection(&f), principal),
        )
        .unwrap();
    let controls = prepared.resources.mcp_controls().unwrap();
    assert!(Arc::ptr_eq(controls.runtime.as_ref().unwrap(), &original));
    assert!(Arc::ptr_eq(
        controls.ephemeral.as_ref().unwrap(),
        &ephemeral
    ));
    assert!(
        original
            .publication_checkpoint()
            .unwrap()
            .same_selection(&checkpoint)
    );
    ephemeral.ready().unwrap();
    // The shared child seed still has no authoritative request-scoped selection.
    let child = f
        .factory
        .0
        .compose_mcp(f.factory.0.services.control_workers.as_ref().unwrap())
        .unwrap();
    assert!(child.ephemeral.is_none());
    assert!(!Arc::ptr_eq(&child.runtime, &original));
    drop(child);
    prepared.owner.retire();
    block_on(poll_fn(|cx| prepared.resources.poll_closed(cx))).unwrap();
    assert!(ephemeral.ready().is_err());
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn rejected_startup_retains_the_original_stage_for_owned_cleanup() {
    let f = FactoryFixture::with_mcp(Arc::new(Clock::default()), true);
    let configuration = NativeMcpEphemeralConfiguration::decode(Some(
        br#"[{"name":"unavailable","command":"/bin/sh","args":[],"env":[]}]"#,
    ))
    .unwrap();
    let mut staged = stage(&f, configuration);
    assert!(staged.ready().is_err());
    assert!(staged.mcp.is_some());
    block_on(staged.settle()).unwrap();
    assert!(staged.ready().is_err());
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn a_foreign_factory_returns_the_ready_original_instead_of_rebinding_it() {
    let f = FactoryFixture::with_mcp(Arc::new(Clock::default()), true);
    let foreign = FactoryFixture::with_mcp(Arc::new(Clock::default()), true);
    let staged = empty(&f);
    let conversation = conversation(&f);
    let principal = principal(&conversation);
    let Err(mut failure) = block_on(foreign.factory.prepare_staged_parent(
        staged,
        conversation,
        selection(&f),
        principal,
    )) else {
        panic!("foreign stage accepted");
    };
    assert_eq!(failure.error, ManagedRuntimeError::Invalid);
    failure.stage.ready().unwrap();
    block_on(failure.stage.settle()).unwrap();
}

#[test]
fn a_closed_stage_cannot_create_a_foreground_runtime() {
    let f = FactoryFixture::with_mcp(Arc::new(Clock::default()), true);
    let mut staged = empty(&f);
    block_on(staged.settle()).unwrap();
    let conversation = conversation(&f);
    let principal = principal(&conversation);
    let Err(mut failure) =
        block_on(
            f.factory
                .prepare_staged_parent(staged, conversation, selection(&f), principal),
        )
    else {
        panic!("closed stage accepted");
    };
    assert_eq!(failure.error, ManagedRuntimeError::Unavailable);
    block_on(failure.stage.settle()).unwrap();
}

#[test]
fn abandoning_a_cleanup_wrapper_retains_the_original_startup_worker_observation() {
    let clock = Arc::new(Clock::default());
    let f = FactoryFixture::with_mcp(clock.clone(), true);
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let (release, receive) = std::sync::mpsc::channel();
    *clock.1.lock().unwrap() = Some((workers.clone(), receive));
    // The first selected-clock read occurs inside the attributed startup poll.
    // A ready stage must not join this still-live original worker yet.
    let mut staged = empty(&f);
    staged.ready().unwrap();
    let mut cleanup = staged.settle();
    let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(cleanup.as_mut().poll(&mut cx).is_pending());
    drop(cleanup);
    assert!(staged.closing.is_some());
    let before = clock.0.load(Ordering::Relaxed);
    let mut resumed = staged.settle();
    assert!(resumed.as_mut().poll(&mut cx).is_pending());
    assert_eq!(clock.0.load(Ordering::Relaxed), before);
    release.send(()).unwrap();
    block_on(resumed).unwrap();
    assert_eq!(staged.settled, Some(Ok(())));
    assert!(staged.closing.is_none());
    block_on(staged.settle()).unwrap();
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn adopted_startup_workers_are_joined_at_retirement_not_each_admission() {
    let clock = Arc::new(Clock::default());
    let f = FactoryFixture::with_mcp(clock.clone(), true);
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let (release, receive) = std::sync::mpsc::channel();
    *clock.1.lock().unwrap() = Some((workers.clone(), receive));
    let staged = empty(&f);
    let conversation = conversation(&f);
    let principal = principal(&conversation);
    let mut prepared =
        block_on(
            f.factory
                .prepare_staged_parent(staged, conversation, selection(&f), principal),
        )
        .unwrap();
    let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(matches!(
        prepared.resources.poll_admission_settled(&mut cx),
        std::task::Poll::Ready(Ok(()))
    ));
    prepared.owner.retire();
    assert!(prepared.resources.poll_closed(&mut cx).is_pending());
    release.send(()).unwrap();
    block_on(poll_fn(|cx| prepared.resources.poll_closed(cx))).unwrap();
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn a_timed_out_cleanup_cannot_later_claim_a_successful_retirement() {
    let clock = Arc::new(Clock::default());
    let f = FactoryFixture::with_mcp(clock.clone(), true);
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let (release, receive) = std::sync::mpsc::channel();
    *clock.1.lock().unwrap() = Some((workers.clone(), receive));
    let mut staged = empty(&f);
    clock.2.store(true, Ordering::Release);
    assert_eq!(
        block_on(staged.settle()),
        Err(ManagedRuntimeError::Unavailable)
    );
    release.send(()).unwrap();
    let before = clock.0.load(Ordering::Relaxed);
    assert_eq!(
        block_on(staged.settle()),
        Err(ManagedRuntimeError::Unavailable)
    );
    assert_eq!(clock.0.load(Ordering::Relaxed), before);
    assert_eq!(staged.settled, Some(Err(ManagedRuntimeError::Unavailable)));
}
