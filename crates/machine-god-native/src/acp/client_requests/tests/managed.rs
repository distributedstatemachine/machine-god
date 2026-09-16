use super::*;

fn named(name: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(name).unwrap(),
        SessionIncarnationId::new("life").unwrap(),
    )
}

#[test]
fn native_parent_replacement_preserves_child_form_rpc_and_original_reply() {
    let mut connection = NativeAcpClientRequests::new().unwrap();
    let parent = connection.inbox.register(owner()).unwrap();
    let _child = connection.inbox.register(named("child")).unwrap();
    let contexts = Arc::new(NativePermissionContexts::new());
    connection
        .activate_native(owner(), contexts.clone())
        .unwrap();
    assert!(connection.principal.is_none());
    let presenter = connection.presenter();
    let mut pending =
        presenter.present(request_for(named("child"), false), CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let child_rpc = id(frame(&mut connection));
    drop(parent);
    let _replacement = connection.inbox.register(named("next-parent")).unwrap();
    connection
        .activate_native(named("next-parent"), contexts)
        .unwrap();
    connection
        .reply(
            &child_rpc,
            Ok(json!({"action":"accept","content":{"answer":"child answer"}})),
        )
        .unwrap();
    assert!(matches!(poll(&mut pending), Poll::Ready(Ok(_))));
    assert_eq!(
        connection.reply(&child_rpc, Ok(json!({"action":"cancel"}))),
        Err(NativeAcpClientRequestError::Stale)
    );
}

#[test]
fn queued_child_url_completion_keeps_child_session_across_parent_replacement() {
    let mut connection = NativeAcpClientRequests::new().unwrap();
    let parent = connection.inbox.register(owner()).unwrap();
    let _child = connection.inbox.register(named("child")).unwrap();
    let contexts = Arc::new(NativePermissionContexts::new());
    connection
        .activate_native(owner(), contexts.clone())
        .unwrap();
    let presenter = connection.presenter();
    let request = request_for(named("child"), true);
    let completion = presenter.register(&request).unwrap();
    let url_id = presenter.id_for(&request).unwrap();
    let mut pending = presenter.present(request, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let rpc = id(frame(&mut connection));
    connection
        .reply(&rpc, Ok(json!({"action":"accept"})))
        .unwrap();
    assert!(matches!(poll(&mut pending), Poll::Ready(Ok(_))));
    completion.finish(McpClientUrlOutcome::Completed);
    drop(parent);
    let _replacement = connection.inbox.register(named("next-parent")).unwrap();
    connection
        .activate_native(named("next-parent"), contexts)
        .unwrap();
    let Poll::Ready(Ok(Some(bytes))) =
        connection.poll_complete(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("child completion");
    };
    let AcpMessage::Notification {
        method,
        params: Some(params),
    } = protocol::decode_frame(&bytes).unwrap()
    else {
        panic!("notification");
    };
    assert_eq!(method, "elicitation/complete");
    assert_eq!(params["sessionId"], "child");
    assert_eq!(params["elicitationId"], url_id.to_string());
}

#[test]
fn native_same_id_replacement_cannot_accept_an_old_form_reply() {
    let mut connection = NativeAcpClientRequests::new().unwrap();
    let parent = connection.inbox.register(owner()).unwrap();
    let contexts = Arc::new(NativePermissionContexts::new());
    connection
        .activate_native(owner(), contexts.clone())
        .unwrap();
    let presenter = connection.presenter();
    let mut old = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut old).is_pending());
    let old_rpc = id(frame(&mut connection));
    drop(parent);
    let _replacement = connection.inbox.register(owner()).unwrap();
    connection.activate_native(owner(), contexts).unwrap();
    assert_eq!(
        connection.reply(
            &old_rpc,
            Ok(json!({"action":"accept","content":{"answer":"old"}}))
        ),
        Err(NativeAcpClientRequestError::Stale)
    );
    assert!(matches!(poll(&mut old), Poll::Ready(Err(_))));
    let mut new = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut new).is_pending());
    let new_rpc = id(frame(&mut connection));
    assert_ne!(old_rpc, new_rpc);
    connection
        .reply(
            &new_rpc,
            Ok(json!({"action":"accept","content":{"answer":"new"}})),
        )
        .unwrap();
    assert!(matches!(poll(&mut new), Poll::Ready(Ok(_))));
}

#[test]
fn native_activation_requires_an_existing_registration_without_replacing_the_old_one() {
    let mut connection = connection();
    let old_epoch = connection.epoch;
    assert_eq!(
        connection.activate_native(named("missing"), Arc::new(NativePermissionContexts::new())),
        Err(NativeAcpClientRequestError::Stale)
    );
    assert_eq!(connection.epoch, old_epoch);
    assert_eq!(connection.owner, Some(owner()));
    assert!(connection.principal.is_some());
}
