use super::*;
use crate::{
    NativeBackgroundUrlExecutable, NativeOwnedWorkerScope,
    mcp::{browser_launcher::NativeMcpBrowserLauncher, interaction::McpUrlRecoveryAnswer},
};
use std::fs::File;

mod process;

const URL: &str = r#""result":{"resultType":"input_required","inputRequests":{"url":{"method":"elicitation/create","params":{"mode":"url","message":"Authorize","url":"https://example.test/auth"}}},"requestState":{"exact":1e-99999}}"#;

/// A retained directory is rejected before `Command::spawn`. These tests exercise
/// real worker admission and the real inbox without creating any process.
struct UnavailableLauncher {
    launcher: NativeMcpBrowserLauncher,
    workers: NativeOwnedWorkerScope,
}
impl UnavailableLauncher {
    fn new(archive: &Archive) -> Self {
        let path = std::fs::canonicalize(&archive.path).unwrap();
        let executable =
            NativeBackgroundUrlExecutable::new(path.clone(), File::open(path).unwrap()).unwrap();
        let workers = NativeOwnedWorkerScope::new();
        let launcher = NativeMcpBrowserLauncher::new(executable, vec![], workers.clone()).unwrap();
        Self { launcher, workers }
    }
}
impl Drop for UnavailableLauncher {
    fn drop(&mut self) {
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
    }
}
fn configured_url(
    archive: &Archive,
    launcher: &UnavailableLauncher,
    body: &'static str,
) -> (Fixture, NativeInteractivePromptInbox) {
    configured_with_launcher(
        archive,
        &[json!({"exact":9_007_199_254_740_993_u64})],
        move |id| {
            envelope(
                id,
                if id == 1 {
                    body
                } else {
                    r#""result":{"resultType":"complete","content":[]}"#
                },
            )
        },
        None,
        Some(launcher.launcher.clone()),
    )
}
fn run_recovery(
    fixture: &Fixture,
    inbox: &mut NativeInteractivePromptInbox,
    recovery: McpUrlRecoveryAnswer,
) -> (Vec<EngineEvent>, usize) {
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut count = 0;
    let events = block_on(async {
        let responder = async {
            loop {
                let view = std::future::poll_fn(|cx| inbox.poll_prompt(cx))
                    .await
                    .unwrap();
                if view.elicitation().is_some() {
                    inbox
                        .reply(view.token(), answer(r#"{"action":"accept"}"#))
                        .unwrap();
                } else {
                    assert!(view.url_recovery().is_some());
                    count += 1;
                    inbox
                        .reply(
                            view.token(),
                            NativeInteractivePromptResponse::UrlRecovery(recovery),
                        )
                        .unwrap();
                }
            }
        };
        match select(Box::pin(turn.collect::<Vec<_>>()), Box::pin(responder)).await {
            Either::Left((events, _)) => events
                .into_iter()
                .collect::<std::result::Result<_, _>>()
                .unwrap(),
            Either::Right(_) => unreachable!(),
        }
    });
    (events, count)
}

#[test]
fn explicit_manual_url_recovery_continues_exact_original_grant_and_state() {
    let archive = Archive::new();
    let launcher = UnavailableLauncher::new(&archive);
    let (fixture, mut inbox) = configured_url(&archive, &launcher, URL);
    let (events, recoveries) =
        run_recovery(&fixture, &mut inbox, McpUrlRecoveryAnswer::ContinueManually);
    assert!(!result(&events).is_error);
    assert_eq!(recoveries, 1);
    let wire = wires(&fixture);
    assert_eq!(wire.len(), 2);
    assert_eq!(
        wire[0]["params"]["arguments"],
        wire[1]["params"]["arguments"]
    );
    assert_eq!(
        wire[1]["params"]["inputResponses"]["url"],
        json!({"action":"accept"})
    );
    assert_eq!(
        serde_json::to_string(&wire[1]["params"]["requestState"]["exact"]).unwrap(),
        "1e-99999"
    );
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
}

#[test]
fn browser_failure_recovery_is_three_questions_then_explicit_cancel() {
    let archive = Archive::new();
    let launcher = UnavailableLauncher::new(&archive);
    let (fixture, mut inbox) = configured_url(&archive, &launcher, URL);
    let (_, recoveries) = run_recovery(&fixture, &mut inbox, McpUrlRecoveryAnswer::RetryBrowser);
    assert_eq!(recoveries, 3);
    assert_eq!(
        wires(&fixture)[1]["params"]["inputResponses"]["url"],
        json!({"action":"cancel"})
    );
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
}

#[test]
fn url_decline_and_cancel_do_not_admit_launcher_workers() {
    for action in ["decline", "cancel"] {
        let archive = Archive::new();
        let launcher = UnavailableLauncher::new(&archive);
        launcher.workers.close();
        let (fixture, mut inbox) = configured_url(&archive, &launcher, URL);
        let (_, count) = run_answered(&fixture, &mut inbox, &format!(r#"{{"action":"{action}"}}"#));
        assert_eq!(count, 1);
        assert_eq!(
            wires(&fixture)[1]["params"]["inputResponses"]["url"],
            json!({"action":action})
        );
    }
}

#[test]
fn revoked_turn_during_url_recovery_discards_prompt_and_does_not_continue() {
    let archive = Archive::new();
    let launcher = UnavailableLauncher::new(&archive);
    let (fixture, mut inbox) = configured_url(&archive, &launcher, URL);
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let handle = turn.handle().unwrap();
    let mut collect = Box::pin(turn.collect::<Vec<_>>());
    let consent = queued_prompt(&mut collect, &mut inbox);
    inbox
        .reply(consent.token(), answer(r#"{"action":"accept"}"#))
        .unwrap();
    let recovery = queued_prompt(&mut collect, &mut inbox);
    assert!(recovery.url_recovery().is_some());
    assert!(handle.cancel());
    let _events = block_on(collect);
    assert!(
        inbox
            .reply(
                recovery.token(),
                NativeInteractivePromptResponse::UrlRecovery(
                    McpUrlRecoveryAnswer::ContinueManually
                )
            )
            .is_err()
    );
    assert_eq!(wires(&fixture).len(), 1);
}

#[test]
fn url_recovery_cancel_cancels_remaining_forms_without_presenting_them() {
    const MIXED: &str = r#""result":{"resultType":"input_required","inputRequests":{"first":{"method":"elicitation/create","params":{"mode":"url","message":"Authorize","url":"https://example.test/auth"}},"second":{"method":"elicitation/create","params":{"message":"Second","requestedSchema":{"type":"object","properties":{}}}}}}"#;
    let archive = Archive::new();
    let launcher = UnavailableLauncher::new(&archive);
    let (fixture, mut inbox) = configured_url(&archive, &launcher, MIXED);
    let (_, recoveries) = run_recovery(&fixture, &mut inbox, McpUrlRecoveryAnswer::Cancel);
    assert_eq!(recoveries, 1);
    assert_eq!(
        wires(&fixture)[1]["params"]["inputResponses"],
        json!({"first":{"action":"cancel"},"second":{"action":"cancel"}})
    );
}

#[test]
fn recovery_keeps_the_original_deadline_and_permission_grant() {
    for expire in [false, true] {
        let archive = Archive::new();
        let launcher = UnavailableLauncher::new(&archive);
        let clock = Arc::new(AdvancingClock::new());
        let (fixture, mut inbox) = configured_with_launcher(
            &archive,
            &[json!({})],
            |id| envelope(id, URL),
            Some(clock.clone()),
            Some(launcher.launcher.clone()),
        );
        fixture.conversation.enqueue("go".into()).unwrap();
        let turn = block_on(fixture.conversation.start_next(1))
            .unwrap()
            .unwrap();
        let mut collect = Box::pin(turn.collect::<Vec<_>>());
        let consent = queued_prompt(&mut collect, &mut inbox);
        clock.advance(1000);
        inbox
            .reply(consent.token(), answer(r#"{"action":"accept"}"#))
            .unwrap();
        let recovery = queued_prompt(&mut collect, &mut inbox);
        assert!(recovery.url_recovery().is_some());
        if expire {
            clock.advance(800); // Original budget, not 30 minutes from recovery.
        } else {
            fixture.conversation.permissions().unwrap().reset().unwrap();
        }
        let events = block_on(collect)
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(result(&events).is_error);
        assert!(
            inbox
                .reply(
                    recovery.token(),
                    NativeInteractivePromptResponse::UrlRecovery(
                        McpUrlRecoveryAnswer::ContinueManually
                    ),
                )
                .is_err()
        );
        assert_eq!(wires(&fixture).len(), 1);
        assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
    }
}
