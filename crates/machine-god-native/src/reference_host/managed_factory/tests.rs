use super::*;
use crate::managed::{
    notices::NoticeLimits,
    scheduler::SchedulerLimits,
    store::{JournalLimits, ManagedJournal},
};
use crate::mcp::{context::NativeMcpContexts, runtime::NativeMcpRuntimeClock};
use crate::reference_host::*;
use crate::*;
use futures_executor::block_on;
use futures_util::future::poll_fn;
use machine_god_core::{
    ManagedConfiguration, ManagedNotifications, SessionId, SessionIncarnationId,
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Instant,
};

// Reuse the existing fully injected reference-host fixture, not another host builder.
use crate::reference_host::mcp::tests::fixture as host_fixture;

struct NoTimerClock;
impl NativeMcpRuntimeClock for NoTimerClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

thread_local! { static REJECT_PUBLICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(super) fn reject_publication_observation() -> bool {
    REJECT_PUBLICATION.with(|reject| reject.replace(false))
}

fn directory(path: &Path) -> rustix::fd::OwnedFd {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap()
}
struct FactoryFixture {
    factory: SharedManagedRuntimeFactory,
    journal: ManagedJournal,
    parent: Arc<crate::managed::principal::NativePrincipal>,
    parent_session: Session,
    host: host_fixture::Fixture,
}
impl FactoryFixture {
    fn new() -> Self {
        Self::with_clock(Arc::new(NoTimerClock))
    }
    fn with_clock(clock: Arc<dyn NativeMcpRuntimeClock>) -> Self {
        let host = host_fixture::Fixture::with_options("auto", true, |options, _, _| {
            options
                .with_model_routes(Arc::new(NativeConversationModelRoutes::new()))
                .with_observations(Arc::new(NativeConversationObservations::new()))
                .with_mcp_runtime(NativeReferenceHostMcpOptions::new(
                    Arc::new(NativeMcpContexts::new()),
                    clock,
                ))
        });
        let services = host.host().services.clone();
        let workspace = NativeWorkspaceAuthority::open_blocking(
            directory(&host.workspace),
            host.workspace.clone(),
            Some(directory(&host.state)),
            host.state.clone(),
            Vec::new(),
            false,
        )
        .unwrap();
        let principals = Arc::new(
            NativePrincipalRegistry::new(64, Arc::new(NativeUndoBudget::default())).unwrap(),
        );
        let parent_session = block_on(services.session_lifecycle.create_generated_with_metadata(
            NativeSessionMetadata::new(&host.workspace, 1, NativeSessionOrigin::Cli).unwrap(),
        ))
        .unwrap();
        let parent = principals.register(&parent_session, 1, &workspace).unwrap();
        let journal_path = host.state.join("factory-journal");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&journal_path)
            .unwrap();
        let journal = block_on(ManagedJournal::open(
            directory(&journal_path),
            services.control_workers.as_ref().unwrap().clone(),
            JournalLimits::default(),
        ))
        .unwrap();
        let archive = Arc::new(
            NativeToolResultArchiveAdapter::new(Arc::new(ToolResultArchive::from_root_descriptor(
                directory(&host.state),
            )))
            .with_worker_scope(services.control_workers.as_ref().unwrap().clone()),
        );
        let mcp =
            Arc::new(NativePrincipalMcpRegistry::new(64, principals.requester(), archive).unwrap());
        let notices =
            Arc::new(ManagedNotices::new(NoticeLimits::default(), host.clock.clone()).unwrap());
        let factory = SharedManagedRuntimeFactory::new(SharedManagedRuntimeFactoryOptions {
            services,
            principals,
            scheduler: ManagedScheduler::new(SchedulerLimits::default()),
            mcp,
            notices,
            workspace_contexts: Arc::new(NativeWorkspaceContexts::new()),
            restoration: ManagedRestorationAuthority {
                workspace: workspace.snapshot().unwrap(),
                policy: NativePermissionPolicySnapshot::new(
                    PermissionMode::Auto,
                    Arc::new(NativeConfiguredPermissionRules::default()),
                ),
                preferences: NativeModelPreferences::new(
                    "fixture/restoration",
                    NativeReasoningEffort::default(),
                    false,
                )
                .unwrap(),
            },
            reserved_tool_names: host.host().reserved_tool_names().to_vec(),
            origin: NativeSessionOrigin::Cli,
            cleanup_timeout: Duration::from_secs(30),
        })
        .unwrap();
        Self {
            factory,
            journal,
            parent,
            parent_session,
            host,
        }
    }
    fn request(&self, id: &str) -> ManagedRuntimeRequest {
        ManagedRuntimeRequest {
            kind: ManagedRuntimePreparationKind::Create,
            child_id: id.into(),
            generation: 1,
            transcript: JournalTranscript {
                session_id: SessionId::new(id).unwrap(),
                incarnation: SessionIncarnationId::new(format!("{id}-life")).unwrap(),
            },
            journal_owner: self.journal.owner_lease(),
            configuration: ManagedConfiguration {
                name: "worker".into(),
                model: None,
                effort: None,
                permission_mode: ManagedPermissionMode::Ask,
                notifications: ManagedNotifications::default(),
            },
            origin: Some(crate::managed::manager::factory::ManagedRuntimeOrigin {
                principal: self.parent.clone(),
                workspace: self.parent.workspace().snapshot().unwrap(),
                policy: NativePermissionPolicySnapshot::new(
                    PermissionMode::Auto,
                    Arc::new(NativeConfiguredPermissionRules::default()),
                ),
                preferences: NativeModelPreferences::new(
                    "fixture/captured",
                    NativeReasoningEffort::parse("high").unwrap(),
                    false,
                )
                .unwrap(),
            }),
            now_ms: 2,
        }
    }
    fn prepare(&self, request: ManagedRuntimeRequest) -> PreparedManagedRuntime {
        match block_on(self.factory.prepare(request, CancellationToken::new())).unwrap() {
            ManagedPreparation::Ready(runtime) => runtime,
            ManagedPreparation::Ambiguous(_) => panic!("fixture publication must confirm"),
        }
    }
}

