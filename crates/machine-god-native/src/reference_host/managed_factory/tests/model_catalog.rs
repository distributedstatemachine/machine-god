use super::*;
use futures_util::StreamExt;
use machine_god_core::{ContentBlock, ModelEvent, TurnEvent};
use serde_json::{Value, json};

struct CatalogTransport(Vec<u8>);
impl AiGatewayModelCatalogTransport for CatalogTransport {
    fn get(
        &self,
        _: AiGatewayModelCatalogRequestAccess,
        _: Instant,
        _: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        Box::pin(async {
            Ok(AiGatewayModelCatalogTransportResponse::new(
                200,
                self.0.clone(),
            ))
        })
    }
    fn wait_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

fn catalog(captured: &[&str], explicit: &[&str]) -> Arc<NativeModelCatalog> {
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::new(CatalogTransport(
            serde_json::to_vec(&json!({"data":[
                {"id":"fixture/captured", "type":"language", "tags":["tool-use"],
                 "reasoning_options":[{"type":"effort","values":captured}]},
                {"id":"fixture/explicit", "type":"language", "tags":["tool-use"],
                 "reasoning_options":[{"type":"effort","values":explicit}]}
            ]}))
            .unwrap(),
        )),
    );
    Arc::new(block_on(provider.list_model_details(CancellationToken::new())).unwrap())
}

fn prepare_parent(f: &FactoryFixture) -> PreparedManagedRuntime {
    let session = block_on(
        f.factory
            .0
            .services
            .session_lifecycle
            .create_generated_with_metadata(
                NativeSessionMetadata::new(&f.host.workspace, 3, NativeSessionOrigin::Cli).unwrap(),
            ),
    )
    .unwrap();
    let authority = &f.factory.0.restoration;
    block_on(f.factory.prepare_parent(
        NativeConversation::from_session(session.clone()).unwrap(),
        ManagedRestorationAuthority {
            workspace: authority.workspace.clone(),
            policy: authority.policy.clone(),
            preferences: authority.preferences.clone(),
        },
        NoticePrincipal {
            id: session.id().to_string(),
            generation: NonZeroU64::MIN,
        },
        f.journal.owner_lease(),
        f.parent_mcp.clone(),
    ))
    .unwrap()
}

fn response_for(f: &FactoryFixture, model: &str, marker: &str) {
    let delta = json!({"type":"text-delta","id":"answer","delta":marker});
    let bytes = format!(
        "data: {delta}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"stop\"}}}}\n\n"
    )
    .into_bytes();
    f.host
        .transport
        .model_responses
        .lock()
        .unwrap()
        .entry(model.into())
        .or_default()
        .push_back(bytes);
}

fn finish_turn(
    f: &FactoryFixture,
    child: &mut PreparedManagedRuntime,
    mut turn: NativeConversationRuntimeTurn,
    marker: &str,
    effort: Option<&str>,
) {
    let mut text = String::new();
    block_on(async {
        while let Some(event) = turn.next().await {
            if let TurnEvent::Model {
                event: ModelEvent::TextDelta { text: delta },
            } = event.unwrap().payload
            {
                text.push_str(&delta);
            }
        }
    });
    // Model-specific responses are selected by the real Gateway model header,
    // so this assertion checks the transport selection, not stored preferences.
    assert_eq!(text, marker);
    let request = f
        .host
        .transport
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(request.get("reasoning"), effort.map(Value::from).as_ref());
    drop(turn);
    let (run, settlement) = child.owner.take_settlement().unwrap();
    block_on(poll_fn(|cx| child.resources.poll_turn_settled(cx, &run))).unwrap();
    settlement.complete().unwrap();
}

fn execute(
    f: &FactoryFixture,
    child: &mut PreparedManagedRuntime,
    model: &str,
    effort: Option<&str>,
    marker: &str,
) {
    response_for(f, model, marker);
    child.runtime.enqueue(marker.into()).unwrap();
    let turn = block_on(child.runtime.start_next(5)).unwrap().unwrap();
    finish_turn(f, child, turn, marker, effort);
}

fn close(child: &mut PreparedManagedRuntime) {
    child.resources.begin_close();
    block_on(poll_fn(|cx| child.resources.poll_closed(cx))).unwrap();
}

