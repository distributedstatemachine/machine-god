use super::*;
use futures_util::{StreamExt, stream};
use machine_god_core::{InferenceOptions, ToolSpec, TurnId};
use serde_json::json;

fn request(advertised: &[&str]) -> ModelRequest {
    ModelRequest {
        session_id: SessionId::new("owner").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "request".into(),
            }],
        }],
        tools: advertised
            .iter()
            .map(|name| ToolSpec {
                name: ToolName::new(*name).unwrap(),
                description: String::new(),
                input_schema: json!({}),
            })
            .collect(),
        options: InferenceOptions::default(),
    }
}

fn configured(bytes: usize, nodes: usize) -> BTreeMap<ToolName, AiGatewayToolInputLimits> {
    [(
        ToolName::new("terminal").unwrap(),
        AiGatewayToolInputLimits {
            max_argument_bytes: bytes,
            max_json_nodes: nodes,
        },
    )]
    .into()
}

fn decode(
    events: &[Value],
    limits: AiGatewayLimits,
    active: &[&str],
    bytes: usize,
    nodes: usize,
) -> Vec<Result<ModelEvent, ProviderError>> {
    let inputs =
        ResponseInputLimits::for_request(&request(active), limits, &configured(bytes, nodes))
            .unwrap();
    let wire: Vec<u8> = events
        .iter()
        .flat_map(|event| format!("data: {event}\n").into_bytes())
        .collect();
    let chunks: Vec<_> = wire
        .chunks(limits.max_chunk_bytes.min(16384))
        .map(|chunk| Ok(chunk.to_vec()))
        .collect();
    let events = GatewayEventStream::with_inputs(
        Box::pin(stream::iter(chunks)),
        &CancellationToken::new(),
        limits,
        inputs,
    );
    futures_executor::block_on(events.collect())
}

fn call(id: &str, name: &str, input: Value) -> Value {
    let mut event = json!({"type":"tool-call","toolCallId":id,"toolName":name});
    event["input"] = input;
    event
}

fn finish() -> Value {
    json!({"type":"finish","finishReason":{"unified":"tool-calls"}})
}

fn successful(events: &[Result<ModelEvent, ProviderError>]) -> bool {
    matches!(events.last(), Some(Ok(ModelEvent::Stop { .. }))) && events.iter().all(Result::is_ok)
}

fn ordinary() -> AiGatewayLimits {
    AiGatewayLimits {
        max_tool_arguments_bytes: 8,
        ..AiGatewayLimits::default()
    }
}

#[test]
fn exact_and_one_over_argument_bytes_for_object_and_serialized_final() {
    for length in [24, 25] {
        let input = json!({"x":"a".repeat(length)});
        assert_eq!(serde_json::to_vec(&input).unwrap().len(), length + 8);
        for input in [input.clone(), Value::String(input.to_string())] {
            let output = decode(
                &[call("one", "terminal", input), finish()],
                ordinary(),
                &["terminal"],
                32,
                10,
            );
            assert_eq!(successful(&output), length == 24);
        }
    }
}

#[test]
fn exact_and_one_over_nodes_for_final_object_array_and_string() {
    for count in [4, 5] {
        let input = Value::Array(vec![Value::Null; count]);
        let object = json!({"values": vec![Value::Null; count - 1]});
        for input in [
            input.clone(),
            Value::String(input.to_string()),
            object.clone(),
            Value::String(object.to_string()),
        ] {
            let output = decode(
                &[call("one", "terminal", input), finish()],
                ordinary(),
                &["terminal"],
                100,
                5,
            );
            assert_eq!(successful(&output), count == 4);
        }
    }
}

#[test]
fn ordinary_unknown_and_unadvertised_tools_do_not_borrow_override() {
    let input = json!({"x":"a".repeat(24)});
    for (name, active) in [
        ("ordinary", vec!["terminal", "ordinary"]),
        ("unknown", vec!["terminal"]),
        ("terminal", vec!["ordinary"]),
        ("terminal", vec![]),
    ] {
        assert!(!successful(&decode(
            &[call("one", name, input.clone()), finish()],
            ordinary(),
            &active,
            32,
            10
        )));
    }
    assert!(successful(&decode(
        &[call("one", "ordinary", json!({})), finish()],
        ordinary(),
        &["terminal", "ordinary"],
        32,
        10
    )));
}

fn streamed(input: &str, final_event: Value) -> Vec<Value> {
    vec![
        json!({"type":"tool-input-start","id":"provisional","toolName":"terminal"}),
        json!({"type":"tool-input-delta","id":"provisional","delta":input}),
        json!({"type":"tool-input-end","id":"provisional"}),
        final_event,
        finish(),
    ]
}

