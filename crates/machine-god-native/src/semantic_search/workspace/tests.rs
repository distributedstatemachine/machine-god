use super::*;
use crate::list_files::enumeration_fixture::{Allow, Fixture, context};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{SessionIncarnationId, ToolCallId};
use serde_json::json;
use std::sync::Arc;

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("semantic").unwrap(),
        name: semantic_search_name(),
        arguments,
    }
}

fn owner(fixture: &Fixture) -> crate::NativeConversation {
    fixture.conversation(
        crate::GrepFilesTool::open(&fixture.primary).unwrap(),
        vec![],
        Arc::new(Allow::default()),
    )
}

#[test]
fn preparation_preserves_two_field_envelope_and_qualified_capability_without_io() {
    let fixture = Fixture::new("semantic-prepare");
    let conversation = owner(&fixture);
    let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let default = prepare(
        &fixture.contexts,
        &key,
        call(json!({"query":"  The\tNeedle  "})),
    )
    .unwrap();
    assert_eq!(
        default.arguments(),
        &json!({"query":"  The\tNeedle  ","path":"."})
    );
    assert_eq!(
        default.capability(),
        Some(&Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: ".".into()
        })
    );
    let missing = fixture.additional.join("absent/file");
    let qualified = prepare(&fixture.contexts,&key,call(json!({"query":"needle","path":format!("{}/./absent/file",fixture.additional.display())}))).unwrap();
    assert_eq!(
        qualified.arguments(),
        &json!({"query":"needle","path":missing})
    );
    assert_eq!(
        qualified.capability(),
        Some(&Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: missing.to_str().unwrap().into()
        })
    );
    // Ordinary permission registration reparses the actual canonical envelope.
    let again = prepare(&fixture.contexts, &key, call(qualified.arguments().clone())).unwrap();
    assert_eq!(qualified.arguments(), again.arguments());
    assert_eq!(qualified.capability(), again.capability());
    let scope = fixture.contexts.snapshot_for_tool(&key).unwrap();
    let args = decode_execution_arguments(qualified.arguments().clone()).unwrap();
    assert_eq!(canonical_route(&scope, &args).unwrap().1, "absent/file");
}

#[test]
fn strict_scoped_paths_envelopes_and_expired_contexts_never_fallback() {
    let fixture = Fixture::new("semantic-invalid");
    let conversation = owner(&fixture);
    let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    for path in [
        fixture.base.clone(),
        fixture.base.join("state"),
        fixture.additional.join("../primary"),
    ] {
        assert!(
            prepare(
                &fixture.contexts,
                &key,
                call(json!({"query":"needle","path":path}))
            )
            .is_err()
        );
    }
    for path in ["", "x\n", "x\u{202e}"] {
        assert!(
            prepare(
                &fixture.contexts,
                &key,
                call(json!({"query":"needle","path":path}))
            )
            .is_err()
        );
    }
    for arguments in [
        json!({"query":"needle","path":null}),
        json!({"query":"needle","extra":true}),
        json!({"query":null}),
    ] {
        assert!(prepare(&fixture.contexts, &key, call(arguments)).is_err());
    }
    let mut foreign = key.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(prepare(&fixture.contexts, &foreign, call(json!({"query":"needle"}))).is_err());
    assert!(
        prepare(
            &NativeWorkspaceContexts::new(),
            &key,
            call(json!({"query":"needle"}))
        )
        .is_err()
    );
    let scope = fixture.contexts.snapshot_for_tool(&key).unwrap();
    let noncanonical = ExecutionArguments {
        query: "needle".into(),
        path: "./".into(),
    };
    assert!(canonical_route(&scope, &noncanonical).is_err());
    drop(turn);
    assert_eq!(
        execute(
            Ok(scope),
            json!({"query":"needle","path":"."}),
            &CancellationToken::new()
        )
        .unwrap_err()
        .code,
        "workspace_context_unavailable"
    );
}

#[test]
fn pure_scope_uses_the_taken_root_set_not_a_later_install() {
    let fixture = Fixture::new("semantic-snapshot");
    let conversation = owner(&fixture);
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    let arguments = json!({"query":"needle","path":fixture.additional});
    assert!(prepare(&fixture.contexts, &key, call(arguments.clone())).is_ok());
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    assert!(
        prepare(
            &fixture.contexts,
            &context(&conversation, &next),
            call(arguments)
        )
        .is_err()
    );
}

#[cfg(target_os = "linux")]
mod linux {
    use super::super::super::{
        ScanCheck, SemanticSearchTool, checked_workspace_path_length, join_workspace_path,
    };
    use super::*;
    use machine_god_core::Tool;

