use super::super::super::super::super::presentation::Modal;
use super::*;
use native::mcp::interaction::McpElicitationPromptSource;

fn input_required() -> Value {
    json!({"resultType":"input_required","requestState":null,"inputRequests":{
        "confirm":{"method":"elicitation/create","params":{
            "message":"Confirm exact human action",
            "requestedSchema":{"type":"object","properties":{}}
        }}
    }})
}

async fn prompt(driver: &mut Driver) -> Modal {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = driver.owner.poll_progress(cx, 201);
            assert!(driver.owner.take_control_outcome().is_none());
            driver
                .inbox
                .poll_prompt(cx)
                .map(|view| Modal::new(view.expect("live human prompt")))
        }),
    )
    .await
    .unwrap()
}

async fn begin(driver: &mut Driver, listener: &TcpListener, get: bool) -> (Modal, Value) {
    driver.command(
        if get {
            r#"/mcp prompt get fixture review {"topic":"rust"}"#
        } else {
            "/mcp resource read fixture test://fixed"
        },
        200,
    );
    let server = async {
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
        http::reply(
            listener,
            if get { "prompts/get" } else { "resources/read" },
            input_required(),
        )
        .await
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::future::join(prompt(driver), server),
    )
    .await
    .unwrap()
}

fn accept(driver: &mut Driver, modal: &mut Modal, action: native::McpFeatureAction) {
    let source = modal.view.elicitation().unwrap().source();
    let McpElicitationPromptSource::HumanFeature {
        owner,
        action: selected,
    } = source
    else {
        panic!("real human origin")
    };
    assert_eq!(*selected, action);
    assert_eq!(
        *owner,
        super::super::super::super::super::principal(&driver.owner)
    );
    let rendered = String::from_utf8(modal.render().unwrap()).unwrap();
    assert!(rendered.contains(&format!("Human feature: {}", action.as_str())));
    assert!(!rendered.contains("Tool:"));
    assert!(
        modal
            .answer("/next", &modal.presentation_binding())
            .is_err()
    );
    // Explicit presentation acknowledgements, matching the ordinary modal API.
    modal.displayed = true;
    let old = modal.binding();
    assert!(modal.answer("/next", &old).unwrap().is_none());
    assert!(!modal.displayed);
    modal.displayed = true;
    assert!(modal.answer("y", &old).is_err());
    let answer = modal.answer("y", &modal.binding()).unwrap().unwrap();
    driver.inbox.reply(modal.view.token(), answer).unwrap();
}

#[test]
fn literal_read_and_get_resume_through_the_exact_command_bridge_and_modal() {
    executor().block_on(async {
        for get in [false, true] {
            let (fixture, mut driver, listener) = http::setup().await;
            let record = driver.owner.runtime().record();
            let (mut modal, original) = begin(&mut driver, &listener, get).await;
            let action = if get { native::McpFeatureAction::PromptGet } else { native::McpFeatureAction::ResourceRead };
            accept(&mut driver, &mut modal, action);
            let body = if get { json!({"messages":[{"role":"user","content":{"type":"text","text":"confirmed prompt"}}]}) } else { json!({"contents":[{"uri":"test://fixed","text":"confirmed resource"}]}) };
            let (outcome, resumed) = tokio::time::timeout(Duration::from_secs(10), futures_util::future::join(control(&mut driver), http::reply(&listener, if get { "prompts/get" } else { "resources/read" }, body))).await.unwrap();
            assert!(!outcome.failed());
            assert!(resumed["id"].as_i64().unwrap() > original["id"].as_i64().unwrap());
            let mut params = resumed["params"].clone();
            assert_eq!(params.as_object_mut().unwrap().remove("requestState"), Some(Value::Null));
            assert_eq!(params.as_object_mut().unwrap().remove("inputResponses"), Some(json!({"confirm":{"action":"accept","content":{}}})));
            assert_eq!(params, original["params"]);
            let Ok(NativeInteractiveControlReceipt::McpFeature(receipt)) = &outcome.result else { panic!("feature receipt") };
            assert_eq!(receipt.action(), action);
            assert!(receipt.revalidate().is_ok());
            assert_eq!(driver.owner.runtime().record(), record);
            assert!(fixture.transport.requests().is_empty());
            drop(outcome);
            Box::pin(http::finish_runtime(driver, fixture)).await;
        }
    });
}

#[test]
fn literal_modal_cancel_sends_only_the_original_canonical_cancel_response() {
    executor().block_on(async {
        let (fixture, mut driver, listener) = http::setup().await;
        let (mut modal, original) = begin(&mut driver, &listener, false).await;
        modal.displayed = true;
        assert!(modal.answer("/next", &modal.binding()).unwrap().is_none());
        modal.displayed = true;
        let stale = modal.answer("y", &modal.binding()).unwrap().unwrap();
        let token = modal.view.token().clone();
        driver.modal = Some(modal);
        driver.command("/cancel", 202);
        // UI cancellation is protocol answer data. It is distinct from
        // retiring the producer's command/turn cancellation token.
        let (outcome, cancelled) = tokio::time::timeout(
            Duration::from_secs(10),
            futures_util::future::join(
                control(&mut driver),
                http::reply(&listener, "resources/read", json!({"contents":[]})),
            ),
        )
        .await
        .unwrap();
        assert!(!outcome.failed());
        assert!(cancelled["id"].as_i64().unwrap() > original["id"].as_i64().unwrap());
        let mut params = cancelled["params"].clone();
        assert_eq!(
            params.as_object_mut().unwrap().remove("requestState"),
            Some(Value::Null)
        );
        assert_eq!(
            params.as_object_mut().unwrap().remove("inputResponses"),
            Some(json!({"confirm":{"action":"cancel"}}))
        );
        assert_eq!(params, original["params"]);
        assert!(driver.inbox.reply(&token, stale).is_err());
        let listener = listener.into_std().unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(fixture.transport.requests().is_empty());
        assert_eq!(driver.owner.runtime().status().queued_jobs, 0);
        drop(outcome);
        Box::pin(http::finish_runtime(driver, fixture)).await;
    });
}
