use super::super::{Driver, OutputBridge, support};
use super::{Submission, submission};
use machine_god_core::CancellationToken;
use machine_god_native as native;
use native::{
    NativeInteractiveControlReceipt, NativeInteractiveInitialSession, NativeInteractiveInput,
    NativeInteractiveInputSource, NativeInteractivePromptBridge, NativeInteractivePromptLimits,
    NativeInteractiveSession, NativeInteractiveSessionOptions, NativeModelPreferencePersistence,
    NativeModelPreferences, NativeReasoningEffort, NativeSandboxMode, NativeSlashCommand,
    NativeUserConfigStore, PermissionMode,
};
use std::{future::poll_fn, sync::Arc, task::Poll, time::Duration};

fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn workspace_without_settings_lists_but_cannot_save() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_workspace();
        let mut driver = driver(&fixture).await;
        assert!(driver.user_config.is_none());
        driver.command("/workspace list", 200);
        let receipt = control(&mut driver).await;
        assert!(!receipt.failed());
        assert!(matches!(
            receipt.result,
            Ok(NativeInteractiveControlReceipt::Workspace(_))
        ));
        driver.command("/workspace clear", 210);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("unavailable")
        );
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn workspace_slash_uses_actual_native_authority_and_reports_independent_receipts() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_workspace();
        let directory = fixture.workspace.parent().unwrap().join("shared one");
        std::fs::create_dir(&directory).unwrap();
        let config_path = fixture.workspace.parent().unwrap().join("workspace-config");
        let mut driver = driver(&fixture).await;
        driver.user_config = Some(Arc::new(NativeUserConfigStore::new(config_path.clone())));
        let record = driver.owner.runtime().record();
        driver.command(&format!("/workspace add {}", directory.display()), 200);
        assert!(!config_path.exists(), "accepting the slash is inert");
        let receipt = control(&mut driver).await;
        assert!(!receipt.failed());
        let NativeInteractiveControlReceipt::Workspace(value) = &receipt.result.as_ref().unwrap()
        else {
            panic!("workspace receipt")
        };
        assert_eq!(value.saved_changed, Some(true));
        assert_eq!(value.runtime_changed, Some(true));
        assert_eq!(value.snapshot.entries()[0].source().identity(), directory);
        assert!(crate::workspace::render_control_receipt(receipt.id.get(), value, 0).is_err());
        assert!(
            config_path.join("config.json").exists(),
            "render rejection never rolls back saved roots"
        );
        let output = super::super::driver::render_control(&receipt).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("shared one"));
        assert!(output.contains("confirmed"));
        assert_eq!(driver.owner.runtime().record(), record);
        driver.control_outcome = Some(receipt);
        driver.command("/workspace clear", 210);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("previous control")
        );
        driver.control_outcome.take();
        driver.command("/workspace clear", 220);
        let receipt = control(&mut driver).await;
        assert!(!receipt.failed());
        let NativeInteractiveControlReceipt::Workspace(value) = receipt.result.unwrap() else {
            panic!("workspace receipt")
        };
        assert!(value.snapshot.entries().is_empty());
        Box::pin(finish(driver, fixture)).await;
    });
}
async fn driver(fixture: &support::Fixture) -> Driver {
    let owner = NativeInteractiveSession::open(
        fixture.host.clone(),
        NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            NativeModelPreferences::new("fixture/default", NativeReasoningEffort::default(), false)
                .unwrap(),
        )
        .unwrap(),
        NativeInteractiveInitialSession::Fresh,
        100,
    )
    .await
    .unwrap();
    let (read, write) = std::io::pipe().unwrap();
    drop(write);
    let input = NativeInteractiveInput::new(
        NativeInteractiveInputSource::AdoptNonblockingStatus(
            std::os::fd::OwnedFd::from(read).into(),
        ),
        CancellationToken::new(),
    );
    let (_, inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    let (work, _) = tokio::sync::mpsc::channel(1);
    let (_, acknowledgements) = tokio::sync::mpsc::channel(1);
    let mut driver = Driver::new(
        owner,
        input,
        inbox,
        OutputBridge {
            work,
            acknowledgements,
        },
    )
    .unwrap();
    driver.notice = None;
    driver
}
async fn control(driver: &mut Driver) -> native::NativeInteractiveControlOutcome {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = driver.owner.poll_progress(cx, 300);
            driver
                .owner
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}