#[test]
fn explicit_host_catalog_reaches_created_restored_and_reconfigured_child_requests() {
    let f = FactoryFixture::new();
    let mut parent = prepare_parent(&f);
    let original = catalog(&["high"], &["low"]);
    block_on(crate::session_resume::owned::prepare_managed_candidate(
        f.host.host(),
        &parent.runtime,
        None,
        Some(original.clone()),
        4,
    ))
    .unwrap();

    // Preparing a foreground does not publish its observations to siblings.
    let mut inherited = f.prepare(f.request("catalog-inherited"));
    assert!(inherited.runtime.model_catalog().is_none());
    assert!(parent.runtime.activate_routes());
    assert!(Arc::ptr_eq(
        &inherited.runtime.model_catalog().unwrap(),
        &original
    ));
    execute(
        &f,
        &mut inherited,
        "fixture/captured",
        Some("high"),
        "inherited",
    );

    let mut request = f.request("catalog-explicit");
    request.configuration.model = Some("fixture/explicit".into());
    request.configuration.effort = Some("low".into());
    let mut explicit = f.prepare(request.clone());
    execute(
        &f,
        &mut explicit,
        "fixture/explicit",
        Some("low"),
        "explicit",
    );

    // The manager installs each accepted work item's frozen preferences through
    // this same API before enqueueing its next FIFO item.
    explicit
        .runtime
        .set_model_preferences(
            NativeModelPreferences::new(
                "fixture/captured",
                NativeReasoningEffort::parse("high").unwrap(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
    execute(
        &f,
        &mut explicit,
        "fixture/captured",
        Some("high"),
        "configured",
    );
    close(&mut explicit);
    drop(explicit);

    request.kind = ManagedRuntimePreparationKind::Restore;
    let mut restored = f.prepare(request);
    // Restore keeps the explicit work settings instead of the previously saved
    // child's selected model and acquires the host's current capability source.
    execute(
        &f,
        &mut restored,
        "fixture/explicit",
        Some("low"),
        "restored",
    );
    assert!(restored.runtime.record().messages.iter().any(|message| {
        message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { text } if text == "configured"))
    }));
    close(&mut restored);
    close(&mut inherited);
    close(&mut parent);
}

#[test]
fn catalog_refresh_changes_future_child_requests_but_not_an_admitted_turn() {
    let f = FactoryFixture::new();
    let mut parent = prepare_parent(&f);
    assert!(parent.runtime.activate_routes());
    parent
        .runtime
        .set_model_catalog(catalog(&["high"], &[]))
        .unwrap();
    let mut child = f.prepare(f.request("catalog-refresh"));
    response_for(&f, "fixture/captured", "captured-before-refresh");
    child
        .runtime
        .enqueue("captured before refresh".into())
        .unwrap();
    let turn = block_on(child.runtime.start_next(5)).unwrap().unwrap();
    // This is the same setter used by the foreground's injected catalog cache
    // refresh; it neither changes child preferences nor starts provider work.
    parent
        .runtime
        .set_model_catalog(catalog(&[], &["low"]))
        .unwrap();
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
    finish_turn(
        &f,
        &mut child,
        turn,
        "captured-before-refresh",
        Some("high"),
    );
    execute(
        &f,
        &mut child,
        "fixture/captured",
        None,
        "unsupported-after-refresh",
    );
    parent
        .runtime
        .set_model_catalog(catalog(&["high"], &[]))
        .unwrap();
    execute(
        &f,
        &mut child,
        "fixture/captured",
        Some("high"),
        "supported-again",
    );
    close(&mut child);
    close(&mut parent);
}

#[test]
fn unavailable_catalog_never_infers_support_or_borrows_another_hosts_catalog() {
    let f = FactoryFixture::new();
    let mut parent = prepare_parent(&f);
    assert!(parent.runtime.activate_routes());
    parent
        .runtime
        .set_model_catalog(catalog(&["high"], &[]))
        .unwrap();
    let other = FactoryFixture::new();
    let mut child = other.prepare(other.request("catalog-unavailable"));
    execute(&other, &mut child, "fixture/captured", None, "no-catalog");
    assert_eq!(child.runtime.model_preferences().effort().label(), "high");
    assert!(child.runtime.model_catalog().is_none());
    close(&mut child);
    close(&mut parent);
}
