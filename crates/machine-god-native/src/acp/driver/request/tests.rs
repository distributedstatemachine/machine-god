use super::*;
use serde_json::json;

fn parse(method: &str, params: Value) -> Request {
    decode(method, Some(params)).unwrap()
}
fn rejected(method: &str, params: Value) {
    let error = decode(method, Some(params)).unwrap_err();
    assert_eq!(error.code, -32602);
    assert_eq!(error.message, "Invalid params");
    assert!(error.data.is_none());
}

#[test]
fn initialization_is_exact_modern_integer() {
    assert!(matches!(
        parse("initialize", json!({"protocolVersion":1})),
        Request::Initialize
    ));
    for token in ["0", "2", "1.0", "1e0", "\"1\"", "true", "null"] {
        rejected(
            "initialize",
            serde_json::from_str(&format!("{{\"protocolVersion\":{token}}}")).unwrap(),
        );
    }
    rejected("initialize", json!({}));
    assert_eq!(decode("initialize", None).unwrap_err().code, -32602);
    assert_eq!(decode("session/legacy", None).unwrap_err().code, -32601);
}

#[test]
fn selections_are_typed_and_mcp_omission_is_authoritative_empty() {
    for method in ["session/new", "session/load", "session/resume"] {
        let Request::Select {
            selection,
            cwd,
            mcp,
        } = parse(
            method,
            json!({
                "sessionId":"native:one", "cwd":"/does-not-exist/./workspace//", "extension":true
            }),
        )
        else {
            panic!("selection")
        };
        assert_eq!(cwd, PathBuf::from("/does-not-exist/workspace"));
        assert!(mcp.is_empty());
        match selection {
            NativeAcpSessionSelection::New => assert_eq!(method, "session/new"),
            NativeAcpSessionSelection::Load(id) => {
                assert_eq!(method, "session/load");
                assert_eq!(id.as_str(), "native:one");
            }
            NativeAcpSessionSelection::Resume(id) => {
                assert_eq!(method, "session/resume");
                assert_eq!(id.as_str(), "native:one");
            }
        }
    }
    let Request::Select { mcp, .. } = parse("session/new", json!({"cwd":"/tmp", "mcpServers":[]}))
    else {
        panic!()
    };
    assert!(mcp.is_empty());
    for bad in [
        Value::Null,
        json!({"servers":[]}),
        json!([{"type":"sse","name":"x","url":"https://example.com"}]),
    ] {
        rejected("session/new", json!({"cwd":"/tmp", "mcpServers":bad}));
    }
}

#[test]
fn mcp_uses_ephemeral_grammar_without_startup() {
    let server = json!({"name":"literal", "command":"/missing-program", "args":[],
        "env":[{"name":"TOKEN","value":"sensitive-value"}]});
    let Request::Select { mcp, .. } = parse(
        "session/new",
        json!({"cwd":"/missing", "mcpServers":[server.clone()]}),
    ) else {
        panic!()
    };
    assert_eq!(mcp.server_count(), 1);
    rejected(
        "session/new",
        json!({"cwd":"/tmp", "mcpServers":[server.clone(),server]}),
    );
    rejected(
        "session/new",
        json!({"cwd":"/tmp", "mcpServers":[{"name":"x", "command":"relative", "args":[],"env":[]}]}),
    );
}

#[test]
fn native_ids_and_cwds_are_bounded_before_retention() {
    for id in [
        String::new(),
        "x".repeat(129),
        "foreign/id".to_owned(),
        "secret\n".to_owned(),
    ] {
        rejected("session/close", json!({"sessionId":id}));
    }
    for path in [
        String::new(),
        "relative".to_owned(),
        "/a/../b".to_owned(),
        "/a\0b".to_owned(),
        format!("/{}", "x".repeat(MAX_CWD_BYTES)),
    ] {
        rejected("session/new", json!({"cwd":path}));
    }
    rejected("session/load", json!({"cwd":"/tmp"}));
    let id = "x".repeat(128);
    let Request::Close { session } = parse("session/close", json!({"sessionId":id})) else {
        panic!()
    };
    assert_eq!(session.as_str().len(), 128);
    let Request::Cancel { session } = parse("session/cancel", json!({"sessionId":"one"})) else {
        panic!()
    };
    assert_eq!(session.as_str(), "one");
}

#[test]
fn list_cursor_is_native_and_optional_params_are_explicit() {
    assert!(matches!(
        decode("session/list", None).unwrap(),
        Request::List {
            cwd: None,
            cursor: None
        }
    ));
    let Request::List { cwd, cursor } = parse(
        "session/list",
        json!({"cwd":"/tmp", "cursor":"v1:unknown:one"}),
    ) else {
        panic!()
    };
    assert_eq!(cwd, Some(PathBuf::from("/tmp")));
    assert_eq!(cursor.unwrap().to_string(), "v1:unknown:one");
    for value in [
        json!({"cursor":"foreign"}),
        json!({"cursor":null}),
        json!({"cwd":null}),
        json!([]),
        Value::Null,
    ] {
        rejected("session/list", value);
    }
}