#[test]
fn streamed_bytes_nodes_and_authoritative_name_resolution() {
    for length in [24, 25] {
        let input = json!({"x":"a".repeat(length)}).to_string();
        let output = decode(
            &streamed(
                &input,
                json!({"type":"tool-call","toolCallId":"provisional"}),
            ),
            ordinary(),
            &["terminal"],
            32,
            10,
        );
        assert_eq!(successful(&output), length == 24);
    }
    for count in [4, 5] {
        let input = Value::Array(vec![Value::Null; count]).to_string();
        let output = decode(
            &streamed(
                &input,
                json!({"type":"tool-call","toolCallId":"provisional"}),
            ),
            ordinary(),
            &["terminal"],
            100,
            5,
        );
        assert_eq!(successful(&output), count == 4);
    }
    let input = json!({"x":"a".repeat(24)});
    for final_event in [
        call("provisional", "ordinary", input.clone()),
        json!({"type":"tool-call","toolCallId":"other","input":input}),
    ] {
        assert!(!successful(&decode(
            &streamed(&input.to_string(), final_event),
            ordinary(),
            &["terminal"],
            32,
            10
        )));
    }
    for final_event in [
        call("other", "terminal", input.clone()),
        json!({"type":"tool-call","toolCallId":"provisional","input":input}),
    ] {
        assert!(successful(&decode(
            &streamed(&input.to_string(), final_event),
            ordinary(),
            &["terminal"],
            32,
            10
        )));
    }
    assert!(!successful(&decode(
        &[
            json!({"type":"tool-call","toolCallId":"one","input":input}),
            finish()
        ],
        ordinary(),
        &["terminal"],
        32,
        10
    )));
}

#[test]
fn late_delta_tombstones_keep_resolved_limits_without_name_borrowing() {
    let input = json!({"x":"a".repeat(24)});
    for (name, delta, expected) in [
        ("terminal", "x".repeat(32), true),
        ("terminal", "x".repeat(33), false),
        ("ordinary", "x".repeat(9), false),
    ] {
        let final_input = if name == "ordinary" {
            json!({})
        } else {
            input.clone()
        };
        let events = vec![
            json!({"type":"tool-input-start","id":"provisional","toolName":name}),
            call("provisional", name, final_input),
            json!({"type":"tool-input-delta","id":"provisional","toolName":"terminal","delta":delta}),
            json!({"type":"tool-input-end","id":"provisional"}),
            finish(),
        ];
        assert_eq!(
            successful(&decode(
                &events,
                ordinary(),
                &["terminal", "ordinary"],
                32,
                10
            )),
            expected
        );
    }
}

#[test]
fn active_input_framing_handles_outer_escaping_without_expanding_other_records() {
    let input = json!({"x":"\"\\\n".repeat(300_000)});
    let bytes = input.to_string().len();
    assert!(bytes > 1024 * 1024);
    let events = streamed(
        &input.to_string(),
        call("provisional", "terminal", Value::String(input.to_string())),
    );
    assert!(successful(&decode(
        &events,
        AiGatewayLimits::default(),
        &["terminal"],
        bytes,
        10
    )));
    assert!(!successful(&decode(
        &events,
        AiGatewayLimits::default(),
        &[],
        bytes,
        10
    )));
    for event in [
        json!({"type":"text-delta","delta":"a".repeat(1024*1024)}),
        json!({"type":"unknown","input":input}),
    ] {
        assert!(!successful(&decode(
            &[event, finish()],
            AiGatewayLimits::default(),
            &["terminal"],
            bytes,
            10
        )));
    }
}

#[test]
fn full_terminal_input_bytes_and_nodes_fit_real_default_record_limits() {
    const BYTES: usize = 16_378_880;
    const NODES: usize = 2_308_160;
    let input = json!({"x":"x".repeat(BYTES - 8)});
    assert!(successful(&decode(
        &streamed(
            &input.to_string(),
            call("provisional", "terminal", Value::String(input.to_string()))
        ),
        AiGatewayLimits::default(),
        &["terminal"],
        BYTES,
        NODES
    )));
    let input = Value::Array(vec![Value::Null; NODES - 1]);
    assert!(successful(&decode(
        &[call("one", "terminal", input), finish()],
        AiGatewayLimits::default(),
        &["terminal"],
        BYTES,
        NODES
    )));
}

#[test]
fn ordinary_frames_keep_exact_total_record_and_node_caps_with_active_override() {
    let frame = format!(
        "data: {}\n",
        json!({"type":"text-delta","delta":"ordinary"})
    );
    let limits = AiGatewayLimits {
        max_total_response_bytes: frame.len() * 2,
        ..ordinary()
    };
    let inputs =
        ResponseInputLimits::for_request(&request(&["terminal"]), limits, &configured(1000, 10))
            .unwrap();
    let mut decoder = GatewayEventStream::with_inputs(
        Box::pin(stream::empty()),
        &CancellationToken::new(),
        limits,
        inputs,
    );
    decoder.consume_chunk(frame.as_bytes()).unwrap();
    decoder.consume_chunk(frame.as_bytes()).unwrap();
    assert!(decoder.consume_chunk(b"\n").is_err());

    let limits = AiGatewayLimits {
        max_record_bytes: 128,
        max_undecoded_bytes: 128,
        max_json_nodes: 16,
        ..ordinary()
    };
    for event in [
        json!({"type":"unknown","payload":"x".repeat(128)}),
        json!({"type":"unknown","payload":vec![Value::Null;16]}),
    ] {
        assert!(
            decode(&[event], limits, &["terminal"], 1000, 100)
                .iter()
                .any(Result::is_err)
        );
    }
}