#[test]
fn preparation_is_inert_before_poll_and_shares_exact_engine_without_parent_state() {
    let f = FactoryFixture::new();
    let request = f.request("child-a");
    drop(f.factory.prepare(request.clone(), CancellationToken::new()));
    assert!(
        block_on(
            f.factory
                .0
                .services
                .session_store
                .load(request.transcript.session_id.clone())
        )
        .unwrap()
        .is_none()
    );
    let mut child = f.prepare(request);
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
    assert!(child.runtime.record().messages.is_empty());
    assert!(child.notice_context.is_some());
    let loaded = block_on(f.factory.0.services.engine.load_session(child.runtime.id()))
        .unwrap()
        .unwrap();
    child
        .notice_context
        .as_ref()
        .unwrap()
        .validate_session(&loaded)
        .unwrap();
    assert!(!Arc::ptr_eq(
        child.owner.principal().undo(),
        f.parent.undo()
    ));
    assert_ne!(child.runtime.id(), f.parent_session.id());
    child.resources.begin_close();
    block_on(poll_fn(|cx| child.resources.poll_closed(cx))).unwrap();
}

#[test]
fn parent_enrollment_uses_actual_session_without_loading_or_publishing_again() {
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
    let original = session.record();
    let authority = &f.factory.0.restoration;
    let mut parent = block_on(f.factory.prepare_parent(
        session.clone(),
        ManagedRestorationAuthority {
            workspace: authority.workspace.clone(),
            policy: authority.policy.clone(),
            preferences: authority.preferences.clone(),
        },
        NoticePrincipal {
            id: "foreground".into(),
            generation: NonZeroU64::new(1).unwrap(),
        },
        f.journal.owner_lease(),
    ))
    .unwrap();
    assert_eq!(parent.runtime.id(), session.id());
    assert_eq!(parent.runtime.record(), original);
    parent
        .notice_context
        .as_ref()
        .unwrap()
        .validate_session(&session)
        .unwrap();
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
    block_on(poll_fn(|cx| parent.resources.poll_closed(cx))).unwrap();
}