#[test]
fn prompt_keeps_canonical_text_and_typed_resource_targets() {
    let Request::Prompt { session, prompt } = parse(
        "session/prompt",
        json!({"sessionId":"one", "prompt":[
            {"type":"text", "text":"please inspect"},
            {"type":"resource", "resource":{"uri":"file:///workspace/example.rs"}},
            {"type":"resource", "resource":{"uri":"https://example.com/file", "text":"embedded text"}}
        ]}),
    ) else {
        panic!()
    };
    assert_eq!(session.as_str(), "one");
    assert!(prompt.prompt().text.contains("please inspect"));
    assert!(prompt.prompt().text.contains("embedded text"));
    assert_eq!(
        prompt.resource_targets(),
        &[PathBuf::from("/workspace/example.rs")]
    );
    assert_eq!(prompt.omissions().len(), 1);
    rejected(
        "session/prompt",
        json!({"sessionId":"one","prompt":[{"type":"image","data":"secret"}]}),
    );
}

#[test]
fn mode_and_configuration_are_current_native_choices() {
    for mode in ["ask", "auto", "yolo"] {
        let Request::SetMode {
            session,
            mode: parsed,
        } = parse(
            "session/set_mode",
            json!({"sessionId":"one", "modeId":mode}),
        )
        else {
            panic!()
        };
        assert_eq!(session.as_str(), "one");
        assert_eq!(parsed, mode);
        let Request::SetConfig {
            session,
            config,
            value,
        } = parse(
            "session/set_config_option",
            json!({"sessionId":"one", "configId":"mode", "value":mode}),
        )
        else {
            panic!()
        };
        assert_eq!(session.as_str(), "one");
        assert_eq!(config, "mode");
        assert_eq!(value, mode);
    }
    assert!(matches!(
        parse(
            "session/set_config_option",
            json!({"sessionId":"one","configId":"model","value":"provider/model"})
        ),
        Request::SetConfig { .. }
    ));
    for (config, value) in [
        ("mode", "legacy"),
        ("unknown", "ask"),
        ("model", ""),
        ("model", " model"),
    ] {
        rejected(
            "session/set_config_option",
            json!({"sessionId":"one","configId":config,"value":value}),
        );
    }
    rejected(
        "session/set_mode",
        json!({"sessionId":"one","modeId":"legacy"}),
    );
}

#[test]
fn programmatic_values_pay_tree_and_escaped_byte_budgets() {
    rejected(
        "session/list",
        json!({"extension":"x".repeat(ACP_MAX_FRAME_BYTES)}),
    );
    rejected(
        "session/list",
        json!({"extension":"\u{1}".repeat(ACP_MAX_FRAME_BYTES/6)}),
    );
    rejected(
        "session/list",
        json!({"extension":vec![Value::Null; 65_536]}),
    );
    let mut deep = Value::Null;
    for _ in 0..65 {
        deep = Value::Array(vec![deep]);
    }
    rejected("session/list", json!({"extension":deep}));
    rejected(
        "session/list",
        Value::Object(Map::from_iter([(
            "extension".to_owned(),
            Value::Number(serde_json::Number::from_string_unchecked(
                "1,\"injected\":true".to_owned(),
            )),
        )])),
    );
}

#[test]
fn rejected_deep_values_drop_without_recursive_stack_growth() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for method in ["session/list", "unknown"] {
                let mut deep = Value::Null;
                for _ in 0..20_000 {
                    deep = Value::Array(vec![deep]);
                }
                assert!(decode(method, Some(deep)).is_err());
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn bounded_writer_enforces_exact_limit_without_partial_append() {
    let mut writer = BoundedWriter::retaining(5);
    writer.write_all(b"12345").unwrap();
    assert!(writer.write_all(b"6").is_err());
    assert_eq!(writer.bytes.as_deref(), Some(b"12345".as_slice()));
    let mut count = BoundedWriter::counting(5);
    count.write_all(b"12345").unwrap();
    assert!(count.write_all(b"6").is_err());
    assert!(count.bytes.is_none());
    // Raw UTF-8 fits, but JSON escape expansion exceeds the MCP-only ceiling.
    rejected(
        "session/new",
        json!({"cwd":"/tmp", "mcpServers":[{"name":"x", "command":"/missing", "args":["\u{1}".repeat(MAX_CONFIG_BYTES/6)],"env":[]}]}),
    );
}

#[test]
fn diagnostics_do_not_expose_payloads() {
    let error = decode(
        "session/load",
        Some(json!({"sessionId":"secret/path","cwd":"/private"})),
    )
    .unwrap_err();
    assert_eq!(format!("{error:?}"), "AcpRpcError { code: -32602, .. }");
    let request = parse("session/close", json!({"sessionId":"secret"}));
    assert_eq!(format!("{request:?}"), "Request(<redacted>)");
}