async fn turn_outcome(driver: &mut Driver, now_ms: i64) -> native::NativeInteractiveOutcome {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = driver.owner.poll_progress(cx, now_ms);
            let _ = driver.owner.take_presentation();
            driver
                .owner
                .take_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}
async fn finish(mut driver: Driver, fixture: support::Fixture) {
    driver.shutdown();
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = driver.owner.poll_progress(cx, 400);
            if driver.owner.is_closed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
    let completion = driver.input.input.completion();
    drop(driver);
    completion.wait_on_worker().unwrap();
    fixture.finish();
}

#[test]
fn ordinary_prompts_bypass_slash_envelope_but_both_keep_their_exact_bounds() {
    let prompt = "p".repeat(native::MAX_NATIVE_QUEUED_PROMPT_BYTES);
    assert!(
        matches!(submission(&prompt), Ok(Submission::Prompt(value)) if value.len() == prompt.len())
    );
    assert!(submission(&(prompt + "p")).is_err());
    assert!(
        submission(&format!(
            "/rename {}",
            "p".repeat(native::MAX_NATIVE_SLASH_INPUT_BYTES)
        ))
        .is_err()
    );
    assert!(
        submission(&format!(
            "/cancel{}",
            " ".repeat(native::MAX_NATIVE_SLASH_INPUT_BYTES)
        ))
        .is_err()
    );
    assert!(matches!(
        submission("unknown /help words"),
        Ok(Submission::Prompt(_))
    ));
}

#[test]
fn aliases_envelopes_and_unknown_prompt_routing_reuse_native_submission_semantics() {
    assert!(matches!(
        submission(" \t/exit"),
        Ok(Submission::Slash(NativeSlashCommand::Quit, ""))
    ));
    assert!(matches!(
        submission("/rename  \"title\" \t"),
        Ok(Submission::Slash(NativeSlashCommand::Rename, "\"title\""))
    ));
    for invalid in [
        "/quit now",
        "/help argument",
        "/resume latest",
        "/resume private:id",
        "/new\r",
        "/undo last",
        "/undo --",
        "/undo\r",
        "/copy last",
        "/copy --",
        "/copy\r",
    ] {
        assert!(submission(invalid).is_err(), "{invalid}");
    }
    assert!(matches!(
        submission("/unregistered-command"),
        Ok(Submission::Prompt(_))
    ));
    assert!(matches!(submission(" /cancel \t"), Ok(Submission::Cancel)));
    assert!(matches!(submission(" \t"), Ok(Submission::Empty)));
    assert!(matches!(
        submission(" \t/undo \t"),
        Ok(Submission::Slash(NativeSlashCommand::Undo, ""))
    ));
    assert!(matches!(
        submission(" \t/copy \t"),
        Ok(Submission::Slash(NativeSlashCommand::Copy, ""))
    ));
}