#[test]
fn configured_stream_and_final_call_counts_remain_independent() {
    let limits = AiGatewayLimits {
        max_tool_calls: 1,
        max_streamed_tool_calls: 1,
        ..ordinary()
    };
    assert!(!successful(&decode(
        &[
            call("one", "terminal", json!({})),
            call("two", "terminal", json!({})),
            finish()
        ],
        limits,
        &["terminal"],
        1000,
        100
    )));
    let events = [
        json!({"type":"tool-input-start","id":"one","toolName":"terminal"}),
        json!({"type":"tool-input-start","id":"two","toolName":"terminal"}),
    ];
    assert!(
        decode(&events, limits, &["terminal"], 1000, 100)
            .iter()
            .any(Result::is_err)
    );
}

#[test]
fn public_provider_activates_only_current_advertisement_and_remains_inert() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Replay {
        calls: AtomicUsize,
    }
    impl AiGatewayTransport for Replay {
        fn stream(
            &self,
            _: AiGatewayTransportRequest,
            _: CancellationToken,
        ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                let wire = format!(
                    "data: {}\ndata: {}\n",
                    call("one", "terminal", json!({"x":"a".repeat(24)})),
                    finish()
                );
                Ok(Box::pin(stream::iter([Ok(wire.into_bytes())])) as AiGatewayByteStream)
            })
        }
    }
    let transport = Arc::new(Replay {
        calls: AtomicUsize::new(0),
    });
    let provider = AiGatewayProvider::with_limits("test/model", transport.clone(), ordinary())
        .unwrap()
        .with_tool_input_limits(configured(32, 10))
        .unwrap();
    drop(provider.stream(request(&["terminal"]), CancellationToken::new()));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    for (names, expected) in [
        (vec!["terminal"], true),
        (vec!["ordinary"], false),
        (vec![], false),
    ] {
        let output = futures_executor::block_on(async {
            provider
                .stream(request(&names), CancellationToken::new())
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
        });
        assert_eq!(successful(&output), expected);
    }
    assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn final_and_streamed_aggregate_capacity_is_not_multiplied_by_overrides() {
    let limits = AiGatewayLimits {
        max_tool_calls: 4,
        max_streamed_tool_calls: 4,
        ..ordinary()
    };
    let input = json!({"x":"a".repeat(24)});
    assert!(!successful(&decode(
        &[
            call("one", "terminal", input.clone()),
            call("two", "terminal", input.clone()),
            finish()
        ],
        limits,
        &["terminal"],
        32,
        10
    )));
    let events = vec![
        json!({"type":"tool-input-start","id":"one","toolName":"terminal"}),
        json!({"type":"tool-input-delta","id":"one","delta":input.to_string()}),
        json!({"type":"tool-input-start","id":"two","toolName":"terminal"}),
        json!({"type":"tool-input-delta","id":"two","delta":"{}"}),
    ];
    assert!(
        decode(&events, limits, &["terminal"], 32, 10)
            .iter()
            .any(Result::is_err)
    );
}

struct Transport;
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        _: AiGatewayTransportRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        panic!("invalid request must not reach transport")
    }
}

fn provider() -> AiGatewayProvider {
    AiGatewayProvider::with_limits("test/model", Arc::new(Transport), ordinary()).unwrap()
}

#[test]
fn override_configuration_is_bounded_unique_and_does_not_expand_history() {
    for limits in [
        AiGatewayToolInputLimits {
            max_argument_bytes: 0,
            max_json_nodes: 1,
        },
        AiGatewayToolInputLimits {
            max_argument_bytes: MAX_COMPLETE_INPUT_BYTES + 1,
            max_json_nodes: 1,
        },
        AiGatewayToolInputLimits {
            max_argument_bytes: 1,
            max_json_nodes: MAX_COMPLETE_INPUT_NODES + 1,
        },
    ] {
        assert!(
            provider()
                .with_tool_input_limits([(ToolName::new("terminal").unwrap(), limits)])
                .is_err()
        );
    }
    let pair = configured(32, 10).into_iter().next().unwrap();
    assert!(
        provider()
            .with_tool_input_limits([pair.clone(), pair.clone()])
            .is_err()
    );
    assert!(
        provider()
            .with_tool_input_limits(
                (0..65).map(|index| (ToolName::new(format!("tool{index}")).unwrap(), pair.1))
            )
            .is_err()
    );
    let provider = provider().with_tool_input_limits([pair]).unwrap();
    let mut request = request(&["terminal"]);
    let id = ToolCallId::new("historical").unwrap();
    request.messages = vec![
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                call: ToolCall {
                    id: id.clone(),
                    name: ToolName::new("terminal").unwrap(),
                    arguments: json!({"x":"a".repeat(24)}),
                },
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: id,
                output: machine_god_core::ToolOutput {
                    content: json!({}),
                    is_error: false,
                },
            }],
        },
    ];
    assert!(
        futures_executor::block_on(provider.stream(request, CancellationToken::new())).is_err()
    );
}
