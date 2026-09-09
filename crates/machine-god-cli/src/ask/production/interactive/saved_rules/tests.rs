use super::super::{support, *};
use super::*;
use machine_god_core::CancellationToken;
use machine_god_native as native;
use std::io::Write;
use std::task::Poll;
use std::time::Duration;

struct Harness {
    fixture: support::Fixture,
    driver: Driver,
    writer: std::io::PipeWriter,
    work: tokio::sync::mpsc::Receiver<OutputWork>,
    ack: tokio::sync::mpsc::Sender<OutputAcknowledgement>,
    _signal: tokio::sync::mpsc::Sender<AskSignal>,
    signals: AskSignals,
    output: Vec<u8>,
}

impl Harness {
    async fn new() -> Self {
        let (bridge, inbox) =
            NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
        let fixture = support::Fixture::with_prompter(bridge);
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            native::NativeModelPreferences::new(
                "fixture/default",
                native::NativeReasoningEffort::default(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let owner = NativeInteractiveSession::open(
            fixture.host.clone(),
            options,
            NativeInteractiveInitialSession::Fresh,
            100,
        )
        .await
        .unwrap();
        let (reader, writer) = std::io::pipe().unwrap();
        let input = NativeInteractiveInput::new(
            NativeInteractiveInputSource::AdoptNonblockingStatus(
                std::os::fd::OwnedFd::from(reader).into(),
            ),
            CancellationToken::new(),
        );
        let (sender, work) = tokio::sync::mpsc::channel(1);
        let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
        let (signal, signals) = tokio::sync::mpsc::channel(1);
        let driver = Driver::new(
            owner,
            input,
            inbox,
            OutputBridge {
                work: sender,
                acknowledgements,
                tape: None,
            },
        )
        .unwrap();
        let mut result = Self {
            fixture,
            driver,
            writer,
            work,
            ack,
            _signal: signal,
            signals: AskSignals::new(signals),
            output: Vec::new(),
        };
        result
            .pump(|driver| {
                driver.notice.is_none() && driver.render.is_none() && driver.in_flight.is_none()
            })
            .await;
        result
    }

    async fn pump(&mut self, done: impl Fn(&Driver) -> bool) {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = self.driver.poll(cx, &mut self.signals);
                if let Ok(work) = self.work.try_recv() {
                    if let OutputWork::Write(bytes) = work {
                        self.output.extend(bytes);
                    }
                    self.ack.try_send(OutputAcknowledgement::Succeeded).unwrap();
                    cx.waker().wake_by_ref();
                }
                if done(&self.driver) {
                    Poll::Ready(())
                } else {
                    assert!(result.is_pending());
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
    }

    async fn send(&mut self, line: &str, done: impl Fn(&Driver) -> bool) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.pump(done).await;
    }

    fn tool(&mut self, followup: bool) {
        self.fixture.transport.push(support::call(
            "write_file",
            &serde_json::json!({"path":"exact.txt","content":"saved effect"}),
        ));
        if followup {
            self.fixture.transport.push(support::answer());
        }
        self.driver
            .owner
            .enqueue("perform exact file action".into())
            .unwrap();
    }

    fn rules(&self) -> NativeSessionPermissionRules {
        NativeSessionPermissionRules::from_metadata(
            &self.driver.owner.runtime().record_snapshot().metadata,
        )
        .unwrap()
    }

    async fn finish(mut self) {
        self.driver.shutdown();
        self.pump(|driver| driver.owner.is_closed()).await;
        let input = self.driver.input.input.completion();
        drop(self.driver);
        input.wait_on_worker().unwrap();
        self.fixture.finish();
    }
}

fn displayed(driver: &Driver) -> bool {
    driver.modal.as_ref().is_some_and(|modal| modal.displayed)
}
fn confirmed_page(driver: &Driver) -> bool {
    driver
        .saved_rule
        .as_ref()
        .is_some_and(|confirmation| confirmation.displayed)
}
fn idle(driver: &Driver) -> bool {
    !driver.owner.runtime().status().active
        && driver.owner.runtime().status().queued_jobs == 0
        && driver.outcome.is_none()
        && driver.control_outcome.is_none()
        && driver.notice.is_none()
        && driver.render.is_none()
        && driver.in_flight.is_none()
}
fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn saved_exact_allow_requires_new_confirmation_and_fresh_preparation_before_effect() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        h.tool(false);
        h.pump(displayed).await;
        assert!(h.driver.modal.as_ref().unwrap().view.can_save_rule());
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        // A pasted yes belongs to the first prompt, never the new confirmation.
        h.send("a\nyes\n/permissions yolo\n", confirmed_page).await;
        h.pump(|driver| {
            driver.notice.is_none() && driver.render.is_none() && driver.in_flight.is_none()
        })
        .await;
        assert!(h.rules().rules().is_empty());
        assert_eq!(
            h.driver
                .owner
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .mode(),
            native::PermissionMode::Ask
        );
        h.send("yes\n", |driver| driver.saved_rule.is_none()).await;
        h.pump(idle).await;
        assert_eq!(h.rules().rules().len(), 1);
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        h.tool(true);
        h.pump(idle).await;
        assert_eq!(
            std::fs::read(h.fixture.workspace.join("exact.txt")).ok(),
            Some(b"saved effect".to_vec()),
            "output: {} requests: {}",
            String::from_utf8_lossy(&h.output),
            h.fixture.transport.requests().len()
        );
        assert!(String::from_utf8_lossy(&h.output).contains("will not execute or retry"));
        Box::pin(h.finish()).await;
    });
}