#[test]
fn rename_is_native_owned_and_cli_receipt_backpressure_rejects_followup_controls() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let mut driver = driver(&fixture).await;
        let before = driver.owner.runtime().record();
        driver.command("/rename durable title", 200);
        assert_eq!(driver.owner.runtime().record(), before);
        let receipt = control(&mut driver).await;
        assert!(matches!(
            receipt.result,
            Ok(NativeInteractiveControlReceipt::Renamed(_))
        ));
        driver.control_outcome = Some(receipt);
        driver.command("/rename forbidden followup", 210);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("previous control")
        );
        driver.command("/permissions yolo", 210);
        assert_eq!(
            driver
                .owner
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .mode(),
            PermissionMode::Ask
        );
        driver.control_outcome.take();
        assert_eq!(
            native::NativeSessionMetadata::from_metadata(&driver.owner.runtime().record().metadata)
                .unwrap()
                .title(),
            Some("durable title")
        );
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn model_selection_and_effort_use_explicit_native_composite_persistence() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("explicit-user"),
        ));
        let mut driver = driver(&fixture)
            .await
            .with_resources(None, Some(store.clone()));
        driver.command("/model selected/model", 200);
        assert_eq!(
            driver.owner.runtime().model_preferences().model(),
            "selected/model"
        );
        let NativeInteractiveControlReceipt::ModelDefaults(receipt) =
            control(&mut driver).await.result.unwrap()
        else {
            panic!("native composite save");
        };
        assert!(matches!(
            receipt.session,
            Ok(NativeModelPreferencePersistence::Saved { .. })
        ));
        assert!(receipt.user_defaults.is_ok());
        driver.command("/model effort future-tier", 210);
        assert!(control(&mut driver).await.result.is_ok());
        assert_eq!(
            store
                .load()
                .unwrap()
                .loaded()
                .config()
                .model_preferences()
                .effort()
                .label(),
            "future-tier"
        );
        driver.command("/model save", 220);
        assert!(matches!(
            control(&mut driver).await.result,
            Ok(NativeInteractiveControlReceipt::ModelSession(
                NativeModelPreferencePersistence::Unchanged
            ))
        ));
        driver.command("/model save-default", 230);
        assert!(matches!(
            control(&mut driver).await.result,
            Ok(NativeInteractiveControlReceipt::ModelDefaults(_))
        ));
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn modes_and_sandbox_change_native_selection_without_rewriting_saved_rules_or_history() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let mut driver = driver(&fixture).await;
        let before = driver.owner.runtime().record();
        driver.command("/permissions YOLO", 200);
        driver.command("/sandbox OS", 200);
        let policy = driver
            .owner
            .runtime()
            .permissions()
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(policy.mode(), PermissionMode::Yolo);
        assert_eq!(policy.sandbox_mode(), NativeSandboxMode::Os);
        assert_eq!(policy.effective_sandbox_mode(), NativeSandboxMode::None);
        driver.command("/permissions reset", 200);
        let policy = driver
            .owner
            .runtime()
            .permissions()
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(policy.mode(), PermissionMode::Ask);
        assert_eq!(policy.sandbox_mode(), NativeSandboxMode::Os);
        assert_eq!(driver.owner.runtime().record(), before);
        driver.command("/sandbox invalid secret", 200);
        assert_eq!(
            driver
                .owner
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .sandbox_mode(),
            NativeSandboxMode::Os
        );
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn transition_acceptance_deactivates_prompts_but_rejected_acceptance_keeps_scope() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let mut driver = driver(&fixture).await;
        driver.command("/clear invalid", 200);
        assert!(driver.scope_active);
        driver.command("/clear", 200);
        assert!(!driver.scope_active);
        assert!(driver.modal.is_none());
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = driver.owner.poll_progress(cx, 300);
                driver
                    .owner
                    .take_outcome()
                    .map_or(Poll::Pending, Poll::Ready)
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            result,
            native::NativeInteractiveOutcome::Transition(_)
        ));
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn undo_dispatches_the_actual_shared_tracker_without_transcript_mutation() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let mut driver = driver(&fixture).await;
        fixture.transport.push(support::call("write_file", &serde_json::json!({
            "path": "undo-世界.txt", "content": "created by actual tool"
        })));
        fixture.transport.push(support::answer());
        driver.owner.enqueue("perform one tracked write".into()).unwrap();
        assert!(matches!(turn_outcome(&mut driver, 200).await, native::NativeInteractiveOutcome::Turn(Ok(_))));
        assert_eq!(std::fs::read(fixture.workspace.join("undo-世界.txt")).unwrap(), b"created by actual tool");
        fixture.transport.push(support::answer());
        driver.owner.enqueue("remain active while undo is requested".into()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), poll_fn(|cx| {
            let progress = driver.owner.poll_progress(cx, 205);
            // Leave the first presentation event retained so provider progress
            // cannot race assertions about the undo's separate effect lane.
            assert!(driver.owner.take_outcome().is_none(), "second turn must be admitted before undo");
            if driver.owner.runtime().status().active && progress.is_ready() { Poll::Ready(()) } else { Poll::Pending }
        })).await.unwrap();
        let record = driver.owner.runtime().record();
        let requests = fixture.transport.requests().len();
        driver.command("/undo extra", 210);
        assert!(String::from_utf8(driver.notice.take().unwrap()).unwrap().contains("rejected"));
        assert!(driver.owner.take_control_outcome().is_none());
        driver.command("/undo", 211);
        assert!(fixture.workspace.join("undo-世界.txt").exists(), "command acceptance is inert");
        let receipt = control(&mut driver).await;
        assert!(matches!(&receipt.result, Ok(NativeInteractiveControlReceipt::Undone(native::FileUndoOutcome::Removed(path))) if path == "undo-世界.txt"));
        assert!(!fixture.workspace.join("undo-世界.txt").exists());
        assert_eq!(driver.owner.runtime().record(), record);
        assert!(driver.owner.runtime().status().active, "undo must not cancel the admitted response");
        assert_eq!(fixture.transport.requests().len(), requests);
        assert_eq!(fixture.undo.undo_last(&CancellationToken::new()).unwrap(), native::FileUndoOutcome::Empty);
        driver.control_outcome = Some(receipt);
        driver.command("/undo", 212);
        assert!(String::from_utf8(driver.notice.take().unwrap()).unwrap().contains("previous control"));
        assert!(driver.owner.take_control_outcome().is_none());
        driver.control_outcome.take();
        driver.command("/undo", 213);
        assert!(matches!(control(&mut driver).await.result, Ok(NativeInteractiveControlReceipt::Undone(native::FileUndoOutcome::Empty))));
        assert!(matches!(turn_outcome(&mut driver, 300).await, native::NativeInteractiveOutcome::Turn(Ok(event)) if matches!(event.payload, machine_god_core::TurnEvent::Completed { .. })));
        assert_eq!(fixture.transport.requests().len(), requests + 1);
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn missing_explicit_resources_and_fast_capabilities_fail_without_false_success() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let mut driver = driver(&fixture).await;
        let before = driver.owner.runtime().record();
        driver.command("/model save-default", 200);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("unavailable")
        );
        driver.command("/fast", 200);
        assert!(!driver.owner.runtime().model_preferences().requested_fast());
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("does not advertise")
        );
        for command in ["/allowlist", "/workspace clear"] {
            driver.command(command, 200);
            assert!(
                String::from_utf8(driver.notice.take().unwrap())
                    .unwrap()
                    .contains("unavailable")
            );
        }
        driver.command("/workspace list", 200);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("rejected")
        );
        assert_eq!(driver.owner.runtime().record(), before);
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn status_escapes_dynamic_content_and_help_does_not_claim_unwired_features() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let mut driver = driver(&fixture).await;
        driver
            .owner
            .set_model_preferences(
                NativeModelPreferences::new(
                    "private/\u{202e}model",
                    NativeReasoningEffort::default(),
                    false,
                )
                .unwrap(),
            )
            .unwrap();
        driver.command("/status", 200);
        let output = String::from_utf8(driver.notice.take().unwrap()).unwrap();
        assert!(!output.contains('\u{202e}'));
        assert!(output.contains("\\u202e"));
        driver.command("/help", 200);
        let help = String::from_utf8(driver.notice.take().unwrap()).unwrap();
        assert!(help.contains("/compact /undo /copy"));
        assert!(help.contains("/resume (picker)"));
        assert!(help.contains("Cmd/Super+R opens the all-workspace session picker."));
        assert!(help.contains(
            "/allowlist [view [effective|local|user]|[local|user] add|remove|reset ...]"
        ));
        assert!(help.contains("/workspace [list|add PATH|remove PATH|clear]"));
        assert!(!help.contains("Workspace editing is not yet wired."));
        driver.command("/version", 200);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains(env!("CARGO_PKG_VERSION"))
        );
        Box::pin(finish(driver, fixture)).await;
    });
}

