use super::*;
mod continuation;
use crate::McpFeatureRequest;
use crate::mcp::{
    catalog::{McpDescriptor, McpDescriptorCatalog, McpDescriptorLimits},
    commands::{McpCommand, McpFeatureCommand},
    pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
    peer::McpPeerCapabilities,
    protocol::{
        NegotiatedProtocol, ProtocolVersion, RpcId, TransportKind, WireLimits, parse_envelope,
    },
};

fn request(command: &str) -> McpFeatureRequest {
    let McpCommand::Feature(command) = command.parse::<McpCommand>().unwrap() else {
        panic!()
    };
    McpFeatureRequest::try_from(command).unwrap()
}
fn options(version: ProtocolVersion, id: i64) -> McpFeatureExchangeOptions {
    let bytes=br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","capabilities":{"resources":{},"prompts":{},"completions":{}}}}"#;
    let envelope = parse_envelope(bytes, WireLimits::default()).unwrap();
    let capabilities = McpPeerCapabilities::admit(&envelope, version).unwrap();
    McpFeatureExchangeOptions::new(
        NegotiatedProtocol {
            version,
            transport: TransportKind::Stdio,
        },
        id,
        capabilities,
    )
    .unwrap()
}
fn catalog(kind: McpCatalogKind, items: &str) -> McpDescriptorCatalog {
    let field = match kind {
        McpCatalogKind::Resources => "resources",
        McpCatalogKind::ResourceTemplates => "resourceTemplates",
        McpCatalogKind::Prompts => "prompts",
        McpCatalogKind::Tools => "tools",
    };
    let response = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","{field}":{items}}}}}"#
    );
    let mut builder =
        McpCatalogBuilder::new(kind, ProtocolVersion::Modern, McpCatalogLimits::default()).unwrap();
    builder
        .append_response(response.as_bytes(), &RpcId::Integer(1), None, 0)
        .unwrap();
    McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default()).unwrap()
}
fn catalogs() -> Vec<McpDescriptorCatalog> {
    vec![
        catalog(
            McpCatalogKind::Resources,
            r#"[{"uri":"test://fixed","name":"fixed"}]"#,
        ),
        catalog(
            McpCatalogKind::ResourceTemplates,
            r#"[{"uriTemplate":"test:///{id}","name":"dynamic"}]"#,
        ),
        catalog(
            McpCatalogKind::Prompts,
            r#"[{"name":"review","arguments":[{"name":"topic","required":true}]}]"#,
        ),
    ]
}
fn exchange(command: &str) -> McpFeatureExchange {
    McpFeatureExchange::prepare(
        &request(command),
        "srv",
        &catalogs(),
        options(ProtocolVersion::Modern, 7),
        None,
        McpFeatureCodecLimits::default(),
    )
    .unwrap()
}

