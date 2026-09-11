use super::*;
mod flow;

#[test]
fn challenge_scope_and_exact_callback_policy() {
    let challenge = McpAuthChallenge::parse(br#"Basic realm="private", Bearer resource_metadata="https://example.com/meta", scope="read write", error="insufficient_scope""#).unwrap();
    assert!(challenge.insufficient_scope());
    assert_eq!(challenge.scope.as_deref(), Some("read write"));
    assert_eq!(
        codec::scopes(&[], challenge.scope.as_deref(), &[], Some("read"), true)
            .unwrap()
            .as_ref(),
        "read write offline_access"
    );
    assert!(
        browser::callback_code(
            "/callback?code=code&state=exact&iss=https%3A%2F%2Fissuer.example",
            "exact",
            "https://issuer.example",
            true
        )
        .is_ok()
    );
    assert!(matches!(
        browser::callback_code("/callback?code=code&state=wrong", "exact", "issuer", false),
        Err(McpAuthError::StateMismatch)
    ));
    assert!(
        browser::callback_code(
            "/callback?code=x&state=exact&state=exact",
            "exact",
            "issuer",
            false
        )
        .is_err()
    );
    assert!(
        browser::callback_code("/callback?code=%ZZ&state=exact", "exact", "issuer", false).is_err()
    );
}

#[test]
fn metadata_paths_and_exact_json_do_not_normalize_numeric_identity() {
    assert_eq!(
        discovery::resource_urls("https://example.com/mcp").unwrap(),
        vec![
            "https://example.com/.well-known/oauth-protected-resource/mcp",
            "https://example.com/.well-known/oauth-protected-resource"
        ]
    );
    assert_eq!(
        discovery::issuer_urls("https://example.com/tenant")
            .unwrap()
            .len(),
        3
    );
    let value =
        codec::json(br#"{"expires_in":9007199254740993,"$serde_json::private::Number":"literal"}"#)
            .unwrap();
    assert_eq!(value["expires_in"].as_i64(), Some(9_007_199_254_740_993));
    assert_eq!(
        value["$serde_json::private::Number"].as_str(),
        Some("literal")
    );
    assert!(codec::json(br#"{"a":1,"a":2}"#).is_err());
    assert!(codec::secure_url("http://public.example:80/mcp").is_err());
    assert!(codec::secure_url("http://127.0.0.1/mcp").is_err());
    assert!(codec::secure_url("http://127.0.0.1:80/mcp").is_ok());
    assert!(codec::oauth_url("http://127.0.0.1:80/token", "https://public.example/mcp").is_err());
}
