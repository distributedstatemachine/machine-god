use super::*;
use crate::{NativeWorkspaceAction as Action, NativeWorkspaceReconciliation as Reconciliation};

fn store_path(fixture: &Fixture) -> std::path::PathBuf {
    fixture
        .workspace
        .parent()
        .unwrap()
        .join("workspace-settings")
}
fn config_store(fixture: &Fixture) -> Arc<NativeUserConfigStore> {
    Arc::new(NativeUserConfigStore::new(store_path(fixture)))
}

async fn apply(
    session: &mut NativeInteractiveSession,
    store: &Arc<NativeUserConfigStore>,
    action: Action,
) -> crate::NativeWorkspaceReceipt {
    session
        .request_control(
            Control::Workspace {
                action,
                store: store.clone(),
            },
            200,
        )
        .unwrap();
    let result = control_outcome(session).await;
    assert!(!result.failed());
    let Receipt::Workspace(receipt) = result.result.unwrap() else {
        panic!("workspace receipt")
    };
    receipt
}

#[test]
fn workspace_control_is_inert_owned_and_shared_by_actual_tools_across_new_session() {
    executor().block_on(async {
        let fixture = Fixture::new_with_workspace();
        let store = config_store(&fixture);
        let shared = fixture.workspace.parent().unwrap().join("shared one");
        std::fs::create_dir(&shared).unwrap();
        let mut session = owner(&fixture).await;
        let original = session.runtime().clone();
        let record = original.record();
        let source = crate::interactive_session::transition::principal(&original);
        session
            .request_control(
                Control::Workspace {
                    action: Action::Add(shared.clone()),
                    store: store.clone(),
                },
                200,
            )
            .unwrap();
        assert!(!store_path(&fixture).exists());
        assert_eq!(original.record(), record);
        let receipt = control_outcome(&mut session).await;
        assert_eq!(receipt.source, source);
        let Receipt::Workspace(receipt) = receipt.result.unwrap() else {
            panic!("workspace receipt")
        };
        assert_eq!(receipt.saved_changed, Some(true));
        assert_eq!(receipt.runtime_changed, Some(true));
        assert!(receipt.snapshot.entries()[0].active());
        assert_eq!(
            original.record(),
            record,
            "workspace selection is not session metadata"
        );

        session
            .request_transition(NativeInteractiveTransition::New, 300)
            .unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert!(!Arc::ptr_eq(&original, session.runtime()));
        let path = shared.join("actual.txt");
        fixture.transport.push(support::call(
            "write_file",
            &serde_json::json!({"path":path,"content":"workspace-backed"}),
        ));
        fixture.transport.push(support::answer());
        session
            .enqueue("write into the selected additional directory".into())
            .unwrap();
        finish_turn(&mut session).await;
        assert_eq!(std::fs::read(&path).unwrap(), b"workspace-backed");
        let removed = apply(&mut session, &store, Action::Clear).await;
        assert!(removed.snapshot.entries().is_empty());
        assert!(
            receipt.snapshot.route(&path).is_ok(),
            "old receipt retains original descriptors"
        );
        assert!(removed.snapshot.route(&path).is_err());
        drop(original);
        close(session, fixture).await;
    });
}

#[test]
fn workspace_busy_list_is_cached_and_mutation_does_not_consume_queued_prompt() {
    executor().block_on(async {
        let fixture = Fixture::new_with_workspace();
        let store = config_store(&fixture);
        let mut session = owner(&fixture).await;
        let queued = session.runtime().enqueue("held prompt".into()).unwrap();
        let receipt = apply(&mut session, &store, Action::List).await;
        assert_eq!(receipt.reconciliation, Reconciliation::CachedBusy);
        assert!(!store_path(&fixture).exists());
        session
            .request_control(
                Control::Workspace {
                    action: Action::Clear,
                    store: store.clone(),
                },
                200,
            )
            .unwrap();
        assert!(matches!(
            control_outcome(&mut session).await.result,
            Err(ControlError::Workspace(
                crate::NativeWorkspaceServiceError::Busy
            ))
        ));
        assert!(session.runtime().cancel_queued(queued));
        assert!(!store_path(&fixture).exists());
        close(session, fixture).await;
    });
}

#[test]
fn workspace_failed_control_prevents_pending_transition_and_legacy_host_rejects() {
    executor().block_on(async {
        let fixture = Fixture::new_with_workspace();
        let store = config_store(&fixture);
        let mut session = owner(&fixture).await;
        let before = session.runtime().clone();
        for path in [String::new(), "a\0b".to_owned(), "x".repeat(4097)] {
            assert!(matches!(
                session.request_control(
                    Control::Workspace {
                        action: Action::Add(path.into()),
                        store: store.clone()
                    },
                    200
                ),
                Err(NativeInteractiveError::Configuration)
            ));
            assert!(session.take_control_outcome().is_none());
        }
        session
            .request_control(
                Control::Workspace {
                    action: Action::Add("missing-root".into()),
                    store: store.clone(),
                },
                200,
            )
            .unwrap();
        session
            .request_transition(NativeInteractiveTransition::New, 210)
            .unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Rejected {
                error: NativeInteractiveError::ControlFailed,
                ..
            }
        ));
        assert!(session.take_control_outcome().unwrap().failed());
        assert!(Arc::ptr_eq(&before, session.runtime()));
        assert!(!store_path(&fixture).exists());
        drop(before);
        close(session, fixture).await;

        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        assert!(matches!(
            session.request_control(
                Control::Workspace {
                    action: Action::List,
                    store: config_store(&fixture)
                },
                200
            ),
            Err(NativeInteractiveError::Configuration)
        ));
        close(session, fixture).await;
    });
}