    fn tool(fixture: &Fixture) -> SemanticSearchTool {
        SemanticSearchTool::open(&fixture.primary)
            .unwrap()
            .with_workspace_contexts(fixture.contexts.clone())
    }
    fn run(
        tool: &SemanticSearchTool,
        key: &ToolContext,
        args: Value,
    ) -> Result<ToolOutput, ToolError> {
        let prepared = tool.prepare_for_turn(key, call(args))?;
        block_on(tool.execute(
            key.clone(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ))
    }

    #[test]
    fn real_engine_uses_exact_additional_permission_and_qualified_ranked_output() {
        let fixture = Fixture::new("semantic-engine");
        let policy = Arc::new(Allow::default());
        let conversation = fixture.conversation(
            tool(&fixture),
            vec![call(
                json!({"query":"additional","path":fixture.additional}),
            )],
            policy.clone(),
        );
        let events = block_on(
            block_on(conversation.prompt("search".into(), 1))
                .unwrap()
                .collect::<Vec<_>>(),
        );
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        let output = format!("{events:?}");
        assert!(
            output.contains(fixture.additional.join("additional.txt").to_str().unwrap()),
            "{output}"
        );
        assert!(!output.contains("primary.txt"));
        assert_eq!(
            *policy.0.lock().unwrap(),
            vec![Capability::Filesystem {
                access: FilesystemAccess::SearchContent,
                path: fixture.additional.to_str().unwrap().into()
            }]
        );
    }

    #[test]
    fn default_is_primary_only_and_selected_file_preserves_scoring() {
        let fixture = Fixture::new("semantic-selection");
        let file = fixture.additional.join("match.txt");
        std::fs::write(&file, "needle one\nneedle two\n").unwrap();
        let conversation = owner(&fixture);
        let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
        let key = context(&conversation, &turn);
        let tool = tool(&fixture);
        assert_eq!(
            run(&tool, &key, json!({"query":"needle"})).unwrap().content["results"],
            json!([])
        );
        let output = run(&tool, &key, json!({"query":"needle","path":file})).unwrap();
        assert_eq!(output.content["results"][0]["path"], file.to_str().unwrap());
        assert_eq!(output.content["results"][0]["score"], 2);
        assert_eq!(output.content["results"][0]["line_number"], 1);
        assert_eq!(output.content["results"][0]["line"], "needle one");
        assert_eq!(output.content["candidate_files"], 1);
    }

    #[test]
    fn ordinary_permission_evidence_matches_the_same_canonical_scoped_tool() {
        use machine_god_core::{
            PermissionInvocation, PermissionRequest, PermissionRequestId, PermissionRisk,
        };
        let fixture = Fixture::new("semantic-permission");
        let conversation = owner(&fixture);
        let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
        let key = context(&conversation, &turn);
        let tool = Arc::new(tool(&fixture));
        let prepared = tool
            .prepare_for_turn(
                &key,
                call(json!({"query":"additional","path":fixture.additional})),
            )
            .unwrap();
        let authority = crate::NativePermissionTargetAuthority::new(
            std::fs::File::open(&fixture.primary).unwrap(),
            fixture.primary.to_str().unwrap().into(),
            vec![crate::NativePermissionTargetTool::Ordinary(tool)],
        )
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone());
        let mut request = PermissionRequest {
            id: PermissionRequestId::new("search-request").unwrap(),
            session_id: key.session_id.clone(),
            session_incarnation_id: key.session_incarnation_id.clone(),
            turn_id: key.turn_id.clone(),
            capability: prepared.capability().unwrap().clone(),
            risk: PermissionRisk::Low,
            reason: "search".into(),
        };
        let name = semantic_search_name();
        let invocation = PermissionInvocation {
            tool_name: &name,
            call_id: &key.call_id,
            arguments: prepared.arguments(),
        };
        let evidence =
            block_on(authority.prepare(&request, invocation, CancellationToken::new())).unwrap();
        assert_eq!(evidence.targets().len(), 1);
        assert_eq!(
            evidence.targets()[0].path(),
            fixture.additional.to_str().unwrap()
        );
        evidence.revalidate().unwrap();
        request.capability = Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: ".".into(),
        };
        assert!(
            block_on(authority.prepare(&request, invocation, CancellationToken::new())).is_err()
        );
        drop(turn);
        assert!(evidence.revalidate().is_err());
    }

    #[test]
    fn retained_roots_and_outer_future_stamps_cannot_retarget() {
        let fixture = Fixture::new("semantic-retain");
        let conversation = owner(&fixture);
        let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
        let key = context(&conversation, &turn);
        let tool = tool(&fixture);
        let prepared = tool
            .prepare_for_turn(
                &key,
                call(json!({"query":"additional","path":fixture.additional})),
            )
            .unwrap();
        let future = tool.execute(
            key.clone(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        );
        std::fs::rename(&fixture.additional, fixture.base.join("retained")).unwrap();
        std::fs::create_dir(&fixture.additional).unwrap();
        std::fs::write(fixture.additional.join("replacement.txt"), "additional").unwrap();
        let output = block_on(future).unwrap();
        assert_eq!(
            output.content["results"][0]["path"],
            fixture.additional.join("additional.txt").to_str().unwrap()
        );
        let stale = tool.execute(key, prepared.arguments().clone(), CancellationToken::new());
        drop(turn);
        assert_eq!(
            block_on(stale).unwrap_err().code,
            "workspace_context_unavailable"
        );
    }

    #[test]
    fn stopword_fast_path_never_invokes_even_the_descriptor_acquisition_closure() {
        let fixture = Fixture::new("semantic-empty");
        let conversation = owner(&fixture);
        let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
        let key = context(&conversation, &turn);
        let scope = fixture.contexts.snapshot_for_tool(&key).unwrap();
        let cancellation = CancellationToken::new();
        let check = ScopedCheck {
            scope: &scope,
            cancellation: &cancellation,
        };
        let args = ExecutionArguments {
            query: "the and it".into(),
            path: fixture.additional.join("absent").to_str().unwrap().into(),
        };
        let output = SemanticSearchTool::execute_scan(&args, &check, || {
            panic!("stopword query must not acquire a descriptor")
        })
        .unwrap();
        assert_eq!(output.content["keywords"], json!([]));
        assert_eq!(output.content["visited_entries"], 0);
        let output = run(&tool(&fixture), &key, args.as_json()).unwrap();
        assert_eq!(output.content["candidate_files"], 0);
    }

    #[test]
    fn qualified_ranking_retains_one_global_heap_and_original_caps() {
        let fixture = Fixture::new("semantic-ranking");
        std::fs::remove_file(fixture.additional.join("additional.txt")).unwrap();
        for index in 0..250 {
            std::fs::write(
                fixture.additional.join(format!("{index:03}.txt")),
                "needle\n",
            )
            .unwrap();
        }
        let conversation = owner(&fixture);
        let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
        let output = run(
            &tool(&fixture),
            &context(&conversation, &turn),
            json!({"query":"needle","path":fixture.additional}),
        )
        .unwrap();
        assert_eq!(output.content["matching_files"], 250);
        assert_eq!(output.content["results"].as_array().unwrap().len(), 100);
        assert_eq!(
            output.content["results"][0]["path"],
            fixture.additional.join("000.txt").to_str().unwrap()
        );
        assert_eq!(
            output.content["results"][99]["path"],
            fixture.additional.join("099.txt").to_str().unwrap()
        );
        assert_eq!(
            output.content["incomplete_reasons"],
            json!(["result_cap", "output_cap"])
        );
        assert_eq!(join_workspace_path("/", "child").unwrap(), "/child");
        assert_eq!(checked_workspace_path_length("/", 5).unwrap(), 6);
    }

    #[test]
    fn turn_cancellation_interrupts_shared_real_scan_at_existing_checkpoint() {
        use std::cell::Cell;
        struct Revoke<'a> {
            checks: Cell<usize>,
            turn: machine_god_core::TurnHandle,
            check: ScopedCheck<'a>,
        }
        impl ScanCheck for Revoke<'_> {
            fn check(&self) -> Result<(), ToolError> {
                self.checks.set(self.checks.get() + 1);
                if self.checks.get() == 20 {
                    assert!(self.turn.cancel());
                }
                self.check.check()
            }
        }
        let fixture = Fixture::new("semantic-cancel");
        std::fs::write(fixture.additional.join("long.txt"), "needle\n".repeat(1000)).unwrap();
        let conversation = owner(&fixture);
        let turn = block_on(conversation.prompt("search".into(), 1)).unwrap();
        let key = context(&conversation, &turn);
        let scope = fixture.contexts.snapshot_for_tool(&key).unwrap();
        let cancellation = CancellationToken::new();
        let check = Revoke {
            checks: Cell::new(0),
            turn: turn.handle(),
            check: ScopedCheck {
                scope: &scope,
                cancellation: &cancellation,
            },
        };
        let args = ExecutionArguments {
            query: "needle".into(),
            path: fixture.additional.to_str().unwrap().into(),
        };
        let (route, relative) = canonical_route(&scope, &args).unwrap();
        let root =
            SemanticSearchTool::from_root_descriptor(route.root_descriptor().try_clone().unwrap());
        let result = SemanticSearchTool::execute_scan(&args, &check, || {
            root.open_search_root(&relative, &check)
        });
        assert_eq!(result.unwrap_err().code, "workspace_context_unavailable");
        assert_eq!(check.checks.get(), 20);
        assert!(!cancellation.is_cancelled());
    }
}
