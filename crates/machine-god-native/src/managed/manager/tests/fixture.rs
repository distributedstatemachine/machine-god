use super::*;
use crate::managed::{
    mailbox::{MailboxLimits, ManagedMailboxRequester},
    notices::NoticeLimits,
    principal::{NativePrincipalRegistry, NativePrincipalTurn},
    scheduler::{ManagedScheduler, SchedulerLimits},
};
use crate::{
    NativeConfiguredPermissionRules, NativeConversation, NativeModelPreferences,
    NativeOwnedWorkerScope, NativePermissionActionPreparer, NativePermissionController,
    NativePermissionPolicySnapshot, NativePreparedPermissionAction, NativeReasoningEffort,
    NativeUndoBudget, NativeWorkspaceAuthority, NativeWorkspaceContexts, PermissionMode,
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
};
use factory::*;
use futures_core::Stream;
use futures_executor::block_on;
use machine_god_core::*;
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub(super) struct Clock(pub Instant);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct NoEffects;
impl NativePermissionActionPreparer for NoEffects {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        panic!("test has no effects")
    }
}
impl PermissionPrompter for NoEffects {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        panic!("test has no permissions")
    }
}
pub(in crate::managed::manager) struct Factory {
    pub registry: NativePrincipalRegistry,
    pub scheduler: ManagedScheduler,
    pub provider: ScriptedModelProvider,
    pub workspace: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
    permissions: Arc<NativePermissionController>,
    engine: Engine,
    next: AtomicU64,
    pub prepared: AtomicU64,
    pub prepared_modes: Mutex<Vec<ManagedPermissionMode>>,
    pub cleanup: Arc<AtomicBool>,
    pub close_error: Arc<AtomicBool>,
    pub ambiguous: AtomicBool,
    pub reconcile: Arc<AtomicBool>,
    notices: Mutex<Option<Arc<super::super::super::notices::ManagedNotices>>>,
    notice_sessions: Mutex<Vec<Session>>,
}
impl ManagedRuntimeFactory for Arc<Factory> {
    fn allocate_identity(
        &self,
    ) -> BoxFuture<'static, Result<store::JournalTranscript, ManagedRuntimeError>> {
        let this = self.clone();
        Box::pin(async move {
            let id = this.next.fetch_add(1, Ordering::Relaxed);
            Ok(store::JournalTranscript {
                session_id: SessionId::new(format!("child-{id}")).unwrap(),
                incarnation: SessionIncarnationId::new("incarnation").unwrap(),
            })
        })
    }
    fn prepare(
        &self,
        request: ManagedRuntimeRequest,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<ManagedPreparation, ManagedRuntimeError>> {
        let this = self.clone();
        Box::pin(async move {
            this.prepared.fetch_add(1, Ordering::Relaxed);
            this.prepared_modes
                .lock()
                .unwrap()
                .push(request.configuration.permission_mode);
            let session = match request.kind {
                ManagedRuntimePreparationKind::Create => this
                    .engine
                    .create_session(
                        request.transcript.session_id,
                        request.transcript.incarnation,
                    )
                    .unwrap(),
                ManagedRuntimePreparationKind::Restore => this
                    .engine
                    .load_session(request.transcript.session_id)
                    .await
                    .unwrap()
                    .unwrap(),
            };
            if request.kind == ManagedRuntimePreparationKind::Create {
                let record = session.record();
                session
                    .update_metadata(record.revision, record.metadata)
                    .await
                    .unwrap();
            }
            let notices = this.notices.lock().unwrap().clone();
            let notice_context = notices.map(|notices| {
                this.notice_sessions.lock().unwrap().push(session.clone());
                Arc::new(
                    super::super::super::prompt_context::ParentNoticeContext::new(
                        &session,
                        super::super::super::notices::NoticePrincipal {
                            id: request.child_id.clone(),
                            generation: std::num::NonZeroU64::new(request.generation).unwrap(),
                        },
                        &notices,
                    ),
                )
            });
            let (conversation, owner) = NativeConversation::from_session(session)
                .unwrap()
                .with_permission_controller(
                    &this.permissions,
                    NativePermissionPolicySnapshot::new(
                        PermissionMode::Ask,
                        Arc::new(NativeConfiguredPermissionRules::default()),
                    ),
                )
                .unwrap()
                .with_managed_execution(
                    &this.registry,
                    this.scheduler.clone(),
                    request.generation,
                    &this.workspace,
                    &this.contexts,
                )
                .unwrap();
            let conversation = match &notice_context {
                Some(context) => conversation.with_notice_context(context).unwrap(),
                None => conversation,
            };
            let runtime = Arc::new(
                NativeConversationRuntime::new(conversation, preferences(), None).unwrap(),
            );
            let prepared = PreparedManagedRuntime {
                skills: None,
                notice_context,
                runtime,
                owner,
                resources: Box::new(Resources {
                    ready: this.cleanup.clone(),
                    close_error: this.close_error.clone(),
                    _owner: request.journal_owner,
                }),
            };
            if this.ambiguous.load(Ordering::Acquire) {
                Ok(ManagedPreparation::Ambiguous(Box::new(Receipt {
                    ready: this.reconcile.clone(),
                    prepared: Some(prepared),
                })))
            } else {
                Ok(ManagedPreparation::Ready(prepared))
            }
        })
    }
}
struct Receipt {
    ready: Arc<AtomicBool>,
    prepared: Option<PreparedManagedRuntime>,
}
impl ManagedPreparationReceipt for Receipt {
    fn poll_reconcile(
        &mut self,
        _: &mut Context<'_>,
    ) -> Poll<Result<Option<PreparedManagedRuntime>, ManagedRuntimeError>> {
        if self.ready.load(Ordering::Acquire) {
            Poll::Ready(Ok(self.prepared.take()))
        } else {
            Poll::Pending
        }
    }
}
struct Resources {
    ready: Arc<AtomicBool>,
    close_error: Arc<AtomicBool>,
    _owner: JournalOwner,
}
impl ManagedRuntimeResources for Resources {
    fn poll_admission_settled(
        &mut self,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), ManagedRuntimeError>> {
        if self.ready.load(Ordering::Acquire) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    fn poll_turn_settled(
        &mut self,
        _: &mut Context<'_>,
        _: &RunRef,
    ) -> Poll<Result<(), ManagedRuntimeError>> {
        if self.ready.load(Ordering::Acquire) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    fn begin_close(&mut self) {}
    fn poll_closed(&mut self, _: &mut Context<'_>) -> Poll<Result<(), ManagedRuntimeError>> {
        if self.close_error.load(Ordering::Acquire) {
            return Poll::Ready(Err(ManagedRuntimeError::Unavailable));
        }
        if self.ready.load(Ordering::Acquire) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}
pub(super) struct Authorizer;
impl ManagedRelationshipAuthorizer for Authorizer {
    fn authorize(
        &self,
        _: ManagedRelationshipProposal,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<bool, ManagedRuntimeError>> {
        Box::pin(async { Ok(true) })
    }
}
struct Capture(Arc<Mutex<Option<ManagedSubagentInvocation>>>);
impl ManagedSubagentAuthority for Capture {
    fn execute(
        &self,
        invocation: ManagedSubagentInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ManagedSubagentResult, ManagedSubagentError>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(invocation);
            std::future::pending().await
        })
    }
}
pub(super) struct Admission {
    _guard: NativePrincipalTurn,
    _turn: Turn,
    _session: Session,
    _engine: Engine,
}
pub(in crate::managed::manager) struct Fixture {
    pub factory: Arc<Factory>,
    pub journal: ManagedJournal,
    pub requester: ManagedMailboxRequester,
    pub manager: ManagedManager,
    path: PathBuf,
    workers: NativeOwnedWorkerScope,
    #[cfg(feature = "ai-gateway-http")]
    archive: Arc<crate::NativeToolResultArchiveAdapter>,
}
pub(super) fn preferences() -> NativeModelPreferences {
    NativeModelPreferences::new("model", NativeReasoningEffort::default(), false).unwrap()
}
impl Fixture {
    pub fn enable_child_notices(&self) {
        *self.factory.notices.lock().unwrap() = Some(self.manager.notices.clone());
    }
    pub fn actual_notice_session(&self, id: &str) -> Session {
        self.factory
            .notice_sessions
            .lock()
            .unwrap()
            .iter()
            .find(|session| session.id().as_str() == id)
            .unwrap()
            .clone()
    }
    pub fn child_session(&self, id: &str) -> Session {
        block_on(
            self.factory
                .engine
                .load_session(SessionId::new(id).unwrap()),
        )
        .unwrap()
        .unwrap()
    }

    pub fn notified_foreground(&self, session: Session) -> PreparedManagedRuntime {
        self.notified_foreground_with_staging(session, false)
    }

    pub fn staged_notified_foreground(&self, session: Session) -> PreparedManagedRuntime {
        self.notified_foreground_with_staging(session, true)
    }

    fn notified_foreground_with_staging(
        &self,
        session: Session,
        staged: bool,
    ) -> PreparedManagedRuntime {
        use crate::managed::{notices::NoticePrincipal, prompt_context::ParentNoticeContext};
        let context = Arc::new(ParentNoticeContext::new(
            &session,
            NoticePrincipal {
                id: session.id().to_string(),
                generation: std::num::NonZeroU64::new(1).unwrap(),
            },
            &self.manager.notices,
        ));
        let conversation = NativeConversation::from_session(session).unwrap();
        let conversation = if staged {
            conversation.with_staged_routes().unwrap()
        } else {
            conversation
        };
        let (conversation, owner) = conversation
            .with_permission_controller(
                &self.factory.permissions,
                NativePermissionPolicySnapshot::new(
                    PermissionMode::Ask,
                    Arc::new(NativeConfiguredPermissionRules::default()),
                ),
            )
            .unwrap()
            .with_managed_execution(
                &self.factory.registry,
                self.factory.scheduler.clone(),
                1,
                &self.factory.workspace,
                &self.factory.contexts,
            )
            .unwrap();
        let conversation = conversation.with_notice_context(&context).unwrap();
        PreparedManagedRuntime {
            skills: None,
            runtime: Arc::new(
                NativeConversationRuntime::new(conversation, preferences(), None).unwrap(),
            ),
            owner,
            resources: Box::new(Resources {
                ready: self.factory.cleanup.clone(),
                _owner: self.journal.owner_lease(),
            }),
            notice_context: Some(context),
        }
    }
    pub fn notice_session(&self) -> Session {
        self.factory
            .engine
            .create_session(
                SessionId::new("notice-parent").unwrap(),
                SessionIncarnationId::new("notice-incarnation").unwrap(),
            )
            .unwrap()
    }
    pub fn new(steps: Vec<ModelProviderStep>) -> Self {
        Self::with_store(steps, InMemorySessionStore::default())
    }
    pub fn with_store(steps: Vec<ModelProviderStep>, store: impl SessionStore) -> Self {
        Self::with_engine_setup(steps, store, |engine| engine)
    }
    pub fn with_engine_setup(
        steps: Vec<ModelProviderStep>,
        store: impl SessionStore,
        configure: impl FnOnce(EngineBuilder) -> EngineBuilder,
    ) -> Self {
        Self::with_engine_and_journal(steps, store, configure, store::JournalLimits::default())
    }
    pub fn with_journal_limits(
        steps: Vec<ModelProviderStep>,
        limits: store::JournalLimits,
    ) -> Self {
        Self::with_engine_and_journal(
            steps,
            InMemorySessionStore::default(),
            |engine| engine,
            limits,
        )
    }
    fn with_engine_and_journal(
        steps: Vec<ModelProviderStep>,
        store: impl SessionStore,
        configure: impl FnOnce(EngineBuilder) -> EngineBuilder,
        limits: store::JournalLimits,
    ) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "mg-managed-manager-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        for name in ["workspace", "state", "journal", "archive"] {
            std::fs::create_dir(path.join(name)).unwrap();
            std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let workspace = NativeWorkspaceAuthority::open_blocking(
            std::fs::File::open(path.join("workspace")).unwrap().into(),
            path.join("workspace"),
            Some(std::fs::File::open(path.join("state")).unwrap().into()),
            path.join("state"),
            vec![],
            false,
        )
        .unwrap();
        let provider = ScriptedModelProvider::new("test", steps);
        let permissions = Arc::new(NativePermissionController::new(
            Arc::new(NoEffects),
            Arc::new(NoEffects),
        ));
        let engine = configure(
            Engine::builder()
                .provider(provider.clone())
                .shared_permission_handler(permissions.clone())
                .session_store(store),
        )
        .build()
        .unwrap();
        let factory = Arc::new(Factory {
            registry: NativePrincipalRegistry::new(64, Arc::new(NativeUndoBudget::default()))
                .unwrap(),
            scheduler: ManagedScheduler::new(SchedulerLimits::new(4, 64, 64).unwrap()),
            provider,
            workspace,
            contexts: Arc::new(NativeWorkspaceContexts::new()),
            permissions,
            engine,
            next: AtomicU64::new(1),
            prepared: AtomicU64::new(0),
            prepared_modes: Mutex::default(),
            cleanup: Arc::new(AtomicBool::new(true)),
            close_error: Arc::new(AtomicBool::new(false)),
            ambiguous: AtomicBool::new(false),
            reconcile: Arc::new(AtomicBool::new(false)),
            notices: Mutex::default(),
            notice_sessions: Mutex::default(),
        });
        let workers = NativeOwnedWorkerScope::new();
        let journal = block_on(ManagedJournal::open(
            std::fs::File::open(path.join("journal")).unwrap().into(),
            workers.clone(),
            limits,
        ))
        .unwrap();
        let mailbox =
            ManagedMailbox::new(factory.registry.requester(), MailboxLimits::default()).unwrap();
        let requester = mailbox.requester();
        let clock = Arc::new(Clock(Instant::now()));
        let notices = Arc::new(
            super::super::super::notices::ManagedNotices::new(
                NoticeLimits::default(),
                clock.clone(),
            )
            .unwrap(),
        );
        let manager = ManagedManager::new(
            journal.clone(),
            mailbox,
            Arc::new(factory.clone()),
            Arc::new(Authorizer),
            notices,
            clock,
            ManagerLimits::default(),
        )
        .unwrap();
        Self {
            #[cfg(feature = "ai-gateway-http")]
            archive: Arc::new(
                crate::NativeToolResultArchiveAdapter::new(Arc::new(
                    crate::ToolResultArchive::from_root_descriptor(
                        std::fs::File::open(path.join("archive")).unwrap().into(),
                    ),
                ))
                .with_worker_scope(workers.clone()),
            ),
            factory,
            journal,
            requester,
            manager,
            path,
            workers,
        }
    }
    pub(super) fn invocation(
        &self,
        command: serde_json::Value,
    ) -> (Admission, ManagedSubagentInvocation) {
        let arguments =
            serde_json::Value::Object(serde_json::Map::from_iter([("command".into(), command)]));
        ManagedSubagentCommand::decode(arguments.clone()).expect("valid fixture command");
        let captured = Arc::new(Mutex::new(None));
        #[cfg(feature = "ai-gateway-http")]
        let tool = crate::reference_host::subagent::NativeManagedSubagentTool::new(
            Arc::new(Capture(captured.clone())),
            self.archive.clone(),
        );
        #[cfg(not(feature = "ai-gateway-http"))]
        let tool = SubagentTool::new(Capture(captured.clone()));
        let engine = Engine::builder()
            .limits(EngineLimits {
                max_cumulative_complete_tool_argument_bytes: std::num::NonZeroUsize::new(
                    MAX_SUBAGENT_ARGUMENT_BYTES,
                )
                .unwrap(),
                max_cumulative_complete_tool_result_bytes: std::num::NonZeroUsize::new(
                    MAX_SUBAGENT_OUTPUT_BYTES + 32,
                )
                .unwrap(),
                ..EngineLimits::default()
            })
            .provider(ScriptedModelProvider::new(
                "caller",
                [ModelProviderStep::events([
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new("operation").unwrap(),
                            name: ToolName::new("subagent").unwrap(),
                            arguments,
                        },
                    },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ])],
            ))
            .permission_handler(ScriptedPermissionHandler::new([PermissionStep::Decision(
                PermissionDecision::Allow {
                    scope: PermissionGrantScope::Once,
                },
            )]))
            .session_store(InMemorySessionStore::default())
            .tool(tool)
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("parent").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        let principal = self
            .factory
            .registry
            .register(&session, 1, &self.factory.workspace)
            .unwrap();
        let mut turn = block_on(session.prompt("control")).unwrap();
        let guard = principal
            .begin_turn(
                &turn,
                NativePermissionPolicySnapshot::new(
                    PermissionMode::Ask,
                    Arc::new(NativeConfiguredPermissionRules::default()),
                ),
                preferences(),
                None,
            )
            .unwrap();
        let invocation = capture_invocation(&mut turn, &captured);
        (
            Admission {
                _guard: guard,
                _turn: turn,
                _session: session,
                _engine: engine,
            },
            invocation,
        )
    }
    pub fn drive(&mut self, predicate: impl Fn(&Self) -> bool) {
        block_on(std::future::poll_fn(|cx| {
            let progress = self.manager.poll_progress(cx, 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            if predicate(self) {
                return Poll::Ready(());
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }));
    }
    pub fn command(&mut self, command: serde_json::Value) -> ManagedSubagentResult {
        let (_admission, invocation) = self.invocation(command);
        let requester = self.requester.clone();
        let mut response = requester.execute(invocation, CancellationToken::new());
        block_on(std::future::poll_fn(|cx| {
            if let Poll::Ready(result) = response.as_mut().poll(cx) {
                return Poll::Ready(result.unwrap());
            }
            let progress = self.manager.poll_progress(cx, 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }))
    }
    pub fn restart_manager(&mut self) {
        self.manager.request_shutdown();
        block_on(std::future::poll_fn(|cx| {
            self.manager.poll_shutdown(cx, 101)
        }))
        .unwrap();
        let mailbox =
            ManagedMailbox::new(self.factory.registry.requester(), MailboxLimits::default())
                .unwrap();
        self.requester = mailbox.requester();
        let clock = self.manager.clock.clone();
        let notices = Arc::new(
            super::super::super::notices::ManagedNotices::new(
                NoticeLimits::default(),
                clock.clone(),
            )
            .unwrap(),
        );
        self.manager = ManagedManager::new(
            self.journal.clone(),
            mailbox,
            Arc::new(self.factory.clone()),
            Arc::new(Authorizer),
            notices,
            clock,
            ManagerLimits::default(),
        )
        .unwrap();
    }

    pub fn restart_journal_owner(&mut self) {
        self.manager.request_shutdown();
        block_on(std::future::poll_fn(|cx| {
            self.manager.poll_shutdown(cx, 101)
        }))
        .unwrap();
        // An empty stand-in lets the fixture release every original manager
        // and journal lease before reopening the same actual directory.
        let standby = self.path.join("journal-standby");
        std::fs::create_dir(&standby).unwrap();
        std::fs::set_permissions(&standby, std::fs::Permissions::from_mode(0o700)).unwrap();
        let standby = block_on(ManagedJournal::open(
            std::fs::File::open(standby).unwrap().into(),
            self.workers.clone(),
            store::JournalLimits::default(),
        ))
        .unwrap();
        let old = std::mem::replace(&mut self.journal, standby);
        self.restart_manager();
        drop(old);
        self.journal = block_on(ManagedJournal::open(
            std::fs::File::open(self.path.join("journal"))
                .unwrap()
                .into(),
            self.workers.clone(),
            store::JournalLimits::default(),
        ))
        .unwrap();
        self.restart_manager();
    }
}

fn capture_invocation(
    turn: &mut Turn,
    captured: &Mutex<Option<ManagedSubagentInvocation>>,
) -> ManagedSubagentInvocation {
    block_on(std::future::poll_fn(|cx| {
        let event = Pin::new(&mut *turn).poll_next(cx);
        if let Poll::Ready(Some(Ok(EngineEvent {
            payload:
                TurnEvent::Failed {
                    component,
                    code,
                    message,
                    ..
                },
            ..
        }))) = &event
        {
            panic!("fixture invocation failed in {component}: {code}: {message}");
        }
        if let Poll::Ready(Some(Err(error))) = &event {
            panic!("fixture invocation stream failed: {error}");
        }
        if let Some(invocation) = captured.lock().unwrap().take() {
            return Poll::Ready(invocation);
        }
        assert!(
            !matches!(event, Poll::Ready(None)),
            "fixture turn ended before actual managed invocation"
        );
        if event.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }))
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.factory.cleanup.store(true, Ordering::Release);
        self.factory.close_error.store(false, Ordering::Release);
        self.factory.reconcile.store(true, Ordering::Release);
        self.manager.request_shutdown();
        block_on(std::future::poll_fn(|cx| {
            self.manager.poll_shutdown(cx, 101)
        }))
        .unwrap();
        self.workers.close();
        block_on(self.workers.completion().wait());
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
