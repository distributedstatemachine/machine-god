use super::*;
use crate::{
    NativeAllowlistReceipt as AllowReceipt, NativeAllowlistReloadError, NativeAllowlistView,
    NativeConfiguredPermissionMutation as Mutation,
    NativeConfiguredPermissionMutationOutcome as MutationOutcome,
    NativeConfiguredPermissionScope as Scope,
};

async fn apply(
    owner: &mut NativeInteractiveSession,
    store: &Arc<NativeUserConfigStore>,
    raw: &str,
) -> AllowReceipt {
    let request = owner.parse_allowlist(raw).unwrap();
    let id = owner
        .request_control(
            Control::Allowlist {
                request,
                store: store.clone(),
            },
            200,
        )
        .unwrap();
    let outcome = control_outcome(owner).await;
    assert_eq!(id, outcome.id);
    let Receipt::Allowlist(receipt) = outcome.result.unwrap() else {
        panic!("allowlist receipt")
    };
    receipt
}

#[test]
fn allowlist_real_store_sources_normalization_views_and_noop_remove_match_runtime() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let runtime = session.runtime().clone();
        let before = runtime.record();
        let permission = runtime.permissions().unwrap();
        permission.set_mode(PermissionMode::Auto).unwrap();
        permission
            .set_sandbox_mode(crate::NativeSandboxMode::Os)
            .unwrap();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("allowlist-config"),
        ));
        let request = session
            .parse_allowlist("USER ADD command \" \r\ngit *\r\n \" trailing")
            .unwrap();
        session
            .request_control(
                Control::Allowlist {
                    request,
                    store: store.clone(),
                },
                200,
            )
            .unwrap();
        assert!(
            !fixture.workspace.join("allowlist-config").exists(),
            "admission is inert"
        );
        assert!(session.request_control(Control::UndoLast, 201).is_err());
        let first = control_outcome(&mut session).await;
        assert!(matches!(
            first.result,
            Ok(Receipt::Allowlist(AllowReceipt::Mutation {
                reload: Some(Ok(())),
                ..
            }))
        ));
        let disk = store.load().unwrap();
        assert_eq!(
            disk.loaded().config().permission_rules().rules()[0].pattern(),
            "git *"
        );
        let old = permission.snapshot().unwrap();
        assert_eq!(old.configured_rules().rules()[0].permission(), "bash");
        let AllowReceipt::Mutation {
            sources: Some(sources),
            reload: Some(Ok(())),
            ..
        } = apply(&mut session, &store, "add tool read_file").await
        else {
            panic!("local receipt")
        };
        assert!(sources.user_shadowed_by_local());
        assert_eq!(
            permission.snapshot().unwrap().configured_rules().rules()[0].permission(),
            "read"
        );
        assert_eq!(old.configured_rules().rules()[0].permission(), "bash");
        permission.set_configured_rules(Arc::default()).unwrap();
        let AllowReceipt::Mutation {
            sources: None,
            reload: None,
            outcome: MutationOutcome::Unchanged,
            ..
        } = apply(&mut session, &store, "user remove command not-present").await
        else {
            panic!("unchanged remove skips reload")
        };
        assert!(
            permission
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()
                .is_empty()
        );
        let AllowReceipt::View {
            view: NativeAllowlistView::User,
            sources,
            reload: Ok(()),
        } = apply(&mut session, &store, "view user").await
        else {
            panic!("view user")
        };
        assert_eq!(sources.user().rules()[0].permission(), "bash");
        assert_eq!(
            permission.snapshot().unwrap().configured_rules().rules()[0].permission(),
            "read",
            "view user still reloads effective local policy"
        );
        let _ = apply(&mut session, &store, "remove tool read_file").await;
        let AllowReceipt::View { sources, .. } = apply(&mut session, &store, "view local").await
        else {
            panic!("view")
        };
        assert!(sources.local().unwrap().rules().is_empty());
        assert!(
            permission
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()
                .is_empty()
        );
        let _ = apply(&mut session, &store, "reset all").await;
        assert!(
            store
                .load()
                .unwrap()
                .loaded()
                .config()
                .permission_sources(&fixture.workspace)
                .unwrap()
                .local()
                .is_some(),
            "zero removal retains empty shadow"
        );
        let _ = apply(&mut session, &store, "add tool read_file").await;
        let _ = apply(&mut session, &store, "reset tools").await;
        assert_eq!(
            permission.snapshot().unwrap().configured_rules().rules()[0].permission(),
            "bash",
            "actual removal restores user source"
        );
        let _ = apply(&mut session, &store, "user add command \" \t\r\n \"").await;
        assert!(
            store
                .load()
                .unwrap()
                .loaded()
                .config()
                .permission_rules()
                .rules()
                .iter()
                .any(|rule| rule.pattern().is_empty())
        );
        assert_eq!(permission.snapshot().unwrap().mode(), PermissionMode::Auto);
        assert_eq!(
            permission.snapshot().unwrap().sandbox_mode(),
            crate::NativeSandboxMode::Os
        );
        assert_eq!(runtime.record(), before);
        drop(runtime);
        close(session, fixture).await;
    });
}

