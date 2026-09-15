use super::*;
use crate::managed::{
    mailbox::{MailboxLimits, ManagedMailbox},
    manager::{ManagedManager, ManagerLimits},
};

#[test]
fn enrolled_parent_shutdown_waits_for_its_admission_worker_not_unrelated_host_work() {
    let f = FactoryFixture::new();
    let session = block_on(
        f.factory
            .0
            .services
            .session_lifecycle
            .create_generated_with_metadata(
                NativeSessionMetadata::new(&f.host.workspace, 3, NativeSessionOrigin::Cli).unwrap(),
            ),
    )
    .unwrap();
    let selected = &f.factory.0.restoration;
    let prepared = block_on(f.factory.prepare_parent(
        session.clone(),
        ManagedRestorationAuthority {
            workspace: selected.workspace.clone(),
            policy: selected.policy.clone(),
            preferences: selected.preferences.clone(),
        },
        NoticePrincipal {
            id: session.id().to_string(),
            generation: NonZeroU64::new(1).unwrap(),
        },
        f.journal.owner_lease(),
    ))
    .unwrap();
    let binding = prepared.owner.binding();
    let context = Arc::downgrade(prepared.notice_context.as_ref().unwrap());
    let owner = prepared.owner.principal().clone();
    let mut manager = ManagedManager::new(
        f.journal.clone(),
        ManagedMailbox::new(f.factory.0.principals.requester(), MailboxLimits::default()).unwrap(),
        Arc::new(SharedManagedRuntimeFactory(f.factory.0.clone())),
        Arc::new(
            crate::reference_host::managed_host::relationship::RelationshipConsent::new(
                f.factory.0.principals.requester(),
                f.host.prompt.clone(),
            ),
        ),
        f.factory.0.notices.clone(),
        Arc::new(NoTimerClock),
        ManagerLimits::default(),
    )
    .unwrap();
    let selected = manager.enroll_foreground(Box::new(prepared)).unwrap();
    let runtime = manager.foreground_runtime(&selected).unwrap().clone();
    assert!(!runtime.status().active);
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let (release, receiver) = std::sync::mpsc::channel();
    let admission = binding.prepare_admission().unwrap();
    let cleanup = binding.admission_completion().unwrap();
    // Attribute through the actual admission before any core run exists.
    admission
        .cohort()
        .unwrap()
        .with_poll(|| {
            workers.spawn(move || {
                let _ = receiver.recv();
            })
        })
        .unwrap();
    drop(admission);
    manager.request_shutdown();
    assert!(!owner.is_live());
    assert!(manager.foreground_runtime(&selected).is_none());
    assert!(
        manager
            .poll_shutdown(
                &mut Context::from_waker(futures_util::task::noop_waker_ref()),
                5
            )
            .is_pending()
    );
    assert!(!cleanup.is_complete());
    assert!(context.upgrade().is_some());
    let (unrelated, receiver) = std::sync::mpsc::channel();
    workers
        .spawn(move || {
            let _ = receiver.recv();
        })
        .unwrap();
    release.send(()).unwrap();
    block_on(poll_fn(|cx| manager.poll_shutdown(cx, 6))).unwrap();
    assert!(cleanup.is_complete());
    assert!(context.upgrade().is_none());
    assert!(!workers.completion().is_complete());
    unrelated.send(()).unwrap();
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}
