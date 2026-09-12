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
fn endpoint_origin_comparison_is_exact() {
    let base = McpEndpoint::parse("https://example.test/rpc").unwrap();
    for value in [
        "https://other.test/messages",
        "https://example.test:444/messages",
    ] {
        assert!(!base.same_origin(&McpEndpoint::parse(value).unwrap()));
    }
    assert!(base.same_origin(&McpEndpoint::parse("https://EXAMPLE.test:443/messages").unwrap()));
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
    // Valid UTF-8 can expand threefold when canonicalized as an HTTP URI.
    let unicode = format!(
        "{prefix}{}",
        "é".repeat((MAX_CONFIGURED_ENDPOINT_BYTES - prefix.len()) / 2)
    );
    let expanded = McpEndpoint::parse(&unicode).unwrap();
    assert!(expanded.as_str().len() > unicode.len());
    assert!(expanded.as_str().len() <= MAX_CANONICAL_ENDPOINT_BYTES);
}

#[test]
fn debug_errors_and_origin_checks_do_not_expose_queries() {
    let a = McpEndpoint::parse("https://example.test/a?secret=private").unwrap();
    let b = McpEndpoint::parse("https://EXAMPLE.test:443/b?other=private").unwrap();
    assert!(a.same_origin(&b));
    assert!(!format!("{a:?}").contains("private"));
    let error = McpEndpoint::parse("https://user:private@other.test/?secret=private").unwrap_err();
    assert_eq!(error, McpEndpointError::Invalid);
    assert!(!format!("{error:?}: {error}").contains("private"));
}
