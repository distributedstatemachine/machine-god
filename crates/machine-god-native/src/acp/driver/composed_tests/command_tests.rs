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

fn config_updates(updates: &[Value]) -> Vec<&Value> {
    updates
        .iter()
        .filter(|params| params["update"]["sessionUpdate"] == "config_option_update")
        .collect()
}

fn assert_config_update(params: &Value, id: &str, expected: &Value) {
    assert_eq!(params["sessionId"], id);
    assert_eq!(params["update"]["sessionUpdate"], "config_option_update");
    assert_eq!(params["update"]["configOptions"], expected["configOptions"]);
    assert_eq!(
        params["update"]["configOptions"].as_array().unwrap().len(),
        2
    );
    assert!(params["update"].get("modes").is_none());
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
        let config = super::super::session_projection::config_response(
            connection.selection.current().unwrap(),
        )
        .unwrap();
        let configs = config_updates(&updates);
        assert_eq!(configs.len(), 1);
        assert_config_update(configs[0], &id, &config);
        assert_eq!(
            config["configOptions"][1]["currentValue"],
            "example/native-command"
        );
        assert!(
            updates
                .iter()
                .position(|params| params == configs[0])
                .unwrap()
                < updates
                    .iter()
                    .position(|params| !params["update"]["command_result"].is_null())
                    .unwrap()
        );
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
fn unchanged_read_only_and_rejected_commands_emit_no_config_update() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        for (index, text) in [
            "/model fixture/main",
            "/model",
            "/model save",
            "/status",
            "/help",
            "/model effort high",
            "/model save-default",
            "/fast",
        ]
        .into_iter()
        .enumerate()
        {
            let rpc = 3 + i64::try_from(index).unwrap();
            command(&mut connection, rpc, &id, text);
            let (_, updates) = response(&mut connection, rpc).await;
            assert!(config_updates(&updates).is_empty(), "{text}");
        }
        assert!(!factory.provider_started.load(Ordering::Acquire));
        shutdown(&mut connection).await;
    });
}

#[test]
fn blocked_output_keeps_changed_command_bound_until_config_acquisition() {
    run(async {
        for replace in [false, true] {
            let factory = Arc::new(Factory::new());
            let (mut connection, id) = connection(factory.clone()).await;
            // Occupy the transport's one acquired frame before admitting work.
            let held = next(&mut connection).await;
            command(&mut connection, 3, &id, "/model fixture/blocked");
            let principal = connection.command.as_ref().unwrap().principal.clone();
            if replace {
                request(
                    &mut connection,
                    4,
                    "session/new",
                    json!({"cwd":factory.workspace}),
                );
            } else {
                request(&mut connection, 4, "session/close", json!({"sessionId":id}));
            }
            futures_util::future::poll_fn(|cx| {
                let _ = connection.poll_progress(cx, 200);
                let current = connection.selection.current().unwrap();
                assert_eq!(current.principal(), principal);
                assert!(current.has_pending_command_control());
                assert!(connection.command.as_ref().unwrap().owner.has_pending());
                if current.runtime().status().model_preferences_pending {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            // Even an already-completed synchronous result must retain the
            // configuration barrier independently of native receipt custody.
            connection.command_progress();
            assert!(
                connection
                    .command
                    .as_ref()
                    .unwrap()
                    .owner
                    .result()
                    .is_some()
            );
            futures_util::future::poll_fn(|cx| {
                let _ = connection.poll_progress(cx, 200);
                assert_eq!(
                    connection.selection.current().unwrap().principal(),
                    principal
                );
                assert!(
                    connection
                        .selection
                        .current()
                        .unwrap()
                        .has_pending_command_control()
                );
                Poll::Ready(())
            })
            .await;
            let expected = super::super::session_projection::config_response(
                connection.selection.current().unwrap(),
            )
            .unwrap();
            drop(held);
            let AcpMessage::Notification {
                params: Some(config),
                ..
            } = next(&mut connection).await
            else {
                panic!("configuration must precede command completion and replacement");
            };
            assert_config_update(&config, &id, &expected);
            assert!(connection.reply.is_none());
            let retained = connection.command.as_ref().unwrap().owner.result().unwrap();
            assert_eq!(retained.principal(), &principal);
            let (result, updates) = response(&mut connection, 3).await;
            assert!(result.is_ok());
            assert!(config_updates(&updates).is_empty());
            assert_eq!(receipt(&updates)["status"], "completed");
            let result = response(&mut connection, 4).await.0.unwrap();
            if replace {
                assert_ne!(result["sessionId"], id);
            } else {
                assert!(connection.selection.current().is_none());
            }
            shutdown(&mut connection).await;
        }
    });
}

#[test]
fn cancelled_changed_model_reports_live_state_without_claiming_rollback() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory).await;
        command(&mut connection, 3, &id, "/model fixture/cancelled");
        cancel(&mut connection, &id);
        let (result, updates) = response(&mut connection, 3).await;
        assert_eq!(result.unwrap()["stopReason"], "cancelled");
        assert_eq!(receipt(&updates)["cancelled"], true);
        let configs = config_updates(&updates);
        assert_eq!(configs.len(), 1);
        let expected = super::super::session_projection::config_response(
            connection.selection.current().unwrap(),
        )
        .unwrap();
        assert_config_update(configs[0], &id, &expected);
        assert_eq!(
            expected["configOptions"][1]["currentValue"],
            "fixture/cancelled"
        );
        shutdown(&mut connection).await;
    });
}