#[test]
fn saved_exact_deny_and_explicit_revoke_restore_prompting_without_resurrecting_old_answers() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        h.tool(false);
        h.pump(displayed).await;
        h.send("d\n", confirmed_page).await;
        h.send("yes\n", |driver| driver.saved_rule.is_none()).await;
        h.pump(idle).await;
        let id = h.rules().rules()[0].id();
        h.tool(true);
        h.pump(idle).await;
        assert!(h.driver.modal.is_none());
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        h.send(&format!("/permissions revoke {id}\n"), confirmed_page)
            .await;
        assert_eq!(h.rules().rules().len(), 1);
        h.send("yes\n", |driver| driver.saved_rule.is_none()).await;
        h.pump(idle).await;
        assert!(h.rules().rules().is_empty());
        h.tool(true);
        h.pump(displayed).await;
        h.send("n\n", |driver| driver.modal.is_none()).await;
        h.pump(idle).await;
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        Box::pin(h.finish()).await;
    });
}

#[test]
fn reset_invalidates_displayed_saved_confirmation_without_persisting_or_executing() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        h.tool(false);
        h.pump(displayed).await;
        h.send("a\n", confirmed_page).await;
        h.send("/permissions reset\nyes\n", |driver| {
            driver.saved_rule.is_none()
        })
        .await;
        h.pump(idle).await;
        assert!(h.rules().rules().is_empty());
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        Box::pin(h.finish()).await;
    });
}

#[test]
fn listing_escapes_display_and_revocation_checks_canonical_id_and_reset_epoch() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        let owner = h.driver.owner.runtime().permissions().unwrap();
        let proposal = owner
            .propose_rule_change(NativePermissionRuleChange::Set {
                key: native::NativePermissionRuleKey::new(
                    native::NativePermissionRuleKind::StructuredTool,
                    "exact native key, not the displayed label",
                )
                .unwrap(),
                display_identity: "display\u{1b}[2J\u{202e}".into(),
                decision: NativePermissionRuleDecision::Deny,
            })
            .unwrap();
        owner.confirm_rule_change(proposal).await.unwrap();
        let id = h.rules().rules()[0].id();
        h.writer.write_all(b"/permissions rules\n").unwrap();
        h.pump(|driver| driver.notice.is_some() || driver.render.is_some())
            .await;
        h.pump(idle).await;
        let output = String::from_utf8_lossy(&h.output);
        assert!(output.contains("[saved exact rules] 1 total"));
        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("exact native key"));
        h.send(&format!("/permissions revoke {id}\n"), confirmed_page)
            .await;
        h.send("/permissions reset\nyes\n", |driver| {
            driver.saved_rule.is_none()
        })
        .await;
        h.pump(idle).await;
        assert_eq!(h.rules().rules()[0].id(), id);
        Box::pin(h.finish()).await;
    });
}

#[test]
fn declining_confirmation_denies_pending_action_without_a_rule_or_effect() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        h.tool(true);
        h.pump(displayed).await;
        h.send("a\n", confirmed_page).await;
        h.send("no\n", |driver| driver.saved_rule.is_none()).await;
        h.pump(idle).await;
        assert!(h.rules().rules().is_empty());
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        Box::pin(h.finish()).await;
    });
}

#[test]
fn ordinary_once_answer_executes_without_saving_a_rule() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        h.tool(true);
        h.pump(displayed).await;
        h.send("y\n", |driver| driver.modal.is_none()).await;
        h.pump(idle).await;
        assert!(h.rules().rules().is_empty());
        assert_eq!(
            std::fs::read(h.fixture.workspace.join("exact.txt")).unwrap(),
            b"saved effect"
        );
        Box::pin(h.finish()).await;
    });
}

#[test]
fn cli_mode_change_preserves_active_ask_and_changes_the_next_queued_job() {
    executor().block_on(async {
        let mut h = Harness::new().await;
        h.tool(true);
        h.tool(true);
        h.pump(displayed).await;
        h.send("/permissions yolo\n", |driver| {
            driver
                .owner
                .runtime()
                .permissions()
                .unwrap()
                .snapshot()
                .unwrap()
                .mode()
                == native::PermissionMode::Yolo
        })
        .await;
        assert!(displayed(&h.driver));
        assert!(!h.fixture.workspace.join("exact.txt").exists());
        h.send("n\n", |driver| driver.modal.is_none()).await;
        h.pump(idle).await;
        assert_eq!(
            std::fs::read(h.fixture.workspace.join("exact.txt")).unwrap(),
            b"saved effect"
        );
        assert!(h.rules().rules().is_empty());
        assert_eq!(h.fixture.transport.requests().len(), 4);
        Box::pin(h.finish()).await;
    });
}
