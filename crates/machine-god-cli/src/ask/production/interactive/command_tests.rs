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
    ] {
        assert!(submission(invalid).is_err(), "{invalid}");
    }
    assert!(matches!(
        submission("/unregistered-command"),
        Ok(Submission::Prompt(_))
    ));
    assert!(matches!(submission(" /cancel \t"), Ok(Submission::Cancel)));
    assert!(matches!(submission(" \t"), Ok(Submission::Empty)));
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
        for command in ["/allowlist", "/undo", "/copy", "/workspace list"] {
            driver.command(command, 200);
            assert!(
                String::from_utf8(driver.notice.take().unwrap())
                    .unwrap()
                    .contains("unavailable")
            );
        }
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
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains("not yet wired")
        );
        driver.command("/version", 200);
        assert!(
            String::from_utf8(driver.notice.take().unwrap())
                .unwrap()
                .contains(env!("CARGO_PKG_VERSION"))
        );
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
