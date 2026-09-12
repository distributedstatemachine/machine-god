//! Real host-owned launcher, literal human commands, CLI modal and HTTP resume.
use super::*;
use futures_util::future::join;
use std::fs::File;

fn launcher(success: bool) -> native::NativeBackgroundUrlExecutable {
    // Explicit mock exit outcomes; neither executable opens the external URL.
    // The production direct-child launcher still owns admission and actual reap.
    let path = std::fs::canonicalize(if success {
        "/usr/bin/true"
    } else {
        "/usr/bin/false"
    })
    .unwrap();
    native::NativeBackgroundUrlExecutable::new(path.clone(), File::open(path).unwrap()).unwrap()
}

fn action(get: bool) -> native::McpFeatureAction {
    if get {
        native::McpFeatureAction::PromptGet
    } else {
        native::McpFeatureAction::ResourceRead
    }
}
fn method(get: bool) -> &'static str {
    if get { "prompts/get" } else { "resources/read" }
}

async fn begin_url(driver: &mut Driver, listener: &TcpListener, get: bool) -> (Modal, Value) {
    driver.command(
        if get {
            r#"/mcp prompt get fixture review {"topic":"rust"}"#
        } else {
            "/mcp resource read fixture test://fixed"
        },
        200,
    );
    let producer = async {
        http::reply(
            listener,
            if get {
                "prompts/list"
            } else {
                "resources/list"
            },
            if get {
                http::prompts()
            } else {
                http::resources()
            },
        )
        .await;
        http::reply(listener, method(get), machine_god_core::json::from_str(r#"{
            "resultType":"input_required","requestState":{"exact":1e-99999,"$serde_json::private::Number":"literal"},
            "inputRequests":{"url":{"method":"elicitation/create","params":{"mode":"url","message":"Approve exact human URL","url":"https://example.test/private-human-url"}}}
        }"#).unwrap()).await
    };
    tokio::time::timeout(Duration::from_secs(10), join(prompt(driver), producer))
        .await
        .unwrap()
}

fn answer(driver: &mut Driver, modal: &mut Modal, line: &str, get: bool, recovery: bool) {
    let selected = if recovery {
        modal.view.url_recovery().unwrap().source()
    } else {
        modal.view.elicitation().unwrap()
    };
    let McpElicitationPromptSource::HumanFeature {
        owner,
        action: observed,
    } = selected.source()
    else {
        panic!("exact human feature origin")
    };
    assert_eq!(*observed, action(get));
    assert_eq!(
        *owner,
        super::super::super::super::super::super::principal(&driver.owner)
    );
    let rendered = String::from_utf8(modal.render().unwrap()).unwrap();
    assert!(rendered.contains(&format!("Human feature: {}", action(get).as_str())));
    assert!(!rendered.contains("Tool:"));
    assert_eq!(rendered.contains("private-human-url"), !recovery);
    assert!(modal.answer(line, &modal.presentation_binding()).is_err());
    modal.displayed = true;
    let response = modal.answer(line, &modal.binding()).unwrap().unwrap();
    driver.inbox.reply(modal.view.token(), response).unwrap();
}

async fn completed(
    driver: &mut Driver,
    listener: &TcpListener,
    original: &Value,
    get: bool,
) -> native::NativeInteractiveControlOutcome {
    let body = if get {
        json!({"messages":[{"role":"user","content":{"type":"text","text":"URL confirmed prompt"}}]})
    } else {
        json!({"contents":[{"uri":"test://fixed","text":"URL confirmed resource"}]})
    };
    let (outcome, mut resumed) = tokio::time::timeout(
        Duration::from_secs(10),
        join(control(driver), http::reply(listener, method(get), body)),
    )
    .await
    .unwrap();
    assert!(!outcome.failed());
    assert!(resumed["id"].as_i64().unwrap() > original["id"].as_i64().unwrap());
    let params = resumed["params"].as_object_mut().unwrap();
    let state = params.remove("requestState").unwrap();
    assert_eq!(serde_json::to_string(&state["exact"]).unwrap(), "1e-99999");
    assert_eq!(state["$serde_json::private::Number"], "literal");
    assert_eq!(
        params.remove("inputResponses"),
        Some(json!({"url":{"action":"accept"}}))
    );
    assert_eq!(
        resumed["params"], original["params"],
        "original arguments and client metadata stay exact"
    );
    let Ok(NativeInteractiveControlReceipt::McpFeature(receipt)) = &outcome.result else {
        panic!("feature receipt")
    };
    assert_eq!(receipt.action(), action(get));
    assert!(receipt.revalidate().is_ok());
    outcome
}

#[test]
fn human_read_and_get_url_consent_composes_owned_handoff_and_manual_recovery() {
    executor().block_on(async {
        for get in [false, true] {
            for success in [false, true] {
                let (fixture, mut driver, listener) = http::setup_with_host_options(|options| {
                    options.with_background_url_opener(launcher(success), vec![])
                })
                .await;
                let record = driver.owner.runtime().record();
                let (mut consent, original) = begin_url(&mut driver, &listener, get).await;
                answer(&mut driver, &mut consent, "y", get, false);
                if !success {
                    let mut recovery = prompt(&mut driver).await;
                    assert!(
                        String::from_utf8(recovery.render().unwrap())
                            .unwrap()
                            .contains("Browser handoff was not confirmed")
                    );
                    answer(&mut driver, &mut recovery, "m", get, true);
                }
                let outcome = completed(&mut driver, &listener, &original, get).await;
                assert_eq!(driver.owner.runtime().record(), record);
                assert_eq!(driver.owner.runtime().status().queued_jobs, 0);
                assert!(fixture.transport.requests().is_empty());
                assert!(
                    driver
                        .inbox
                        .poll_prompt(&mut std::task::Context::from_waker(std::task::Waker::noop()))
                        .is_pending(),
                    "successful handoff does not ask for recovery"
                );
                drop(outcome);
                Box::pin(http::finish_runtime(driver, fixture)).await;
            }
        }
    });
}

#[test]
fn cancelled_human_url_recovery_rejects_stale_answer_without_resuming_original_request() {
    executor().block_on(async {
        let (fixture, mut driver, listener) = http::setup_with_host_options(|options| {
            options.with_background_url_opener(launcher(false), vec![])
        })
        .await;
        let (mut consent, _) = begin_url(&mut driver, &listener, false).await;
        answer(&mut driver, &mut consent, "y", false, false);
        let mut recovery = prompt(&mut driver).await;
        recovery.displayed = true;
        let response = recovery.answer("m", &recovery.binding()).unwrap().unwrap();
        assert!(driver.owner.request_cancel());
        let outcome = tokio::time::timeout(Duration::from_secs(10), control(&mut driver))
            .await
            .unwrap();
        assert!(outcome.failed());
        assert!(driver.inbox.reply(recovery.view.token(), response).is_err());
        let listener = listener.into_std().unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(driver.owner.runtime().status().queued_jobs, 0);
        assert!(fixture.transport.requests().is_empty());
        drop(outcome);
        Box::pin(http::finish_runtime(driver, fixture)).await;
    });
}