#[test]
fn allowlist_post_commit_reload_is_fresh_and_failure_preserves_durable_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let session = owner(&fixture).await;
        let directory = fixture.workspace.join("allowlist-config");
        let store = Arc::new(NativeUserConfigStore::new(directory.clone()));
        let hook_store = store.clone();
        let workspace = fixture.workspace.clone();
        let hook: crate::allowlist::service::Hook = Arc::new(move |stage| {
            if stage == crate::allowlist::service::Stage::AfterCommit {
                let snapshot = hook_store.load().unwrap();
                futures_executor::block_on(hook_store.apply_permission_mutation(
                    &snapshot,
                    &workspace,
                    Scope::User,
                    &Mutation::Add {
                        permission: "read".into(),
                        pattern: "after-primary-commit".into(),
                    },
                ))
                .unwrap();
            }
        });
        let request = session.parse_allowlist("user add command git *").unwrap();
        let result = crate::allowlist::service::execute_inner(
            session.runtime().clone(),
            store.clone(),
            fixture.workspace.clone(),
            fixture.host.control_workers().unwrap(),
            request,
            Some(hook),
        )
        .await
        .unwrap();
        let AllowReceipt::Mutation {
            sources: Some(sources),
            reload: Some(Ok(())),
            ..
        } = result
        else {
            panic!("fresh receipt")
        };
        assert_eq!(sources.user().rules().len(), 2);
        assert_eq!(
            session
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()
                .len(),
            2
        );

        let hook_store = store.clone();
        let corrupted = directory.join("config.json");
        let hook: crate::allowlist::service::Hook = Arc::new(move |stage| {
            if stage == crate::allowlist::service::Stage::AfterCommit {
                assert!(
                    hook_store
                        .load()
                        .unwrap()
                        .loaded()
                        .config()
                        .permission_rules()
                        .rules()
                        .iter()
                        .any(|rule| rule.pattern() == "committed-before-reload")
                );
                std::fs::write(&corrupted, b"malformed-after-publication").unwrap();
            }
        });
        let request = session
            .parse_allowlist("user add command committed-before-reload")
            .unwrap();
        let result = crate::allowlist::service::execute_inner(
            session.runtime().clone(),
            store,
            fixture.workspace.clone(),
            fixture.host.control_workers().unwrap(),
            request,
            Some(hook),
        )
        .await
        .unwrap();
        assert!(matches!(
            result,
            AllowReceipt::Mutation {
                outcome: MutationOutcome::Changed { .. },
                sources: None,
                reload: Some(Err(NativeAllowlistReloadError::Config(_))),
                ..
            }
        ));
        assert!(result.failed());
        assert_eq!(
            session
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()
                .len(),
            2,
            "failed reload leaves previous policy"
        );
        close(session, fixture).await;
    });
}

