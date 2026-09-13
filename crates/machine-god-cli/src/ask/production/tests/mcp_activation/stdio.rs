//! Actual production capture and fresh release-helper stdio acceptance.
use super::super::super::ConversationSetup;
use super::configured_support as support;
use super::*;
use machine_god_native::TokioWebSearchRuntime;
use serde_json::{Value, json};

const REVISION: &str = "captured-stdio";

#[test]
fn captured_stdio_required_startup_discovers_calls_and_joins_the_actual_host() {
    exercise(true);
}

#[test]
fn captured_stdio_optional_first_demand_discovers_calls_and_joins_the_actual_host() {
    exercise(false);
}

fn configured_host(
    directory: &ScopedTestDirectory,
    required: bool,
    transcript: &std::path::Path,
) -> (NativeReferenceHost, Arc<OneShotTransport>) {
    private_file(
        directory,
        "profile/mcp.json",
        &json!({"mcp":{"remote":{
            "command":"/bin/bash",
            "args":["--noprofile","--norc","-c",include_str!("stdio/producer.bash"),
                "mcp-stdio-fixture",transcript,
                json!({"resultType":"complete","supportedVersions":["2026-07-28"],
                    "capabilities":{"tools":{}}}).to_string(),
                json!({"resultType":"complete","tools":[{
                    "name":"lookup","description":format!("Configured lookup {REVISION}"),
                    "inputSchema":{"type":"object","properties":{"revision":{"type":"string"}},
                        "required":["revision"],"additionalProperties":false}
                }],"ttlMs":300_000}).to_string(),
                json!({"resultType":"complete","content":[{"type":"text","text":"stdio-result"}],
                    "structuredContent":{"revision":REVISION,"marker":"captured-stdio-result"}}).to_string()
            ],
            "required":required,"startup_timeout_ms":5000,"operation_timeout_ms":5000
        }}}),
    );
    private_file(
        directory,
        "config/machine-god/config.json",
        &json!({
            "schema_version":7,"permission_mode":"yolo","sandbox_mode":"none",
            "permission_rules":[],"workspace_permission_rules":[],"workspace_directories":[],
            "provider":"vercel_ai_gateway","transport":"ai_gateway_http","credential_source":"environment",
            "model":"test/configured","effort":"auto","fast_mode":false
        }),
    );
    let config = load_native_config(&NativeEnvironment::new(
        Some(directory.path().join("config").into_os_string()),
        None,
        None,
    ))
    .unwrap();
    let provider = Arc::new(OneShotTransport::scripted(support::script(
        "stdio", REVISION,
    )));
    // This uses capture_startup and the explicit fresh release binary unchanged:
    // no test factory or test-selected helper argument can bypass production.
    let host = host_with_provider(directory, true, provider.clone(), config);
    (host, provider)
}

fn exercise(required: bool) {
    let directory = ScopedTestDirectory::new(&format!("mcp-captured-stdio-{required}"));
    let transcript = directory.path().join("stdio-transcript");
    let (host, provider) = configured_host(&directory, required, &transcript);
    let completion = host.terminal_shutdown_completion().unwrap();
    let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
    let construction_was_inert = !transcript.exists() && provider.request_bodies().is_empty();
    let result = with_settled_terminal_host(host, &runtime, |host| {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(receiver);
        runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    mcp_startup::activate(host, NativeMcpStartupPhase::AskStartup, &mut signals),
                )
                .await
            })
            .map_err(|_| ())??;
        let after_startup = fs::read_to_string(&transcript);
        let startup_used_no_provider = provider.request_bodies().is_empty();
        let evidence = turn(host, &runtime, &provider)?;
        Ok((after_startup, startup_used_no_provider, evidence))
    });

    // Even the negative baseline's expected startup failure must settle MCP,
    // drop the host and join its workers before any success assertion fails.
    assert!(completion.is_complete());
    let (after_startup, startup_used_no_provider, evidence) =
        result.expect("production-captured stdio must activate and complete the Ask turn");
    assert!(construction_was_inert);
    assert!(startup_used_no_provider);
    assert_startup(required, after_startup);
    assert_turn(&evidence, &provider);
    assert_transcript(&fs::read_to_string(transcript).unwrap());
}

fn assert_startup(required: bool, after_startup: io::Result<String>) {
    if required {
        let startup = after_startup.expect("required startup must launch the producer");
        assert_eq!(
            startup.lines().count(),
            3,
            "started, discover and tools/list"
        );
    } else {
        assert!(
            matches!(after_startup, Err(error) if error.kind() == io::ErrorKind::NotFound),
            "optional startup must remain deferred"
        );
    }
}