#[test]
fn failed_model_save_reports_accepted_live_state_not_persistence_success() {
    run(async {
        use machine_god_core::SessionStore;
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory).await;
        let store = connection
            .selection
            .current_host()
            .unwrap()
            .session_store()
            .clone();
        let current = connection.selection.current().unwrap();
        let record = store.load(current.id()).await.unwrap().unwrap();
        let revision = record.revision;
        // A distinct durable revision rejects the command's old expected revision.
        store.save(record, Some(revision)).await.unwrap();
        command(&mut connection, 3, &id, "/model fixture/save-conflict");
        let (result, updates) = response(&mut connection, 3).await;
        assert_eq!(result.unwrap()["stopReason"], "end_turn");
        assert_eq!(receipt(&updates)["status"], "failed");
        let configs = config_updates(&updates);
        assert_eq!(configs.len(), 1);
        let expected = super::super::session_projection::config_response(
            connection.selection.current().unwrap(),
        )
        .unwrap();
        assert_config_update(configs[0], &id, &expected);
        assert_eq!(
            expected["configOptions"][1]["currentValue"],
            "fixture/save-conflict"
        );
        assert!(configs[0]["update"].get("persistence").is_none());
        command(&mut connection, 4, &id, "/model save");
        let (_, updates) = response(&mut connection, 4).await;
        assert_eq!(receipt(&updates)["status"], "failed");
        assert!(config_updates(&updates).is_empty());
        shutdown(&mut connection).await;
    });
}

#[test]
fn retired_observation_never_projects_replacement_configuration() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        command(&mut connection, 3, &id, "/model fixture/old");
        response(&mut connection, 3).await.0.unwrap();
        command(&mut connection, 4, &id, "/status");
        let principal = connection.command.as_ref().unwrap().principal.clone();
        request(
            &mut connection,
            5,
            "session/new",
            json!({"cwd":factory.workspace}),
        );
        futures_util::future::poll_fn(|cx| {
            let _ = connection.poll_progress(cx, 200);
            if connection
                .selection
                .current()
                .is_some_and(|current| current.principal() != principal)
            {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        assert_eq!(
            connection
                .selection
                .current()
                .unwrap()
                .runtime()
                .model_preferences()
                .model(),
            "fixture/main"
        );
        let (result, updates) = response(&mut connection, 4).await;
        result.unwrap();
        assert_eq!(receipt(&updates)["receipt"]["model"], "fixture/old");
        assert!(config_updates(&updates).is_empty());
        assert_ne!(
            response(&mut connection, 5).await.0.unwrap()["sessionId"],
            id
        );
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
        assert!(config_updates(&updates).is_empty());
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
        command(&mut connection, 3, &id, "/model fixture/eof");
        assert!(connection.command.as_ref().unwrap().configuration.is_some());
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