#[test]
fn all_seven_requests_derive_fixed_methods_and_exact_typed_metadata() {
    for (command, method) in [
        ("resource list srv", "resources/list"),
        ("resource templates srv", "resources/templates/list"),
        ("resource read srv test://fixed", "resources/read"),
        ("prompt list srv", "prompts/list"),
        (r#"prompt get srv review {"topic":"x"}"#, "prompts/get"),
        (
            "prompt complete srv review unknown x",
            "completion/complete",
        ),
        (
            "resource complete srv test:///{id} unknown x",
            "completion/complete",
        ),
    ] {
        let exchange = exchange(command);
        assert_eq!(exchange.method(), method);
        let wire = machine_god_core::json::from_str(exchange.wire_json().get()).unwrap();
        assert_eq!(wire["id"], 7);
        assert_eq!(wire["method"], method);
        assert_eq!(
            wire["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
            "machine-god"
        );
        assert!(!format!("{exchange:?}").contains("test://"));
    }
    let exchange = exchange("resource complete srv test:///{id} id abc");
    let value = machine_god_core::json::from_str(exchange.wire_json().get()).unwrap();
    assert_eq!(value["params"]["ref"]["type"], "ref/resource");
    assert_eq!(value["params"]["argument"]["value"], "abc");
}

#[test]
fn capability_identity_required_arguments_and_protocol_checks_are_exact() {
    for command in [
        "prompt get srv review {}",
        r#"prompt get srv review {"topic":"x","unknown":"y"}"#,
        "prompt complete srv absent topic",
        "resource complete srv test:///{other} id",
        "resource read srv test://absent",
    ] {
        assert!(
            McpFeatureExchange::prepare(
                &request(command),
                "srv",
                &catalogs(),
                options(ProtocolVersion::Modern, 7),
                None,
                McpFeatureCodecLimits::default()
            )
            .is_err()
        );
    }
    assert!(matches!(
        exchange("resource read srv test:///hello%20world").identity(),
        McpFeatureIdentity::ResourceTemplate(_)
    ));
    assert!(
        McpFeatureExchange::prepare(
            &request("resource list srv"),
            "other",
            &[],
            options(ProtocolVersion::Modern, 7),
            None,
            McpFeatureCodecLimits::default()
        )
        .is_err()
    );
    let empty = McpFeatureExchangeOptions::new(
        NegotiatedProtocol {
            version: ProtocolVersion::Modern,
            transport: TransportKind::Stdio,
        },
        1,
        McpPeerCapabilities::default(),
    )
    .unwrap();
    assert_eq!(
        McpFeatureExchange::prepare(
            &request("resource list srv"),
            "srv",
            &[],
            empty,
            None,
            McpFeatureCodecLimits::default()
        )
        .unwrap_err(),
        McpFeatureCodecError::Unsupported
    );
    assert!(
        McpFeatureExchangeOptions::new(
            NegotiatedProtocol {
                version: ProtocolVersion::Modern,
                transport: TransportKind::Stdio
            },
            -1,
            McpPeerCapabilities::default()
        )
        .is_err()
    );
}

#[test]
fn list_codecs_use_atomic_existing_pagination_and_descriptor_admission() {
    for (command, field, item) in [
        (
            "resource list srv",
            "resources",
            r#"{"uri":"x","name":"x"}"#,
        ),
        (
            "resource templates srv",
            "resourceTemplates",
            r#"{"uriTemplate":"x/{id}","name":"x"}"#,
        ),
        (
            "prompt list srv",
            "prompts",
            r#"{"name":"x","arguments":[]}"#,
        ),
    ] {
        let first = exchange(command);
        let mut load = McpFeatureCatalogLoad::new(
            &first,
            McpCatalogLimits::default(),
            McpDescriptorLimits::default(),
        )
        .unwrap();
        let page = format!(
            r#"{{"jsonrpc":"2.0","id":7,"result":{{"{field}":[{item}],"nextCursor":""}}}}"#
        );
        assert!(load.append(&first, page.as_bytes(), 0).unwrap());
        assert_eq!(load.next_cursor(), Some(""));
        let next = McpFeatureExchange::prepare(
            first.request(),
            "srv",
            &[],
            options(ProtocolVersion::Modern, 8),
            Some(""),
            McpFeatureCodecLimits::default(),
        )
        .unwrap();
        let page = format!(r#"{{"jsonrpc":"2.0","id":8,"result":{{"{field}":[]}}}}"#);
        assert!(!load.append(&next, page.as_bytes(), 1).unwrap());
        assert_eq!(load.finish().unwrap().descriptors().len(), 1);
    }
    let request = exchange("prompt list srv");
    let mut bad = McpFeatureCatalogLoad::new(
        &request,
        McpCatalogLimits::default(),
        McpDescriptorLimits::default(),
    )
    .unwrap();
    assert!(
        bad.append(
            &request,
            br#"{"jsonrpc":"2.0","id":99,"result":{"prompts":[]}}"#,
            0
        )
        .is_err()
    );
    assert!(bad.finish().is_err());
}

#[test]
fn complete_resource_and_prompt_results_preserve_full_data_and_numeric_metadata() {
    let response=exchange("resource read srv test://fixed").admit_response(br#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"test://other","text":"hello","_meta":{"n":9007199254740993.0,"$serde_json::private::Number":"literal"}},{"uri":"blob","blob":"aGVsbG8="}],"ttlMs":-1.2,"cacheScope":"public","unknown":1e999999}}"#).unwrap();
    let McpFeatureOutcome::Resource { contents, cache } = response.outcome() else {
        panic!()
    };
    assert_eq!(contents.len(), 2);
    assert_eq!(cache.ttl_ms, Some(0));
    assert!(contents[0].raw_json().get().contains("9007199254740993.0"));
    assert!(response.result_json().get().contains("1e999999"));
    let response=exchange(r#"prompt get srv review {"topic":"x"}"#).admit_response(br#"{"jsonrpc":"2.0","id":7,"result":{"description":"data","messages":[{"role":"user","content":{"type":"text","text":"untrusted","unknown":-0}},{"role":"assistant","content":{"type":"image","data":"AA==","mimeType":"image/png"}},{"role":"assistant","content":{"type":"audio","data":"AA==","mimeType":"audio/wav"}},{"role":"user","content":{"type":"resource_link","uri":"x","name":"x","size":18446744073709551615.0}},{"role":"user","content":{"type":"resource","resource":{"uri":"x","text":"embedded"}}}]}}"#).unwrap();
    let McpFeatureOutcome::Prompt { messages, .. } = response.outcome() else {
        panic!()
    };
    assert_eq!(messages.len(), 5);
    assert!(messages[0].content().raw_json().get().contains("-0"));
}

#[test]
fn completion_results_preserve_order_duplicates_and_exact_total() {
    for command in [
        "prompt complete srv review notAdvertised x",
        "resource complete srv test:///{id} notAdvertised x",
    ] {
        let response=exchange(command).admit_response(br#"{"jsonrpc":"2.0","id":7,"result":{"completion":{"values":["a","a",""],"total":18446744073709551615.0,"hasMore":true},"_meta":{"n":-0}}}"#).unwrap();
        let McpFeatureOutcome::Completion {
            values,
            total,
            has_more,
        } = response.outcome()
        else {
            panic!()
        };
        assert_eq!(values.len(), 3);
        assert_eq!(*total, Some(u64::MAX));
        assert_eq!(*has_more, Some(true));
    }
}

#[test]
fn malformed_correlation_contents_and_input_required_never_become_success() {
    let read = exchange("resource read srv test://fixed");
    for response in [
        r#"{"jsonrpc":"2.0","id":"7","result":{"contents":[]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"x","text":"a","blob":"AA=="}]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"x","blob":"AB=="}]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"x","text":"a","annotations":{"priority":1.0000000000000000000001}}]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":{"contents":[],"contents":[]}}"#,
    ] {
        assert!(read.admit_response(response.as_bytes()).is_err());
    }
    let input=read.admit_response(br#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","inputRequests":[]}}"#).unwrap();
    assert!(matches!(
        input.outcome(),
        McpFeatureOutcome::UnvalidatedInputRequired
    ));
    let failure=read.admit_response(br#"{"jsonrpc":"2.0","id":7,"error":{"code":-32602,"message":"private diagnostic","data":{"n":9007199254740993.0}}}"#).unwrap();
    assert!(matches!(
        failure.outcome(),
        McpFeatureOutcome::ProtocolFailure { code: -32602 }
    ));
    assert!(!format!("{failure:?}").contains("private diagnostic"));
}

#[test]
fn full_native_content_is_not_truncated_to_the_old_model_adapter() {
    let read = exchange("resource read srv test://fixed");
    let text = "x".repeat(1024 * 1024);
    let response = format!(
        r#"{{"jsonrpc":"2.0","id":7,"result":{{"contents":[{{"uri":"x","text":"{text}"}}]}}}}"#
    );
    let result = read.admit_response(response.as_bytes()).unwrap();
    let McpFeatureOutcome::Resource { contents, .. } = result.outcome() else {
        panic!()
    };
    let McpResourceData::Text(text) = contents[0].data() else {
        panic!()
    };
    assert_eq!(text.len(), 1024 * 1024);
    let small = McpFeatureExchange::prepare(
        read.request(),
        "srv",
        &catalogs(),
        options(ProtocolVersion::Modern, 7),
        None,
        McpFeatureCodecLimits {
            max_content_field_bytes: 4,
            ..McpFeatureCodecLimits::default()
        },
    )
    .unwrap();
    assert!(small.admit_response(response.as_bytes()).is_err());
}

#[test]
fn template_operators_literal_percent_bytes_and_shared_work_match_the_pin() {
    for (template, yes, no) in [
        ("x/{v}", "x/a%20b", "x/a/b"),
        ("x/{+v}", "x/a/b", "x/a b"),
        ("x/{#v}", "x/#a/b", "x/a"),
        ("x/{.v}", "x/.a", "x/a"),
        ("x/{/v}", "x//a", "x/a"),
        ("x/{;v}", "x/;v", "x/;z"),
        ("x/{?v}", "x/?v=", "x/?v"),
        ("x/{&v}", "x/&v=a", "x/&z=a"),
    ] {
        let catalog = catalog(
            McpCatalogKind::ResourceTemplates,
            &format!(r#"[{{"name":"x","uriTemplate":"{template}"}}]"#),
        );
        let McpDescriptor::ResourceTemplate(descriptor) = &catalog.descriptors()[0] else {
            panic!()
        };
        assert!(
            matches_resource_template(
                descriptor,
                yes,
                &mut McpTemplateMatchBudget::new(1024 * 1024).unwrap()
            )
            .unwrap()
        );
        assert!(
            !matches_resource_template(
                descriptor,
                no,
                &mut McpTemplateMatchBudget::new(1024 * 1024).unwrap()
            )
            .unwrap()
        );
        assert!(
            matches_resource_template(
                descriptor,
                yes,
                &mut McpTemplateMatchBudget::new(1).unwrap()
            )
            .is_err()
        );
    }
    let catalog = catalog(
        McpCatalogKind::ResourceTemplates,
        r#"[{"name":"ambiguous","uriTemplate":"x/{+a}/z/{b}"}]"#,
    );
    let McpDescriptor::ResourceTemplate(descriptor) = &catalog.descriptors()[0] else {
        panic!()
    };
    assert!(
        matches_resource_template(
            descriptor,
            "x/foo/z/bar/z/end",
            &mut McpTemplateMatchBudget::new(1024 * 1024).unwrap()
        )
        .unwrap()
    );
    let mut shared = McpTemplateMatchBudget::new(50).unwrap();
    assert!(matches_resource_template(descriptor, "x/a/z/b", &mut shared).unwrap());
    assert!(matches_resource_template(descriptor, "x/a/z/b", &mut shared).is_err());
}

#[test]
fn modern_advertisements_reuse_the_tool_codec() {
    use crate::mcp::protocol::McpClientMetadata;
    let metadata =
        McpClientMetadata::for_protocol(ProtocolVersion::Modern, Some(u64::MAX), true, true);
    let text = serde_json::to_string(&metadata).unwrap();
    assert!(text.contains("18446744073709551615"));
    let value = machine_god_core::json::from_str(&text).unwrap();
    assert!(value["io.modelcontextprotocol/clientCapabilities"]["elicitation"]["form"].is_object());
    let request = McpFeatureRequest::try_from(McpFeatureCommand::ResourceList {
        server: "srv".into(),
    })
    .unwrap();
    let exchange = McpFeatureExchange::prepare(
        &request,
        "srv",
        &[],
        options(ProtocolVersion::Modern, 7),
        None,
        McpFeatureCodecLimits::default(),
    )
    .unwrap();
    assert!(exchange.wire_json().get().contains("_meta"));
}

#[test]
fn inclusive_content_completion_and_server_pagination_boundaries() {
    let base = exchange("resource read srv test://fixed");
    let small = McpFeatureExchange::prepare(
        base.request(),
        "srv",
        &catalogs(),
        options(ProtocolVersion::Modern, 7),
        None,
        McpFeatureCodecLimits {
            max_content_bytes: 5,
            max_content_items: 1,
            ..McpFeatureCodecLimits::default()
        },
    )
    .unwrap();
    assert!(
        small
            .admit_response(
                br#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"x","text":"abcd"}]}}"#
            )
            .is_ok()
    );
    for response in [br#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"x","text":"abcde"}]}}"#.as_slice(),br#"{"jsonrpc":"2.0","id":7,"result":{"contents":[{"uri":"x","text":""},{"uri":"x","text":""}]}}"#] {assert!(small.admit_response(response).is_err());}
    let completion = exchange("prompt complete srv review topic x");
    for (count, length, valid) in [
        (100, 0, true),
        (101, 0, false),
        (1, 4096, true),
        (1, 4097, false),
        (16, 4096, true),
        (17, 4096, false),
    ] {
        let values = std::iter::repeat_n(format!("\"{}\"", "a".repeat(length)), count)
            .collect::<Vec<_>>()
            .join(",");
        let response = format!(
            r#"{{"jsonrpc":"2.0","id":7,"result":{{"completion":{{"values":[{values}]}}}}}}"#
        );
        assert_eq!(
            completion.admit_response(response.as_bytes()).is_ok(),
            valid
        );
    }
    let first = exchange("resource list srv");
    let mut load = McpFeatureCatalogLoad::new(
        &first,
        McpCatalogLimits::default(),
        McpDescriptorLimits::default(),
    )
    .unwrap();
    let other = McpFeatureExchange::prepare(
        &request("resource list other"),
        "other",
        &[],
        options(ProtocolVersion::Modern, 7),
        None,
        McpFeatureCodecLimits::default(),
    )
    .unwrap();
    assert!(
        load.append(
            &other,
            br#"{"jsonrpc":"2.0","id":7,"result":{"resources":[]}}"#,
            0
        )
        .is_err()
    );
    assert!(load.finish().is_err());
    let admitted = catalogs();
    let get = exchange(r#"prompt get srv review {"topic":"x"}"#);
    let bytes = admitted
        .iter()
        .map(McpDescriptorCatalog::retained_byte_charge)
        .sum::<usize>()
        + get.wire_json().get().len() * 2
        + 1024
        + 1024;
    for (maximum, valid) in [(bytes, true), (bytes - 1, false)] {
        assert_eq!(
            McpFeatureExchange::prepare(
                get.request(),
                "srv",
                &admitted,
                options(ProtocolVersion::Modern, 7),
                None,
                McpFeatureCodecLimits {
                    max_retained_bytes: maximum,
                    ..McpFeatureCodecLimits::default()
                }
            )
            .is_ok(),
            valid
        );
    }
}