fn assert_turn(evidence: &TurnEvidence, provider: &OneShotTransport) {
    assert_eq!(evidence.outcome, AskCommandOutcome::Completed);
    assert_eq!(evidence.output.bytes, b"configured complete");
    assert_eq!(evidence.output.flushes, 1);
    assert_eq!(evidence.transitions, ["turn", "final"]);
    assert!(evidence.signal.is_none());
    assert_eq!(evidence.record.next_turn_sequence, 2);
    support::assert_search(&evidence.record, "stdio");
    let output = support::persisted(&evidence.record, "stdio-execute");
    assert!(!output.is_error, "{output:?}");
    assert_eq!(output.content["content"][0]["text"], "stdio-result");
    assert_eq!(output.content["structuredContent"]["revision"], REVISION);
    assert_eq!(
        output.content["structuredContent"]["marker"],
        "captured-stdio-result"
    );
    support::assert_projection(provider, 1, REVISION);
    let requests = provider.request_bodies();
    assert_eq!(requests.len(), 4);
    let id = provider.session_ids()[0].clone();
    assert_eq!(provider.session_ids(), vec![id; 4]);
    assert!(String::from_utf8_lossy(&requests[3]).contains("captured-stdio-result"));
}

fn assert_transcript(transcript: &str) {
    let lines: Vec<_> = transcript.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "one launch, discover, list, call and owned EOF"
    );
    assert_eq!(lines[0], "started");
    assert_eq!(
        lines[4], "eof",
        "owned shutdown must close the producer's stdin"
    );
    for (index, method) in ["server/discover", "tools/list", "tools/call"]
        .iter()
        .enumerate()
    {
        let request: Value = serde_json::from_str(lines[index + 1]).unwrap();
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["id"], index + 1);
        assert_eq!(request["method"], *method);
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        if *method == "tools/call" {
            assert_eq!(request["params"]["name"], "lookup");
            assert_eq!(request["params"]["arguments"], json!({"revision":REVISION}));
        }
    }
}

struct TurnEvidence {
    record: SessionRecord,
    outcome: AskCommandOutcome,
    output: RecordingOutput,
    transitions: Vec<&'static str>,
    signal: Option<AskSignal>,
}

fn turn(
    host: &NativeReferenceHost,
    runtime: &TokioWebSearchRuntime,
    provider: &OneShotTransport,
) -> Result<TurnEvidence, ()> {
    std::thread::scope(|scope| {
        let (work, work_receiver) = tokio::sync::mpsc::channel(1);
        let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
        let output = scope.spawn(move || {
            let mut output = RecordingOutput::default();
            serve_output(work_receiver, &ack, &mut output);
            output
        });
        let (sender, mut controls) = tokio::sync::mpsc::channel(1);
        let control = AskSignalControlSender { sender };
        let guardian = scope.spawn(move || {
            let mut transitions = Vec::new();
            while let Some(command) = controls.blocking_recv() {
                match command {
                    AskSignalControl::ActivateTurn(ready) => {
                        transitions.push("turn");
                        ready.send(()).unwrap();
                    }
                    AskSignalControl::EnterFinal(ready) => {
                        transitions.push("final");
                        ready.send(()).unwrap();
                        break;
                    }
                    _ => panic!("unexpected successful-turn guardian transition"),
                }
            }
            transitions
        });
        let (_signal_owner, receiver) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(receiver);
        let setup = ConversationSetup {
            workspace: host.workspace_root().to_owned(),
            model_routes: host.model_routes().unwrap(),
            observations: host.observations().unwrap(),
            catalog: None,
            now_ms: 100,
        };
        let result = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(20),
                execute_turn(
                    host,
                    SessionSelection::CreateGenerated,
                    "configured MCP request".into(),
                    setup,
                    OutputBridge {
                        work,
                        acknowledgements,
                        tape: None,
                    },
                    &mut signals,
                    &control,
                ),
            )
            .await
        });
        let finalized = control.enter_final();
        drop(control);
        let output = output.join().unwrap();
        let transitions = guardian.join().unwrap();
        finalized?;
        let outcome = result.map_err(|_| ())??.outcome;
        let id = provider.session_ids().first().cloned().ok_or(())?;
        let record = runtime
            .block_on(host.session_lifecycle().replay(id))
            .map_err(|_| ())?;
        Ok(TurnEvidence {
            record,
            outcome,
            output,
            transitions,
            signal: signals.first_observed,
        })
    })
}

fn private_file(directory: &ScopedTestDirectory, name: &str, value: &Value) {
    let path = directory.path().join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