#[test]
fn allowlist_dropped_response_keeps_effect_permit_and_actual_host_thread_join() {
    struct ThreadCleanup {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Drop for ThreadCleanup {
        fn drop(&mut self) {
            let _ = self.entered.send(());
            let _ = self
                .release
                .recv_timeout(std::time::Duration::from_secs(10));
        }
    }
    thread_local! {
        static CLEANUP: std::cell::RefCell<Option<ThreadCleanup>> = const { std::cell::RefCell::new(None) };
    }
    executor().block_on(async {
        let fixture = Fixture::new();
        let session = owner(&fixture).await;
        let runtime = session.runtime().clone();
        let completion = fixture.host.terminal_shutdown_completion().unwrap();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("allowlist-config"),
        ));
        let (entered, observing) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let waiting = std::sync::Mutex::new(waiting);
        let (tls_entered, tls_observing) = std::sync::mpsc::channel();
        let (tls_release, tls_waiting) = std::sync::mpsc::channel();
        let cleanup = std::sync::Mutex::new(Some(ThreadCleanup {
            entered: tls_entered,
            release: tls_waiting,
        }));
        let hook: crate::allowlist::service::Hook = Arc::new(move |stage| {
            if stage == crate::allowlist::service::Stage::BeforeLoad {
                CLEANUP.with(|slot| *slot.borrow_mut() = cleanup.lock().unwrap().take());
                entered.send(()).unwrap();
                waiting
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
            }
        });
        let request = session
            .parse_allowlist("user add command retained-worker-effect")
            .unwrap();
        let mut response = crate::allowlist::service::execute_inner(
            runtime.clone(),
            store.clone(),
            fixture.workspace.clone(),
            fixture.host.control_workers().unwrap(),
            request,
            Some(hook),
        );
        assert!(
            response
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        observing
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let mut quiescence = runtime.begin_quiescence().unwrap();
        let mut idle = quiescence.wait_idle();
        assert!(
            idle.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(response);
        assert!(
            idle.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert!(!fixture.workspace.join("allowlist-config").exists());
        release.send(()).unwrap();
        tls_observing
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert!(
            idle.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_ready()
        );
        drop(idle);
        drop(quiescence);
        assert_eq!(
            store
                .load()
                .unwrap()
                .loaded()
                .config()
                .permission_rules()
                .rules()[0]
                .pattern(),
            "retained-worker-effect"
        );
        assert_eq!(
            runtime
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()[0]
                .pattern(),
            "retained-worker-effect"
        );
        drop(runtime);
        drop(session);
        drop(fixture);
        assert!(
            !completion.is_complete(),
            "job result/admission release is not TLS join"
        );
        tls_release.send(()).unwrap();
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
    });
}

#[test]
fn allowlist_runs_during_active_generation_and_retains_receipt_behind_output() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        fixture.transport.push(support::answer());
        session.enqueue("active".into()).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 200);
                if session.presentation.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(session.runtime().status().active);
        let source = session.runtime().id();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("allowlist-config"),
        ));
        let request = session.parse_allowlist("user add tool read_file").unwrap();
        session
            .request_control(Control::Allowlist { request, store }, 201)
            .unwrap();
        assert!(session.request_control(Control::UndoLast, 202).is_err());
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 203);
                if session.control_outcome.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(session.presentation.is_some());
        assert!(session.runtime().status().active);
        assert_eq!(session.runtime().id(), source);
        assert!(matches!(
            &session.control_outcome.as_ref().unwrap().result,
            Ok(Receipt::Allowlist(AllowReceipt::Mutation {
                reload: Some(Ok(())),
                ..
            }))
        ));
        assert!(
            session.request_control(Control::UndoLast, 204).is_err(),
            "unconsumed receipt owns control slot"
        );
        session.request_shutdown();
        while !session.is_closed() {
            let _ = outcome(&mut session).await;
        }
        assert!(
            session.take_control_outcome().is_some(),
            "shutdown retains unacknowledged receipt"
        );
        drop(session);
        fixture.finish();
    });
}

