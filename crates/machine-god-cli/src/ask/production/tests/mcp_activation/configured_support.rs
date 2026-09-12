//! Configured production-host acceptance. Run only with the exact fresh helper.
use super::super::super::ConversationSetup;
use super::configured_wire as wire;
use super::*;
use futures_util::{FutureExt, future::join};
use machine_god_core::{ContentBlock, ToolOutput};
use machine_god_native::TokioWebSearchRuntime;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};

pub(super) struct Fixture<'a> {
    pub host: Option<NativeReferenceHost>,
    pub provider: Arc<OneShotTransport>,
    pub runtime: TokioWebSearchRuntime,
    pub listener: TcpListener,
    _directory: &'a ScopedTestDirectory,
}

impl<'a> Fixture<'a> {
    pub fn new(
        directory: &'a ScopedTestDirectory,
        required: bool,
        responses: Vec<Vec<u8>>,
    ) -> Self {
        let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let listener = runtime
            .block_on(TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)))
            .unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        private_file(
            directory,
            "profile/mcp.json",
            &json!({"mcp":{"remote":{
                "type":"http","url":endpoint,"required":required,"startup_timeout_ms":5000,"operation_timeout_ms":5000
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
        let provider = Arc::new(OneShotTransport::scripted(responses));
        let host = host_with_provider(directory, true, provider.clone(), config);
        assert!(host.mcp_controller().is_some());
        assert!(provider.request_bodies().is_empty());
        Self {
            host: Some(host),
            provider,
            runtime,
            listener,
            _directory: directory,
        }
    }

    pub fn host(&self) -> &NativeReferenceHost {
        self.host.as_ref().unwrap()
    }

    pub fn activate(&self, required: bool, revision: &str) -> Option<TcpStream> {
        self.runtime.block_on(async {
            let (_sender, receiver) = tokio::sync::mpsc::channel(1);
            let mut signals = AskSignals::new(receiver);
            let activate =
                mcp_startup::activate(self.host(), NativeMcpStartupPhase::AskStartup, &mut signals);
            let socket = if required {
                let (ready, socket) = tokio::time::timeout(
                    Duration::from_secs(10),
                    join(activate, wire::startup(&self.listener, revision)),
                )
                .await
                .unwrap();
                ready.unwrap();
                Some(socket)
            } else {
                activate.await.unwrap();
                assert!(self.listener.accept().now_or_never().is_none());
                None
            };
            assert!(
                self.host()
                    .mcp_controller()
                    .unwrap()
                    .required_readiness()
                    .is_ok()
            );
            assert!(self.provider.request_bodies().is_empty());
            socket
        })
    }

    pub fn turn<T>(&self, selection: SessionSelection, wire: impl Future<Output = T>) -> T {
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
                workspace: self.host().workspace_root().to_owned(),
                model_routes: self.host().model_routes().unwrap(),
                observations: self.host().observations().unwrap(),
                catalog: None,
                now_ms: 100,
            };
            let (result, wire) = self.runtime.block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(20),
                    join(
                        execute_turn(
                            self.host(),
                            selection,
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
                        wire,
                    ),
                )
                .await
                .unwrap()
            });
            control.enter_final().unwrap();
            drop(control);
            let output = output.join().unwrap();
            assert_eq!(guardian.join().unwrap(), ["turn", "final"]);
            assert_eq!(result.unwrap().outcome, AskCommandOutcome::Completed);
            assert_eq!(output.bytes, b"configured complete");
            assert_eq!(output.flushes, 1);
            assert!(signals.first_observed.is_none());
            wire
        })
    }

    pub fn record(&self, id: SessionId) -> SessionRecord {
        self.runtime
            .block_on(self.host().session_lifecycle().replay(id))
            .unwrap()
    }

    pub fn finish(mut self, subscription: TcpStream) {
        self.shutdown();
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), wire::closed(subscription))
                .await
                .unwrap();
            assert!(self.listener.accept().now_or_never().is_none());
        });
    }

    fn shutdown(&mut self) {
        if let Some(host) = self.host.take() {
            let completion = host.terminal_shutdown_completion().unwrap();
            mcp_startup::settle(&host, &self.runtime).unwrap();
            drop(host);
            completion.wait_on_worker().unwrap();
            assert!(completion.is_complete());
        }
    }
}
impl Drop for Fixture<'_> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn private_file(directory: &ScopedTestDirectory, name: &str, value: &Value) {
    let path = directory.path().join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

pub(super) fn call(id: &str, name: &str, input: &Value) -> Vec<u8> {
    let call = json!({"type":"tool-call","toolCallId":id,"toolName":name,"input":input});
    format!("data: {call}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n").into_bytes()
}

pub(super) fn script(prefix: &str, revision: &str) -> Vec<Vec<u8>> {
    vec![
        call(&format!("{prefix}-search"), "mcp_search_tools", &json!({"query":"remote lookup"})),
        call(&format!("{prefix}-select"), "mcp_select_tool", &json!({"name":wire::NAME})),
        call(&format!("{prefix}-execute"), wire::NAME, &json!({"revision":revision})),
        b"data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"configured complete\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n".to_vec(),
    ]
}

pub(super) fn persisted(record: &SessionRecord, id: &str) -> ToolOutput {
    record
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult { call_id, output } if call_id.as_str() == id => {
                Some(output.clone())
            }
            _ => None,
        })
        .unwrap()
}

pub(super) fn assert_search(record: &SessionRecord, prefix: &str) {
    let result = persisted(record, &format!("{prefix}-search"));
    assert!(!result.is_error);
    assert_eq!(result.content["count"], 1);
    assert_eq!(result.content["tools"][0]["name"], wire::NAME);
    assert!(!persisted(record, &format!("{prefix}-select")).is_error);
}

pub(super) fn archive(record: &SessionRecord, prefix: &str) -> Value {
    let result = persisted(record, &format!("{prefix}-execute"));
    assert!(!result.is_error);
    assert_eq!(result.content["type"], "tool_result_archive");
    assert!(serde_json::to_vec(&result.content).unwrap().len() < 4096);
    result.content["archive"]["handle"].clone()
}

pub(super) fn assert_projection(provider: &OneShotTransport, select_round: usize, revision: &str) {
    let requests = provider.request_bodies();
    for (index, bytes) in requests.iter().enumerate() {
        let body = machine_god_core::json::from_slice(bytes).unwrap();
        let tool = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == wire::NAME);
        assert_eq!(tool.is_some(), index > select_round);
        if let Some(tool) = tool {
            assert!(serde_json::to_string(tool).unwrap().contains(revision));
        }
    }
}
