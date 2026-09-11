use super::*;

#[test]
fn pinned_endpoint_policy_and_typed_request_parts() {
    for value in [
        "https://mcp.example.com/rpc?workspace=one",
        "http://localhost:4321/mcp",
        "http://127.0.0.1:4321/mcp",
        "http://[::1]:4321/mcp",
    ] {
        assert!(McpEndpoint::parse(value).is_ok(), "{value}");
    }
    let endpoint = McpEndpoint::parse("HTTPS://EXAMPLE.test:443/a/../rpc?q=1").unwrap();
    assert_eq!(endpoint.as_str(), "https://example.test/rpc?q=1");
    assert_eq!(endpoint.authority(), "example.test");
    assert_eq!(endpoint.request_target(), "/rpc?q=1");
    assert_eq!(endpoint.host(), Host::Domain("example.test"));
    assert_eq!(endpoint.port(), 443);
    assert!(endpoint.is_tls());
    let ipv6 = McpEndpoint::parse("http://[::1]:4321").unwrap();
    assert_eq!(ipv6.authority(), "[::1]:4321");
    assert_eq!(ipv6.request_target(), "/");
    assert!(!ipv6.is_tls());
}

#[test]
fn insecure_or_normalized_loopback_aliases_are_not_admitted() {
    for value in [
        "http://example.com/mcp",
        "http://localhost/mcp",
        "http://127.1:123/mcp",
        "http://2130706433:123/mcp",
        "http://0x7f000001:123/mcp",
        "http://localhost.:123/mcp",
        "http://%6cocalhost:123/mcp",
        "http://[0:0:0:0:0:0:0:1]:123/mcp",
        "ftp://example.com/mcp",
    ] {
        assert!(McpEndpoint::parse(value).is_err(), "{value}");
    }
    let explicit_default = McpEndpoint::parse("http://LOCALHOST:80/mcp").unwrap();
    assert_eq!(explicit_default.port(), 80);
    assert_eq!(explicit_default.as_str(), "http://localhost/mcp");
    assert_eq!(
        explicit_default
            .resolve_message_endpoint("messages")
            .unwrap()
            .as_str(),
        "http://localhost/messages"
    );
}

#[test]
fn invalid_urls_and_silent_parser_repairs_are_rejected() {
    for value in [
        "https://user@example.com/mcp",
        "https://@example.com/mcp",
        "https://:secret@example.com/mcp",
        "https://example.com/mcp#fragment",
        "https:example.com/mcp",
        "https:///example.com/mcp",
        "https://example.com:99999/mcp",
        "https://[::invalid]/mcp",
        "https://example.com\\path",
        "https://example.com/\tpath",
        "https://example.com/ space",
        "https://example.com/%",
        "https://example.com/%0",
        "https://example.com/%xx",
    ] {
        assert!(McpEndpoint::parse(value).is_err(), "{value}");
    }
    assert!(McpEndpoint::parse("https://example.com/a%20b?q=%0d%0a").is_ok());
}

#[test]
fn pinned_sse_endpoint_resolution_is_same_origin() {
    let base = McpEndpoint::parse("https://example.test/events/sse").unwrap();
    assert_eq!(
        base.resolve_message_endpoint("../messages?session=one")
            .unwrap()
            .as_str(),
        "https://example.test/messages?session=one"
    );
    for event in [
        "https://other.test/messages",
        "//other.test/messages",
        "https://example.test:444/messages",
        "http://example.test:443/messages",
        "https://user@example.test/messages",
        "//@example.test/messages",
        "https://example.test/messages#fragment",
        "#fragment",
        "\\other.test/messages",
        "javascript:alert(1)",
        "https:example.test/messages",
        "https:///example.test/messages",
        "",
    ] {
        assert!(base.resolve_message_endpoint(event).is_err(), "{event}");
    }
    assert_eq!(
        base.resolve_message_endpoint("//EXAMPLE.test:443/messages")
            .unwrap()
            .as_str(),
        "https://example.test/messages"
    );
    let local = McpEndpoint::parse("http://127.0.0.1:4321/sse").unwrap();
    assert_eq!(
        local
            .resolve_message_endpoint("http://127.0.0.1:4321/messages")
            .unwrap()
            .as_str(),
        "http://127.0.0.1:4321/messages"
    );
}

#[test]
fn endpoint_budgets_are_inclusive_before_and_after_canonicalization() {
    let prefix = "https://example.test/";
    let exact = format!(
        "{prefix}{}",
        "a".repeat(MAX_CONFIGURED_ENDPOINT_BYTES - prefix.len())
    );
    assert!(McpEndpoint::parse(&exact).is_ok());
    assert_eq!(
        McpEndpoint::parse(&(exact + "a")).unwrap_err(),
        McpEndpointError::Limit
    );
    let base = McpEndpoint::parse(prefix).unwrap();
    assert!(
        base.resolve_message_endpoint(&"a".repeat(MAX_ENDPOINT_EVENT_BYTES))
            .is_ok()
    );
    assert_eq!(
        base.resolve_message_endpoint(&"a".repeat(MAX_ENDPOINT_EVENT_BYTES + 1))
            .unwrap_err(),
        McpEndpointError::Limit
    );
    // Valid UTF-8 can expand threefold when canonicalized as an HTTP URI.
    assert_eq!(
        base.resolve_message_endpoint(&"é".repeat(MAX_ENDPOINT_EVENT_BYTES / 2))
            .unwrap_err(),
        McpEndpointError::Limit
    );
}

#[test]
fn debug_errors_and_origin_checks_do_not_expose_queries() {
    let a = McpEndpoint::parse("https://example.test/a?secret=private").unwrap();
    let b = McpEndpoint::parse("https://EXAMPLE.test:443/b?other=private").unwrap();
    assert!(a.same_origin(&b));
    assert!(!format!("{a:?}").contains("private"));
    let error = a
        .resolve_message_endpoint("https://other.test/?secret=private")
        .unwrap_err();
    assert_eq!(error, McpEndpointError::CrossOrigin);
    assert!(!format!("{error:?}: {error}").contains("private"));
}
