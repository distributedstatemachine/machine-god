use super::*;
use crate::mcp::pagination::{McpCatalogBuilder, McpCatalogLimits};
use crate::mcp::protocol::RpcId;
use crate::mcp::schema::{McpSchemaAssessment, McpSchemaValidation};

fn raw(kind: McpCatalogKind, items: &str) -> McpRawCatalog {
    let field = match kind {
        McpCatalogKind::Tools => "tools",
        McpCatalogKind::Resources => "resources",
        McpCatalogKind::ResourceTemplates => "resourceTemplates",
        McpCatalogKind::Prompts => "prompts",
    };
    let response = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","ttlMs":100,"{field}":{items}}}}}"#
    );
    let mut builder =
        McpCatalogBuilder::new(kind, ProtocolVersion::Modern, McpCatalogLimits::default()).unwrap();
    builder
        .append_response(response.as_bytes(), &RpcId::Integer(1), None, 42)
        .unwrap();
    builder.finish().unwrap()
}
fn admit(kind: McpCatalogKind, items: &str) -> Result<McpDescriptorCatalog> {
    McpDescriptorCatalog::admit(raw(kind, items), McpDescriptorLimits::default())
}
fn tools(items: &str) -> McpDescriptorCatalog {
    admit(McpCatalogKind::Tools, items).unwrap()
}