async fn allowlist(driver: &mut Driver, command: &str) -> native::NativeAllowlistReceipt {
    driver.command(command, 220);
    match control(driver).await.result.unwrap() {
        NativeInteractiveControlReceipt::Allowlist(receipt) => receipt,
        _ => panic!("native allowlist receipt"),
    }
}

#[test]
fn allowlist_commands_persist_scoped_rules_and_preserve_remove_reset_distinctions() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("allowlist-user"),
        ));
        let mut driver = driver(&fixture)
            .await
            .with_resources(None, Some(store.clone()));
        let before = driver.owner.runtime().record();
        assert!(matches!(
            allowlist(&mut driver, "/allowlist user add tool read_file").await,
            native::NativeAllowlistReceipt::Mutation {
                outcome: native::NativeConfiguredPermissionMutationOutcome::Changed { .. },
                reload: Some(Ok(())),
                ..
            }
        ));
        allowlist(&mut driver, "/allowlist add command \"git status\"").await;
        let native::NativeAllowlistReceipt::View {
            sources, reload, ..
        } = allowlist(&mut driver, "/allowlist").await
        else {
            panic!("view receipt")
        };
        assert!(reload.is_ok());
        assert!(sources.user_shadowed_by_local());
        assert_eq!(sources.user().rules()[0].permission(), "read");
        assert_eq!(sources.effective().rules()[0].permission(), "bash");
        assert_eq!(sources.effective().rules()[0].pattern(), "git status");
        allowlist(&mut driver, "/allowlist remove command \"git status\"").await;
        let native::NativeAllowlistReceipt::Mutation {
            outcome,
            sources,
            reload,
            ..
        } = allowlist(&mut driver, "/allowlist reset all").await
        else {
            panic!("reset receipt")
        };
        assert_eq!(
            outcome,
            native::NativeConfiguredPermissionMutationOutcome::Unchanged
        );
        assert_eq!(reload, Some(Ok(())));
        assert!(sources.unwrap().local().unwrap().rules().is_empty());
        allowlist(&mut driver, "/allowlist add tool read_file").await;
        let native::NativeAllowlistReceipt::Mutation {
            outcome, sources, ..
        } = allowlist(&mut driver, "/allowlist reset tools").await
        else {
            panic!("reset receipt")
        };
        assert_eq!(
            outcome,
            native::NativeConfiguredPermissionMutationOutcome::Changed { removed_rules: 1 }
        );
        let sources = sources.unwrap();
        assert!(sources.local().is_none());
        assert_eq!(sources.effective().rules()[0].permission(), "read");
        let snapshot = store.load().unwrap();
        assert!(
            snapshot
                .loaded()
                .config()
                .permission_sources(&fixture.workspace)
                .unwrap()
                .local()
                .is_none()
        );
        assert_eq!(driver.owner.runtime().record(), before);
        assert!(fixture.transport.requests().is_empty());
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn allowlist_invalid_input_and_pending_receipts_do_not_change_settings() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let root = fixture.workspace.join("allowlist-user");
        let store = Arc::new(NativeUserConfigStore::new(root.clone()));
        let mut driver = driver(&fixture)
            .await
            .with_resources(None, Some(store.clone()));
        for command in [
            "/allowlist user",
            "/allowlist add tool web_fetch",
            "/allowlist add tool READ_FILE",
            "/allowlist add web-fetch-domain https://example.test",
        ] {
            driver.command(command, 200);
            assert!(
                String::from_utf8(driver.notice.take().unwrap())
                    .unwrap()
                    .contains("usage:")
            );
            assert!(!root.exists());
        }
        driver.command("/allowlist add tool read_file", 220);
        driver.control_outcome = Some(control(&mut driver).await);
        driver.command("/allowlist reset all", 230);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("pending")
        );
        let snapshot = store.load().unwrap();
        assert_eq!(
            snapshot
                .loaded()
                .config()
                .permission_sources(&fixture.workspace)
                .unwrap()
                .effective()
                .rules()
                .len(),
            1
        );
        assert!(driver.control_outcome.is_some());
        assert!(fixture.transport.requests().is_empty());
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn allowlist_view_reports_shadow_and_inert_malformed_rules_without_displaying_them() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("allowlist-user"),
        ));
        let mut driver = driver(&fixture)
            .await
            .with_resources(None, Some(store.clone()));
        allowlist(&mut driver, "/allowlist user add tool read_file").await;
        let snapshot = store.load().unwrap();
        store
            .apply_permission_mutation(
                &snapshot,
                &fixture.workspace,
                native::NativeConfiguredPermissionScope::Local,
                &native::NativeConfiguredPermissionMutation::Add {
                    permission: "web_fetch".into(),
                    pattern: "bad\u{1b}[2J".into(),
                },
            )
            .await
            .unwrap();
        let receipt = allowlist(&mut driver, "/allowlist view effective").await;
        let rendered =
            String::from_utf8(super::super::allowlist_view::render(1, &receipt).unwrap()).unwrap();
        assert!(rendered.contains("effective persistent allow rules: (none)"));
        assert!(rendered.contains("user rules are shadowed by local workspace rules"));
        assert!(rendered.contains("ignored 1 malformed web_fetch rule;"));
        assert!(!rendered.contains("bad"));
        assert!(!rendered.contains('\u{1b}'));
        let native::NativeAllowlistReceipt::View { sources, .. } = receipt else {
            panic!("view receipt")
        };
        assert_eq!(
            sources.local().unwrap().rules().len(),
            1,
            "presentation does not delete malformed settings"
        );
        let receipt = allowlist(&mut driver, "/allowlist view user").await;
        let rendered =
            String::from_utf8(super::super::allowlist_view::render(2, &receipt).unwrap()).unwrap();
        assert!(rendered.contains("read: workspace"));
        assert!(rendered.contains("user rules are shadowed"));
        assert!(!rendered.contains("malformed"));
        assert!(fixture.transport.requests().is_empty());
        Box::pin(finish(driver, fixture)).await;
    });
}

