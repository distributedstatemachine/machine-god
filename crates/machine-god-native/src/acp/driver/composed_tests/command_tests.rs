use super::*;

fn command(connection: &mut NativeAcpConnection, rpc: i64, id: &str, text: &str) {
    request(
        connection,
        rpc,
        "session/prompt",
        json!({"sessionId":id,
        "prompt":[{"type":"text","text":text}]}),
    );
}

fn receipt(updates: &[Value]) -> &Value {
    let results: Vec<_> = updates
        .iter()
        .filter(|params| !params["update"]["command_result"].is_null())
        .collect();
    assert_eq!(results.len(), 1);
    assert!(results[0]["update"]["toolCallId"].is_null());
    &results[0]["update"]["command_result"]
}

#[test]
fn local_commands_have_real_receipts_and_never_enter_the_provider() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        command(&mut connection, 3, &id, "/help");
        let (result, updates) = response(&mut connection, 3).await;
        assert_eq!(result.unwrap()["stopReason"], "end_turn");
        assert_eq!(receipt(&updates)["command"], "/help");
        let names = receipt(&updates)["receipt"]["availableCommands"]
            .as_array()
            .unwrap();
        assert!(names.iter().any(|row| row["name"] == "model"));
        assert!(
            !names
                .iter()
                .any(|row| row["name"] == "new" || row["name"] == "upgrade")
        );
        command(&mut connection, 4, &id, "/model example/native-command");
        let (result, updates) = response(&mut connection, 4).await;
        assert_eq!(result.unwrap()["stopReason"], "end_turn");
        assert_eq!(receipt(&updates)["status"], "completed");
        assert_eq!(
            connection
                .selection
                .current()
                .unwrap()
                .runtime()
                .model_preferences()
                .model(),
            "example/native-command"
        );
        match next(&mut connection).await {
            AcpMessage::Notification {
                params: Some(params),
                ..
            } => assert_eq!(
                params["update"]["sessionUpdate"],
                "available_commands_update"
            ),
            other => panic!("expected refreshed capabilities: {other:?}"),
        }
        request(
            &mut connection,
            5,
            "session/resume",
            json!({"sessionId":id,"cwd":factory.workspace}),
        );
        let result = response(&mut connection, 5).await.0.unwrap();
        assert!(
            result["configOptions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == "model" && row["currentValue"] == "example/native-command")
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        shutdown(&mut connection).await;
    });
}

#[test]
fn unsupported_or_foreign_commands_are_not_model_prompts() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        for (rpc, text) in [
            (3, "/new"),
            (4, "/model save-default"),
            (5, "/no-such-command"),
        ] {
            command(&mut connection, rpc, &id, text);
            assert_eq!(
                response(&mut connection, rpc).await.0.unwrap_err().code,
                -32602
            );
        }
        command(&mut connection, 6, "foreign-session", "/help");
        assert!(response(&mut connection, 6).await.0.is_err());
        assert!(!factory.provider_started.load(Ordering::Acquire));
        assert!(connection.command.is_none());
        shutdown(&mut connection).await;
    });
}

#[test]
fn close_drains_old_command_update_and_prompt_reply_before_close_reply() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        command(&mut connection, 3, &id, "/model save");
        let old = connection.command.as_ref().unwrap().principal.clone();
        assert!(connection.command.as_ref().unwrap().owner.has_pending());
        request(&mut connection, 4, "session/close", json!({"sessionId":id}));
        let (result, updates) = response(&mut connection, 3).await;
        assert_eq!(result.unwrap()["stopReason"], "cancelled");
        assert_eq!(receipt(&updates)["cancelled"], true);
        assert!(
            updates
                .iter()
                .filter(|params| !params["update"]["command_result"].is_null())
                .all(|params| params["sessionId"] == old.session_id().as_str())
        );
        response(&mut connection, 4).await.0.unwrap();
        assert!(connection.selection.current().is_none());
        shutdown(&mut connection).await;
    });
}

#[test]
fn replacement_cancels_command_without_rebinding_its_receipt() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        command(&mut connection, 3, &id, "/model save");
        request(
            &mut connection,
            4,
            "session/new",
            json!({"cwd":factory.workspace}),
        );
        cancel(&mut connection, &id);
        let (result, updates) = response(&mut connection, 3).await;
        assert_eq!(result.unwrap()["stopReason"], "cancelled");
        assert_eq!(receipt(&updates)["cancelled"], true);
        assert_eq!(
            updates
                .iter()
                .find(|params| !params["update"]["command_result"].is_null())
                .unwrap()["sessionId"],
            id
        );
        assert_ne!(
            response(&mut connection, 4).await.0.unwrap()["sessionId"],
            id
        );
        assert!(connection.command.is_none());
        assert!(!factory.provider_started.load(Ordering::Acquire));
        shutdown(&mut connection).await;
    });
}

#[test]
fn eof_drains_native_command_without_an_output_consumer() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        command(&mut connection, 3, &id, "/model save");
        connection.begin_shutdown();
        futures_util::future::poll_fn(|cx| {
            let _ = connection.poll_progress(cx, 200);
            if connection.is_closed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        assert!(connection.command.is_none());
        assert!(connection.selection.current().is_none());
        assert!(!factory.provider_started.load(Ordering::Acquire));
    });
}