#[test]
fn close_waits_for_cleanup_capacity_without_borrowing_full_ordinary_quota() {
    let f = FactoryFixture::new();
    let mut child = f.prepare(f.request("cleanup-capacity"));
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let ordinary: Vec<_> = (0..64)
        .map(|_| workers.begin_run_with_keepalive(Arc::new(())).unwrap())
        .collect();
    let mut cleanup: Vec<_> = (0..64)
        .map(|_| {
            workers
                .begin_cleanup_run_with_keepalive(Arc::new(()))
                .unwrap()
        })
        .collect();
    child.resources.begin_close();
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(child.resources.poll_closed(&mut cx).is_pending());
    cleanup.pop().unwrap().close();
    block_on(poll_fn(|cx| child.resources.poll_closed(cx))).unwrap();
    assert!(child.resources.poll_closed(&mut cx).is_ready());
    for cohort in cleanup.into_iter().chain(ordinary) {
        cohort.close();
    }
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn cleanup_capacity_timeout_remains_an_error_without_reset_or_false_completion() {
    struct DeadlineClock {
        base: Instant,
        expired: std::sync::atomic::AtomicBool,
        reads: AtomicUsize,
    }
    impl NativeMcpRuntimeClock for DeadlineClock {
        fn now(&self) -> Instant {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.base
        }
        fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
            assert_eq!(deadline, self.base + Duration::from_secs(30));
            Box::pin(poll_fn(|_| {
                if self.expired.load(Ordering::Acquire) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }))
        }
    }
    let clock = Arc::new(DeadlineClock {
        base: Instant::now(),
        expired: std::sync::atomic::AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let f = FactoryFixture::with_clock(clock.clone());
    let mut child = f.prepare(f.request("cleanup-deadline"));
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let cleanup: Vec<_> = (0..64)
        .map(|_| {
            workers
                .begin_cleanup_run_with_keepalive(Arc::new(()))
                .unwrap()
        })
        .collect();
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(child.resources.poll_closed(&mut cx).is_pending());
    clock.expired.store(true, Ordering::Release);
    assert_eq!(
        child.resources.poll_closed(&mut cx),
        Poll::Ready(Err(ManagedRuntimeError::Unavailable))
    );
    for cohort in cleanup {
        cohort.close();
    }
    assert_eq!(
        child.resources.poll_closed(&mut cx),
        Poll::Ready(Err(ManagedRuntimeError::Unavailable))
    );
    assert_eq!(clock.reads.load(Ordering::Relaxed), 1);
}

#[test]
fn explicit_model_effort_and_stricter_policy_win_over_captured_preferences() {
    let f = FactoryFixture::new();
    let mut request = f.request("configured");
    request.configuration.model = Some("fixture/explicit".into());
    request.configuration.effort = Some("low".into());
    let child = f.prepare(request);
    assert_eq!(
        child.runtime.model_preferences().model(),
        "fixture/explicit"
    );
    assert_eq!(child.runtime.model_preferences().effort().label(), "low");
    assert_eq!(
        child
            .runtime
            .permissions()
            .unwrap()
            .snapshot()
            .unwrap()
            .mode(),
        PermissionMode::Ask
    );
    let mut request = f.request("escalation");
    request.configuration.permission_mode = ManagedPermissionMode::Yolo;
    assert!(matches!(
        block_on(f.factory.prepare(request, CancellationToken::new())),
        Err(ManagedRuntimeError::Invalid)
    ));
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn restore_is_exact_and_effects_free_and_never_creates_missing_identity() {
    let f = FactoryFixture::new();
    let mut missing = f.request("absent");
    missing.kind = ManagedRuntimePreparationKind::Restore;
    missing.origin = None;
    assert!(matches!(
        block_on(f.factory.prepare(missing.clone(), CancellationToken::new())),
        Err(ManagedRuntimeError::Missing)
    ));
    assert!(
        block_on(
            f.factory
                .0
                .services
                .session_store
                .load(missing.transcript.session_id)
        )
        .unwrap()
        .is_none()
    );
    let request = f.request("restored");
    let child = f.prepare(request.clone());
    drop(child);
    let mut restore = request;
    restore.kind = ManagedRuntimePreparationKind::Restore;
    restore.origin = None;
    let child = f.prepare(restore.clone());
    assert_eq!(
        child.runtime.model_preferences().model(),
        "fixture/restoration"
    );
    drop(child);
    restore.transcript.incarnation = SessionIncarnationId::new("wrong-incarnation").unwrap();
    assert!(matches!(
        block_on(f.factory.prepare(restore, CancellationToken::new())),
        Err(ManagedRuntimeError::Invalid)
    ));
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn ambiguous_creation_keeps_exact_initial_receipt_and_never_reallocates_identity() {
    let f = FactoryFixture::new();
    let request = f.request("ambiguous");
    REJECT_PUBLICATION.with(|reject| reject.set(true));
    let ManagedPreparation::Ambiguous(mut receipt) =
        block_on(f.factory.prepare(request.clone(), CancellationToken::new())).unwrap()
    else {
        panic!("injected observation failure retains original receipt");
    };
    let stored = block_on(
        f.factory
            .0
            .services
            .session_store
            .load(request.transcript.session_id.clone()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.incarnation_id, request.transcript.incarnation);
    assert!(stored.messages.is_empty());
    let runtime = block_on(poll_fn(|cx| receipt.poll_reconcile(cx)))
        .unwrap()
        .unwrap();
    assert_eq!(runtime.runtime.id(), request.transcript.session_id);
    assert_eq!(
        runtime.runtime.incarnation_id(),
        request.transcript.incarnation
    );
    assert_eq!(runtime.runtime.record(), stored);
    assert!(matches!(
        block_on(poll_fn(|cx| receipt.poll_reconcile(cx))),
        Err(ManagedRuntimeError::Invalid)
    ));
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn turn_settlement_waits_exact_worker_and_close_does_not_wait_unrelated_host_work() {
    let f = FactoryFixture::new();
    let mut child = f.prepare(f.request("cleanup"));
    child.runtime.enqueue("standalone work".into()).unwrap();
    let turn = block_on(child.runtime.start_next(3)).unwrap().unwrap();
    let run = child.owner.run().unwrap();
    let cleanup = child.owner.cleanup_for(&run).unwrap();
    let workers = f.factory.0.services.control_workers.as_ref().unwrap();
    let (release, receiver) = std::sync::mpsc::channel();
    cleanup
        .with_poll(|| {
            workers.spawn(move || {
                let _ = receiver.recv();
            })
        })
        .unwrap();
    drop(turn);
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(
        child
            .resources
            .poll_turn_settled(&mut cx, &run)
            .is_pending()
    );
    release.send(()).unwrap();
    block_on(poll_fn(|cx| child.resources.poll_turn_settled(cx, &run))).unwrap();
    assert!(matches!(
        child.resources.poll_turn_settled(&mut cx, &run),
        Poll::Ready(Ok(()))
    ));
    if let Some((_, settlement)) = child.owner.take_settlement() {
        settlement.complete().unwrap();
    }
    let (unrelated, receiver) = std::sync::mpsc::channel();
    workers
        .spawn(move || {
            let _ = receiver.recv();
        })
        .unwrap();
    child.resources.begin_close();
    block_on(poll_fn(|cx| child.resources.poll_closed(cx))).unwrap();
    assert!(!workers.completion().is_complete());
    unrelated.send(()).unwrap();
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn child_uses_original_workspace_snapshot_not_later_parent_mutation() {
    let f = FactoryFixture::new();
    let request = f.request("workspace-child");
    let extra = f.host.workspace.parent().unwrap().join("later-extra");
    fs::create_dir(&extra).unwrap();
    let selected = f
        .parent
        .workspace()
        .prepare_blocking(
            vec![
                NativeWorkspaceEntrySpec::new(
                    NativeWorkspaceSource::new(extra.clone(), extra, true).unwrap(),
                    false,
                    true,
                )
                .unwrap(),
            ],
            false,
        )
        .unwrap();
    f.parent.workspace().install(selected).unwrap();
    let child = f.prepare(request);
    assert_eq!(f.parent.workspace().snapshot().unwrap().entries().len(), 1);
    assert!(
        child
            .owner
            .principal()
            .workspace()
            .snapshot()
            .unwrap()
            .entries()
            .is_empty()
    );
}