#[test]
fn producer_tool_descriptor_retains_optional_fields_and_exact_schema() {
    let source = r#"[{"name":"echo","title":"Echo","description":"Echo input","icons":[{"src":"data:image/png;base64,AA=="}],"annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","properties":{"n":{"const":9007199254740993.0}}},"outputSchema":false,"_meta":{"$serde_json::private::Number":"data","n":9007199254740993.0}}]"#;
    let catalog = tools(source);
    assert_eq!(catalog.fetched_at_ms(), 42);
    assert_eq!(catalog.expires_at_ms(), 142);
    let McpDescriptor::Tool(tool) = &catalog.descriptors()[0] else {
        panic!()
    };
    assert_eq!(tool.name(), "echo");
    assert_eq!(tool.title(), Some("Echo"));
    assert_eq!(tool.description(), Some("Echo input"));
    assert!(tool.raw_json().get().contains("9007199254740993.0"));
    assert!(
        tool.metadata_json()
            .unwrap()
            .get()
            .contains("9007199254740993.0")
    );
    assert_eq!(
        tool.input_schema()
            .validate_json(br#"{"n":9007199254740993}"#)
            .unwrap(),
        McpSchemaValidation::Valid
    );
    assert_eq!(tool.output_schema().unwrap().raw_json(), "false");
    assert!(tool.icons_json().is_some());
    assert!(tool.annotations_json().is_some());
    assert!(!format!("{tool:?}").contains("echo"));
}

#[test]
fn all_feature_families_admit_full_descriptor_shapes() {
    let resources = admit(McpCatalogKind::Resources, r#"[{"uri":"custom://guide","name":"Guide","title":"Title","description":"Read","mimeType":"text/plain","size":18446744073709551615.0,"icons":[{"src":"icon","sizes":["","16x16"],"theme":"dark"}],"annotations":{"audience":["user","assistant"],"priority":1,"lastModified":"now"},"_meta":{"n":9007199254740993.0}}]"#).unwrap();
    let McpDescriptor::Resource(resource) = &resources.descriptors()[0] else {
        panic!()
    };
    assert_eq!(resource.size(), Some(u64::MAX));
    assert_eq!(resource.mime_type(), Some("text/plain"));
    assert_eq!(resource.uri(), "custom://guide");
    let templates = admit(McpCatalogKind::ResourceTemplates, r#"[{"uriTemplate":"custom://{path}/detail{?query}","name":"Template","size":-0,"mimeType":"text/plain"}]"#).unwrap();
    let McpDescriptor::ResourceTemplate(template) = &templates.descriptors()[0] else {
        panic!()
    };
    assert_eq!(template.uri_template(), "custom://{path}/detail{?query}");
    let prompts = admit(McpCatalogKind::Prompts, r#"[{"name":"review","description":"Review code","arguments":[{"name":"code","description":"Source","required":true},{"name":"style"}],"icons":[],"_meta":{"vendor":true},"annotations":"unknown keyword"}]"#).unwrap();
    let McpDescriptor::Prompt(prompt) = &prompts.descriptors()[0] else {
        panic!()
    };
    assert!(prompt.argument_named("code").unwrap().required());
    assert!(!prompt.argument_named("style").unwrap().required());
    assert!(prompt.annotations_json().is_none());
    assert!(prompt.raw_json().get().contains("unknown keyword"));
}

#[test]
fn malformed_descriptor_fields_and_hard_schemas_reject_atomically() {
    for bad in [
        r#"{"name":"bad"}"#,
        r#"{"name":"bad","inputSchema":true}"#,
        r#"{"name":"bad","inputSchema":{"type":["object"]}}"#,
        r#"{"name":"bad","inputSchema":{"type":"object","required":4}}"#,
        r#"{"name":"bad","inputSchema":{"type":"object"},"outputSchema":3}"#,
        r#"{"name":"bad","inputSchema":{"type":"object"},"description":null}"#,
        r#"{"name":"bad","inputSchema":{"type":"object"},"icons":[{"src":"x","theme":"blue"}]}"#,
        r#"{"name":"bad","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":"yes"}}"#,
        r#"{"name":"bad","inputSchema":{"type":"object"},"_meta":[]}"#,
    ] {
        assert!(
            admit(
                McpCatalogKind::Tools,
                &format!(r#"[{{"name":"good","inputSchema":{{"type":"object"}}}},{bad}]"#)
            )
            .is_err(),
            "{bad}"
        );
    }
    for extra in [
        r#""size":-1"#,
        r#""size":1.5"#,
        r#""size":18446744073709551616"#,
        r#""annotations":{"priority":1.0000000000000000001}"#,
        r#""annotations":{"audience":["system"]}"#,
    ] {
        assert!(
            admit(
                McpCatalogKind::Resources,
                &format!(r#"[{{"uri":"x","name":"x",{extra}}}]"#)
            )
            .is_err()
        );
    }
    for arguments in [
        r#"[{"name":"x"},{"name":"x"}]"#,
        r#"[{"name":"x","required":1}]"#,
        "null",
    ] {
        assert!(
            admit(
                McpCatalogKind::Prompts,
                &format!(r#"[{{"name":"p","arguments":{arguments}}}]"#)
            )
            .is_err()
        );
    }
    let delegated = tools(
        r#"[{"name":"remote","inputSchema":{"type":"object","properties":{"x":{"pattern":"(?=a)"}}}}]"#,
    );
    let McpDescriptor::Tool(tool) = &delegated.descriptors()[0] else {
        panic!()
    };
    assert_eq!(
        tool.input_schema().assessment(),
        McpSchemaAssessment::ServerAuthoritative
    );
}

#[test]
fn naming_is_sorted_byte_sanitized_reserved_and_config_ordered() {
    let first = [tools(
        r#"[{"name":"a?b","inputSchema":{"type":"object"}},{"name":"a/b","inputSchema":{"type":"object"}},{"name":"é","inputSchema":{"type":"object"}}]"#,
    )];
    let second = [tools(
        r#"[{"name":"echo","inputSchema":{"type":"object"}}]"#,
    )];
    let inputs = [
        McpCatalogServerInput {
            server_name: "z",
            catalogs: &first,
            tool_policy: McpToolExposurePolicy::Standard,
        },
        McpCatalogServerInput {
            server_name: "a",
            catalogs: &second,
            tool_policy: McpToolExposurePolicy::Standard,
        },
    ];
    let candidate =
        McpCatalogCandidate::build(&inputs, &["mcp_z_a_b"], McpDescriptorLimits::default())
            .unwrap();
    assert_eq!(
        candidate
            .tools()
            .iter()
            .map(McpExposedTool::name)
            .collect::<Vec<_>>(),
        ["mcp_z_a_b_2", "mcp_z_a_b_3", "mcp_z___", "mcp_a_echo"]
    );
    let projection = candidate.model_projection("mcp_z_a_b_2").unwrap();
    assert_eq!(projection.server(), "z");
    assert_eq!(projection.description(), "MCP tool");
    assert_eq!(
        projection
            .tags()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["mcp", "z", "a", "b"]
    );
    assert!(candidate.model_projection("foreign").is_none());
    assert_eq!(candidate.tools()[0].descriptor().name(), "a/b");
    assert!(candidate.tools()[0].search_text().contains("a/b mcp tool"));
}

#[test]
fn modern_http_exclusion_is_explicit_complete_and_does_not_reserve_names() {
    let catalogs = [tools(
        r#"[{"name":"a/b","inputSchema":{"type":"object"}},{"name":"a?b","inputSchema":{"type":"object"}}]"#,
    )];
    let decisions = [
        McpToolExposureDecision {
            remote_name: "a/b",
            eligibility: McpToolEligibility::ExcludeModernHttpHeaders,
        },
        McpToolExposureDecision {
            remote_name: "a?b",
            eligibility: McpToolEligibility::Admit,
        },
    ];
    let input = McpCatalogServerInput {
        server_name: "x",
        catalogs: &catalogs,
        tool_policy: McpToolExposurePolicy::ModernHttp(&decisions),
    };
    let candidate =
        McpCatalogCandidate::build(&[input], &[], McpDescriptorLimits::default()).unwrap();
    assert_eq!(candidate.tools()[0].name(), "mcp_x_a_b");
    assert_eq!(candidate.exclusions()[0].descriptor().name(), "a/b");
    let input = McpCatalogServerInput {
        server_name: "x",
        catalogs: &catalogs,
        tool_policy: McpToolExposurePolicy::ModernHttp(&decisions[..1]),
    };
    assert_eq!(
        McpCatalogCandidate::build(&[input], &[], McpDescriptorLimits::default()).unwrap_err(),
        McpCatalogError::InvalidEligibility
    );
}

#[test]
fn full_schema_and_description_projection_exceeds_legacy_limits_without_truncation() {
    let description = "d".repeat(64 * 1024);
    let schema_description = "s".repeat(200 * 1024);
    let catalog = tools(&format!(
        r#"[{{"name":"large","description":"{description}","inputSchema":{{"type":"object","description":"{schema_description}"}}}}]"#
    ));
    let catalogs = [catalog];
    let input = McpCatalogServerInput {
        server_name: "full",
        catalogs: &catalogs,
        tool_policy: McpToolExposurePolicy::Standard,
    };
    let candidate =
        McpCatalogCandidate::build(&[input], &[], McpDescriptorLimits::default()).unwrap();
    let projection = candidate.model_projection("mcp_full_large").unwrap();
    assert_eq!(projection.description().len(), 64 * 1024);
    assert!(projection.input_schema().raw_json().len() > 200 * 1024);
    assert_eq!(candidate.tools().len(), 1);
}

#[test]
fn search_schema_compacts_escaped_names_without_rounding_numbers_or_private_keys() {
    let catalogs = [tools(
        r#"[{"name":"exact","inputSchema":{"type":"object","properties":{"\u0066oo":{"const":9007199254740993.0}},"default":{"negativeZero":-0,"huge":1E+999999,"opaque":{"$serde_json::private::Number":"data"}}}}]"#,
    )];
    let input = McpCatalogServerInput {
        server_name: "x",
        catalogs: &catalogs,
        tool_policy: McpToolExposurePolicy::Standard,
    };
    let candidate =
        McpCatalogCandidate::build(&[input], &[], McpDescriptorLimits::default()).unwrap();
    let tool = &candidate.tools()[0];
    assert!(tool.search_text().contains(r#""foo""#));
    assert!(!tool.search_text().contains(r"\u0066"));
    assert!(tool.search_text().contains("9007199254740993.0"));
    assert!(tool.search_text().contains("1e+999999"));
    assert!(tool.search_text().contains(r#""negativezero":-0"#));
    assert!(
        tool.search_text()
            .contains(r#""$serde_json::private::number":"data""#)
    );
    assert!(
        tool.descriptor()
            .input_schema()
            .raw_json()
            .contains(r"\u0066oo")
    );
    assert!(tool.descriptor().raw_json().get().contains("1E+999999"));
}

#[test]
fn template_grammar_matches_pinned_expression_boundaries() {
    for value in [
        "file://fixed",
        "x/{var}",
        "x/{+var}",
        "x/{#var}",
        "x/{.var}",
        "x/{/var}",
        "x/{;var}",
        "x/{?var}",
        "x/{&var}",
        "x/%20/{a.b}",
    ] {
        assert!(template::valid(value), "{value}");
    }
    for value in [
        "x/{a,b}", "x/{a*}", "x/{a:2}", "x/{a}{b}", "x/{a..b}", "x/{.}", "x/{a", "x/}", "x/%xx",
        "x/é",
    ] {
        assert!(!template::valid(value), "{value}");
    }
    assert!(template::valid(&"/{x}".repeat(64)));
    assert!(!template::valid(&"/{x}".repeat(65)));
}

#[test]
fn candidate_bounds_fail_before_publication_and_names_remain_deterministic() {
    let catalogs = [tools(
        r#"[{"name":"echo","inputSchema":{"type":"object"}}]"#,
    )];
    let input = || McpCatalogServerInput {
        server_name: "x",
        catalogs: &catalogs,
        tool_policy: McpToolExposurePolicy::Standard,
    };
    let original =
        McpCatalogCandidate::build(&[input()], &[], McpDescriptorLimits::default()).unwrap();
    assert_eq!(
        McpCatalogCandidate::build(
            &[input()],
            &[],
            McpDescriptorLimits {
                max_candidate_bytes: 1,
                ..McpDescriptorLimits::default()
            }
        )
        .unwrap_err(),
        McpCatalogError::Limit
    );
    assert_eq!(
        McpCatalogCandidate::build(&[input(), input()], &[], McpDescriptorLimits::default())
            .unwrap_err(),
        McpCatalogError::DuplicateServer
    );
    assert_eq!(original.tools()[0].name(), "mcp_x_echo");
    assert_eq!(
        McpCatalogCandidate::build(&[input()], &["bad/name"], McpDescriptorLimits::default())
            .unwrap_err(),
        McpCatalogError::InvalidReservedName
    );
    let mut names = names::Names::new(&[], 10);
    let remote = "a".repeat(256);
    let first = names.allocate("server", &remote).unwrap();
    let second = names.allocate("server", &remote).unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(second.len(), 64);
    assert!(second.ends_with("_2"));
}

#[test]
fn catalog_and_candidate_charge_schema_indexes_before_retention() {
    let children = (0..10)
        .map(|index| format!(r#""s{index}":{{"$id":"s{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r#"[{{"name":"indexed","inputSchema":{{"type":"object","$id":"https://example.test/{}/","$defs":{{{children}}}}}}}]"#,
        "a".repeat(4096)
    );
    let catalog = tools(&source);
    let McpDescriptor::Tool(tool) = &catalog.descriptors()[0] else {
        panic!()
    };
    assert!(tool.input_schema().retained_byte_charge() > source.len() * 4);
    assert!(catalog.retained_byte_charge() > tool.input_schema().retained_byte_charge());
    assert!(
        McpDescriptorCatalog::admit(
            raw(McpCatalogKind::Tools, &source),
            McpDescriptorLimits {
                max_catalog_bytes: source.len() * 4 + 1,
                ..McpDescriptorLimits::default()
            }
        )
        .is_err()
    );
    let catalogs = [catalog];
    let input = || McpCatalogServerInput {
        server_name: "x",
        catalogs: &catalogs,
        tool_policy: McpToolExposurePolicy::Standard,
    };
    let old = McpCatalogCandidate::build(&[input()], &[], McpDescriptorLimits::default()).unwrap();
    assert_eq!(
        McpCatalogCandidate::build(
            &[input()],
            &[],
            McpDescriptorLimits {
                max_candidate_bytes: source.len() * 4,
                ..McpDescriptorLimits::default()
            }
        )
        .unwrap_err(),
        McpCatalogError::Limit
    );
    assert_eq!(old.tools()[0].name(), "mcp_x_indexed");
}
