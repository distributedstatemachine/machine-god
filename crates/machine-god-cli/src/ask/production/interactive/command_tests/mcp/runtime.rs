//! Composed literal slash dispatch. Requires the exact fresh release helper.
use super::*;
use serde_json::{Value, json};
use std::time::Instant;
use tokio::net::TcpListener;

#[path = "runtime/support.rs"]
mod http;

#[path = "runtime/continuation.rs"]
mod continuation;

type Case = (
    &'static str,
    native::McpFeatureAction,
    &'static str,
    Value,
    Value,
);

fn cases() -> [Case; 7] {
    [
        (
            "/mcp resource list fixture",
            native::McpFeatureAction::ResourceList,
            "resources/list",
            json!({}),
            http::resources(),
        ),
        (
            "/mcp resource templates fixture",
            native::McpFeatureAction::ResourceTemplates,
            "resources/templates/list",
            json!({}),
            http::templates(),
        ),
        (
            "/mcp resource read fixture test://fixed",
            native::McpFeatureAction::ResourceRead,
            "resources/read",
            json!({"uri":"test://fixed"}),
            json!({"contents":[{"uri":"test://fixed","text":"external resource"}]}),
        ),
        (
            "/mcp prompt list fixture",
            native::McpFeatureAction::PromptList,
            "prompts/list",
            json!({}),
            http::prompts(),
        ),
        (
            r#"/mcp prompt get fixture review {"topic":"rust"}"#,
            native::McpFeatureAction::PromptGet,
            "prompts/get",
            json!({"name":"review","arguments":{"topic":"rust"}}),
            json!({"messages":[{"role":"user","content":{"type":"text","text":"external prompt"}}]}),
        ),
        (
            "/mcp prompt complete fixture review topic ru",
            native::McpFeatureAction::PromptComplete,
            "completion/complete",
            json!({"ref":{"type":"ref/prompt","name":"review"},"argument":{"name":"topic","value":"ru"}}),
            json!({"completion":{"values":["rust","ruby"],"total":2,"hasMore":false}}),
        ),
        (
            "/mcp resource complete fixture test:///{id} id ru",
            native::McpFeatureAction::ResourceComplete,
            "completion/complete",
            json!({"ref":{"type":"ref/resource","uri":"test:///{id}"},"argument":{"name":"id","value":"ru"}}),
            json!({"completion":{"values":["rust"],"total":1,"hasMore":false}}),
        ),
    ]
}

#[test]
fn seven_literal_feature_commands_use_actual_http_owner_and_exact_receipts() {
    executor().block_on(async {
        let (fixture, mut driver, listener) = http::setup().await;
        let record = driver.owner.runtime().record();
        let mut previous = 2;
        for (command, action, method, params, body) in cases() {
            driver.command(command, 200);
            let (outcome, request) = tokio::time::timeout(
                Duration::from_secs(10),
                futures_util::future::join(control(&mut driver), async {
                    // Explicit list commands above do not populate the
                    // dependency cache of a later read/get/completion.
                    match action {
                        native::McpFeatureAction::ResourceRead => {
                            http::reply(&listener, "resources/list", http::resources()).await;
                        }
                        native::McpFeatureAction::PromptGet => {
                            http::reply(&listener, "prompts/list", http::prompts()).await;
                        }
                        native::McpFeatureAction::ResourceComplete => {
                            http::reply(&listener, "resources/templates/list", http::templates())
                                .await;
                        }
                        _ => {}
                    }
                    http::reply(&listener, method, body).await
                }),
            )
            .await
            .unwrap();
            assert!(!outcome.failed());
            let id = request["id"].as_i64().unwrap();
            assert!(id > previous);
            previous = id;
            let mut sent = request["params"].clone();
            sent.as_object_mut().unwrap().remove("_meta");
            assert_eq!(sent, params);
            let Ok(NativeInteractiveControlReceipt::McpFeature(receipt)) = &outcome.result else {
                panic!("native feature receipt")
            };
            assert_eq!(receipt.server(), "fixture");
            assert_eq!(receipt.action(), action);
            assert!(receipt.revalidate().is_ok());
            let mut paging = super::super::super::super::mcp_feature_pages::Paging::default();
            let rendered = paging
                .prepare(
                    outcome.id.get(),
                    action,
                    receipt.server(),
                    receipt.reply(),
                    true,
                )
                .unwrap();
            assert!(!rendered.is_empty());
            assert!(fixture.transport.requests().is_empty());
            assert_eq!(driver.owner.runtime().status().queued_jobs, 0);
            assert_eq!(driver.owner.runtime().record(), record);
            drop(outcome);
        }
        Box::pin(http::finish_runtime(driver, fixture)).await;
    });
}