#[test]
fn allowlist_reload_failure_rejects_pending_transition_without_losing_saved_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let current = session.runtime().clone();
        let directory = fixture.workspace.join("allowlist-config");
        let store = Arc::new(NativeUserConfigStore::new(directory.clone()));
        let request = session.parse_allowlist("user add tool read_file").unwrap();
        session.request_control(Control::Allowlist { request: request.clone(), store: store.clone() }, 201).unwrap();
        let hook: crate::allowlist::service::Hook = Arc::new(move |stage| {
            if stage == crate::allowlist::service::Stage::AfterCommit {
                std::fs::write(directory.join("config.json"), b"invalid-after-confirmed-commit").unwrap();
            }
        });
        let future = crate::allowlist::service::execute_inner(current.clone(), store, fixture.workspace.clone(), fixture.host.control_workers().unwrap(), request, Some(hook));
        session.control.as_mut().unwrap().future = Box::pin(async move {
            future.await.map(Receipt::Allowlist).map_err(ControlError::Allowlist)
        });
        let transition = session.request_transition(NativeInteractiveTransition::New, 202).unwrap();
        let control = control_outcome(&mut session).await;
        assert!(matches!(control.result, Ok(Receipt::Allowlist(AllowReceipt::Mutation { outcome: MutationOutcome::Changed { .. }, reload: Some(Err(_)), .. }))));
        assert!(matches!(session.take_outcome(), Some(NativeInteractiveOutcome::Rejected { request, error: NativeInteractiveError::ControlFailed, .. }) if request == transition.id));
        assert!(Arc::ptr_eq(session.runtime(), &current));
        drop(current);
        close(session, fixture).await;
    });
}

#[test]
fn allowlist_revalidates_host_registry_and_reports_real_config_lock_busy_without_retry() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let directory = fixture.workspace.join("allowlist-config");
        let store = Arc::new(NativeUserConfigStore::new(directory.clone()));
        let foreign = crate::allowlist::parse("add tool foreign_registry_only", |_| true).unwrap();
        assert!(matches!(
            session.request_control(
                Control::Allowlist {
                    request: foreign,
                    store: store.clone()
                },
                200
            ),
            Err(NativeInteractiveError::Configuration)
        ));
        assert!(!directory.exists());
        drop(crate::allowlist::service::execute(
            session.runtime().clone(),
            store.clone(),
            fixture.workspace.clone(),
            fixture.host.control_workers().unwrap(),
            session
                .parse_allowlist("user add command unpolled")
                .unwrap(),
        ));
        assert!(
            !directory.exists(),
            "unpolled service has no filesystem effects"
        );
        let _ = apply(&mut session, &store, "user add command original").await;
        let before = std::fs::read(directory.join("config.json")).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.join(".config.lock"))
            .unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
        let request = session
            .parse_allowlist("user add command rejected-busy")
            .unwrap();
        session
            .request_control(
                Control::Allowlist {
                    request,
                    store: store.clone(),
                },
                201,
            )
            .unwrap();
        let result = control_outcome(&mut session).await;
        assert!(matches!(
            result.result,
            Err(ControlError::Allowlist(
                crate::NativeAllowlistError::Config(crate::NativeUserConfigError::Busy)
            ))
        ));
        assert_eq!(
            std::fs::read(directory.join("config.json")).unwrap(),
            before
        );
        assert_eq!(
            session
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()
                .len(),
            1
        );
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::Unlock).unwrap();
        drop(lock);
        let _ = apply(&mut session, &store, "user add command explicit-retry").await;
        assert_eq!(
            session
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .configured_rules()
                .rules()
                .len(),
            2
        );
        close(session, fixture).await;
    });
}