struct CatalogTransport;
impl native::AiGatewayModelCatalogTransport for CatalogTransport {
    fn wait_until(&self, _: std::time::Instant) -> machine_god_core::BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
    fn get(
        &self,
        _: native::AiGatewayModelCatalogRequestAccess,
        _: std::time::Instant,
        _: CancellationToken,
    ) -> machine_god_core::BoxFuture<
        '_,
        Result<
            native::AiGatewayModelCatalogTransportResponse,
            native::AiGatewayModelCatalogTransportError,
        >,
    > {
        Box::pin(async {
            Ok(native::AiGatewayModelCatalogTransportResponse::new(
                200,
                serde_json::to_vec(&serde_json::json!({"data":[
                    {"id":"openai/model-five", "fast_options":[{"type":"toggle"}]},
                    {"id":"anthropic/model-three"}
                ]}))
                .unwrap(),
            ))
        })
    }
}

#[test]
fn cached_catalog_query_selection_and_fast_toggle_use_advertised_native_capabilities() {
    executor().block_on(async {
        let fixture = support::Fixture::new();
        let catalog = Arc::new(
            native::AiGatewayModelCatalogProvider::new(
                native::AiGatewayModelCatalogAccessMode::PublicOnly,
                Arc::new(CatalogTransport),
            )
            .list_model_details(CancellationToken::new())
            .await
            .unwrap(),
        );
        let mut driver = driver(&fixture).await.with_resources(Some(catalog), None);
        driver.command("/models", 200);
        let text = String::from_utf8(driver.notice.take().unwrap()).unwrap();
        assert!(text.contains("openai/model-five"));
        assert!(text.contains("anthropic/model-three"));
        driver.command("/model MODEL-FIVE", 210);
        assert!(control(&mut driver).await.result.is_ok());
        assert_eq!(
            driver.owner.runtime().model_preferences().model(),
            "openai/model-five"
        );
        driver.command("/fast", 220);
        assert!(control(&mut driver).await.result.is_ok());
        assert!(driver.owner.runtime().model_preferences().requested_fast());
        driver.command("/fast", 230);
        assert!(control(&mut driver).await.result.is_ok());
        assert!(!driver.owner.runtime().model_preferences().requested_fast());
        Box::pin(finish(driver, fixture)).await;
    });
}
