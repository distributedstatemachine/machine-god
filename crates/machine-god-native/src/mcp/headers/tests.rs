use super::*;
use crate::mcp::config::{McpServerConfig, McpTransportConfig};

fn remote(fields: serde_json::Value) -> McpRemoteConfig {
    let serde_json::Value::Object(mut object) = fields else {
        panic!("object fixture")
    };
    object.insert("type".into(), "http".into());
    object.insert("url".into(), "https://example.test/mcp".into());
    let server = McpServerConfig::decode("test", &serde_json::to_vec(&object).unwrap()).unwrap();
    let McpTransportConfig::Http(remote) = server.transport() else {
        panic!("HTTP fixture")
    };
    remote.clone()
}

#[test]
fn pinned_resolution_orders_static_environment_and_generated_authorization() {
    let config = remote(
        serde_json::json!({"headers":{"X-Static":"one"},"header_env":{"X-Env":"HEADER"},"bearer_token_env":"TOKEN"}),
    );
    let mut lookups = Vec::new();
    let headers = McpResolvedHeaders::resolve(
        &config,
        |name| {
            lookups.push(name.to_owned());
            match name {
                "HEADER" => Some(&b"two"[..]),
                "TOKEN" => Some(&b"secret"[..]),
                _ => None,
            }
        },
        None,
        &[("X-ACP", b"three")],
    )
    .unwrap();
    assert_eq!(lookups, ["HEADER", "TOKEN"]);
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        [
            ("X-Static", &b"one"[..]),
            ("X-Env", &b"two"[..]),
            ("X-ACP", &b"three"[..]),
            ("Authorization", &b"Bearer secret"[..])
        ]
    );
}

#[test]
fn active_oauth_bypasses_missing_bearer_lookup_but_not_required_header_environment() {
    let config =
        remote(serde_json::json!({"header_env":{"X-Env":"HEADER"},"bearer_token_env":"MISSING"}));
    let headers = McpResolvedHeaders::resolve(
        &config,
        |name| {
            assert_eq!(name, "HEADER");
            Some(&b"header"[..])
        },
        Some(b"active"),
        &[],
    )
    .unwrap();
    assert_eq!(
        headers.iter().last(),
        Some(("Authorization", &b"Bearer active"[..]))
    );
    assert_eq!(
        McpResolvedHeaders::resolve(&config, |_| None, Some(b"active"), &[]).unwrap_err(),
        McpHeaderError::MissingHeaderEnvironment
    );
}

#[test]
fn oauth_configuration_alone_does_not_bypass_bearer_environment() {
    let config = remote(serde_json::json!({"oauth":{},"bearer_token_env":"TOKEN"}));
    assert_eq!(
        McpResolvedHeaders::resolve(&config, |_| None, None, &[]).unwrap_err(),
        McpHeaderError::MissingBearerEnvironment
    );
    let headers = McpResolvedHeaders::resolve(
        &config,
        |name| {
            assert_eq!(name, "TOKEN");
            Some(&b"fallback"[..])
        },
        None,
        &[],
    )
    .unwrap();
    assert_eq!(
        headers.iter().next(),
        Some(("Authorization", &b"Bearer fallback"[..]))
    );
    let config = remote(serde_json::json!({"oauth":{}}));
    assert!(
        McpResolvedHeaders::resolve(&config, |_| panic!("no configured lookup"), None, &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn empty_present_values_and_empty_active_tokens_match_pin() {
    let config =
        remote(serde_json::json!({"header_env":{"X-Empty":"EMPTY"},"bearer_token_env":"TOKEN"}));
    let headers = McpResolvedHeaders::resolve(
        &config,
        |name| {
            assert_eq!(name, "EMPTY");
            Some(&b""[..])
        },
        Some(b""),
        &[],
    )
    .unwrap();
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        [("X-Empty", &b""[..]), ("Authorization", &b"Bearer "[..])]
    );
}

#[test]
fn producer_resolved_header_fixture_accepts_authorization_and_rejects_duplicates() {
    let headers = McpResolvedHeaders::from_resolved(&[
        ("Authorization", b"Bearer redacted"),
        ("X-Workspace", b"one"),
    ])
    .unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(
        McpResolvedHeaders::from_resolved(&[("X-Workspace", b"one"), ("x-workspace", b"two")])
            .unwrap_err(),
        McpHeaderError::Duplicate
    );
    assert_eq!(
        McpResolvedHeaders::from_resolved(&[("Bad Header", b"value")]).unwrap_err(),
        McpHeaderError::InvalidName
    );
    assert_eq!(
        McpResolvedHeaders::from_resolved(&[("X-Test", b"one\r\ntwo")]).unwrap_err(),
        McpHeaderError::InvalidValue
    );
}

#[test]
fn all_protocol_owned_names_are_rejected_case_insensitively() {
    for name in [
        "accept",
        "accept-encoding",
        "connection",
        "content-length",
        "content-type",
        "host",
        "last-event-id",
        "mcp-method",
        "mcp-name",
        "mcp-protocol-version",
        "mcp-session-id",
        "transfer-encoding",
        "mcp-param-",
        "mcp-param-region",
    ] {
        assert_eq!(
            McpResolvedHeaders::from_resolved(&[(&name.to_ascii_uppercase(), b"override")])
                .unwrap_err(),
            McpHeaderError::Reserved
        );
    }
}

#[test]
fn every_control_byte_except_tab_is_rejected_and_obs_text_is_preserved() {
    for byte in (0..=31).chain([127]) {
        let result = McpResolvedHeaders::from_resolved(&[("X-Byte", &[byte])]);
        if byte == b'\t' {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err(), McpHeaderError::InvalidValue);
        }
    }
    let raw = [b' ', b'\t', 0x80, 0xff];
    let headers = McpResolvedHeaders::from_resolved(&[("X-Raw", &raw)]).unwrap();
    assert_eq!(headers.iter().next().unwrap().1, raw);
    for name in ["", "a:b", "a b", "é", "a\tb", "a\0b"] {
        assert_eq!(
            McpResolvedHeaders::from_resolved(&[(name, b"ok")]).unwrap_err(),
            McpHeaderError::InvalidName
        );
    }
    assert!(McpResolvedHeaders::from_resolved(&[("!#$%&'*+-.^_`|~09Az", b"ok")]).is_ok());
}

#[test]
fn resolved_captured_and_bearer_values_share_injection_policy() {
    let config = remote(serde_json::json!({"header_env":{"X-Env":"HEADER"}}));
    assert_eq!(
        McpResolvedHeaders::resolve(&config, |_| Some(&b"bad\nvalue"[..]), None, &[]).unwrap_err(),
        McpHeaderError::InvalidValue
    );
    let config = remote(serde_json::json!({"bearer_token_env":"TOKEN"}));
    assert_eq!(
        McpResolvedHeaders::resolve(&config, |_| Some(&b"bad\rvalue"[..]), None, &[]).unwrap_err(),
        McpHeaderError::InvalidValue
    );
    let headers = McpResolvedHeaders::resolve(
        &config,
        |_| panic!("active OAuth bypass"),
        Some(&[0xff, b'\t']),
        &[],
    )
    .unwrap();
    assert_eq!(headers.iter().next().unwrap().1, b"Bearer \xff\t");
}

#[test]
fn explicit_authorization_and_generated_authorization_never_override_each_other() {
    let config = remote(serde_json::json!({"bearer_token_env":"TOKEN"}));
    for active in [None, Some(&b"active"[..])] {
        assert_eq!(
            McpResolvedHeaders::resolve(
                &config,
                |_| Some(&b"env"[..]),
                active,
                &[("aUtHoRiZaTiOn", b"explicit")]
            )
            .unwrap_err(),
            McpHeaderError::Duplicate
        );
    }
    let config =
        remote(serde_json::json!({"headers":{"X-One":"one"},"header_env":{"X-Two":"TWO"}}));
    for name in ["x-one", "x-two"] {
        assert_eq!(
            McpResolvedHeaders::resolve(&config, |_| Some(&b"two"[..]), None, &[(name, b"extra")])
                .unwrap_err(),
            McpHeaderError::Duplicate
        );
    }
}

#[test]
fn profile_authorization_remains_forbidden_while_explicit_resolved_is_allowed() {
    assert!(
        McpServerConfig::decode(
            "test",
            br#"{"type":"http","url":"https://example.test","headers":{"Authorization":"token"}}"#
        )
        .is_err()
    );
    let config = remote(serde_json::json!({}));
    assert!(
        McpResolvedHeaders::resolve(
            &config,
            |_| panic!("no lookup"),
            None,
            &[("Authorization", b"Basic explicit")]
        )
        .is_ok()
    );
}

#[test]
fn final_count_limit_includes_generated_authorization_before_lookups() {
    let names: Vec<_> = (0..MAX_HEADERS).map(|i| format!("X-{i}")).collect();
    let mut input: Vec<_> = names.iter().map(|name| (name.as_str(), &b""[..])).collect();
    assert_eq!(
        McpResolvedHeaders::from_resolved(&input).unwrap().len(),
        MAX_HEADERS
    );
    let config = remote(serde_json::json!({"bearer_token_env":"TOKEN"}));
    assert_eq!(
        McpResolvedHeaders::resolve(&config, |_| panic!("preflight count"), None, &input)
            .unwrap_err(),
        McpHeaderError::Limit
    );
    input.pop();
    assert_eq!(
        McpResolvedHeaders::resolve(&config, |_| Some(&b""[..]), None, &input)
            .unwrap()
            .len(),
        MAX_HEADERS
    );
    input.extend([("Extra-One", &b""[..]), ("Extra-Two", &b""[..])]);
    assert_eq!(
        McpResolvedHeaders::from_resolved(&input).unwrap_err(),
        McpHeaderError::Limit
    );
}

#[test]
fn field_limits_are_inclusive_and_bearer_prefix_counts() {
    let name = "X".repeat(MAX_HEADER_FIELD_BYTES);
    let value = vec![b'x'; MAX_HEADER_FIELD_BYTES];
    assert!(McpResolvedHeaders::from_resolved(&[(&name, &value)]).is_ok());
    assert_eq!(
        McpResolvedHeaders::from_resolved(&[(&(name + "x"), b"")]).unwrap_err(),
        McpHeaderError::Limit
    );
    assert_eq!(
        McpResolvedHeaders::from_resolved(&[("X", &vec![b'x'; MAX_HEADER_FIELD_BYTES + 1])])
            .unwrap_err(),
        McpHeaderError::Limit
    );
    let config = remote(serde_json::json!({}));
    assert!(
        McpResolvedHeaders::resolve(
            &config,
            |_| None,
            Some(&value[..MAX_HEADER_FIELD_BYTES - 7]),
            &[]
        )
        .is_ok()
    );
    assert_eq!(
        McpResolvedHeaders::resolve(
            &config,
            |_| None,
            Some(&value[..MAX_HEADER_FIELD_BYTES - 6]),
            &[]
        )
        .unwrap_err(),
        McpHeaderError::Limit
    );
}

#[test]
fn aggregate_limit_is_inclusive_and_identity_framing_stays_bounded() {
    let names: Vec<_> = (0..32).map(|i| format!("X{i:02}")).collect();
    let value = vec![b'v'; MAX_HEADER_FIELD_BYTES - 3];
    let mut input: Vec<_> = names
        .iter()
        .map(|name| (name.as_str(), value.as_slice()))
        .collect();
    let headers = McpResolvedHeaders::from_resolved(&input).unwrap();
    assert_eq!(
        headers
            .iter()
            .map(|(n, v)| n.len() + v.len())
            .sum::<usize>(),
        MAX_HEADER_BYTES
    );
    assert_eq!(
        headers.authentication_identity_bytes().len(),
        MAX_HEADER_BYTES + 32 * 8 + 8
    );
    assert!(headers.authentication_identity_bytes().len() <= MAX_AUTHENTICATION_IDENTITY_BYTES);
    input.push(("Extra", b""));
    assert_eq!(
        McpResolvedHeaders::from_resolved(&input).unwrap_err(),
        McpHeaderError::Limit
    );
}

#[test]
fn exact_identity_is_case_and_order_independent_but_not_value_or_boundary_blind() {
    let a = McpResolvedHeaders::from_resolved(&[("X-B", b"secret"), ("X-A", b"value")]).unwrap();
    let b = McpResolvedHeaders::from_resolved(&[("x-a", b"value"), ("x-b", b"secret")]).unwrap();
    assert_eq!(
        a.authentication_identity_bytes(),
        b.authentication_identity_bytes()
    );
    assert_eq!(a.iter().next().unwrap().0, "X-B");
    let changed =
        McpResolvedHeaders::from_resolved(&[("X-A", b"Value"), ("X-B", b"secret")]).unwrap();
    assert_ne!(
        a.authentication_identity_bytes(),
        changed.authentication_identity_bytes()
    );
    let a = McpResolvedHeaders::from_resolved(&[("ab", b"c")]).unwrap();
    let b = McpResolvedHeaders::from_resolved(&[("a", b"bc")]).unwrap();
    assert_ne!(
        a.authentication_identity_bytes(),
        b.authentication_identity_bytes()
    );
    assert_eq!(
        &*a.authentication_identity_bytes(),
        b"MGH1\0\0\0\x01\0\0\0\x02ab\0\0\0\x01c"
    );
    assert_eq!(
        &*McpResolvedHeaders::from_resolved(&[])
            .unwrap()
            .authentication_identity_bytes(),
        b"MGH1\0\0\0\0"
    );
}

#[test]
fn owned_results_and_clones_outlive_capture_without_exposing_debug_secrets() {
    let captured = vec![0xff, b's', b'e', b'c'];
    let config = remote(serde_json::json!({"header_env":{"X-Secret":"PRIVATE_ENV"}}));
    let headers = McpResolvedHeaders::resolve(&config, |_| Some(&captured), None, &[]).unwrap();
    let cloned = headers.clone();
    drop(captured);
    drop(config);
    drop(headers);
    assert_eq!(cloned.iter().next().unwrap().1, &[0xff, b's', b'e', b'c']);
    assert_eq!(format!("{cloned:?}"), "McpResolvedHeaders { <redacted> }");
    for error in [
        McpHeaderError::Limit,
        McpHeaderError::InvalidName,
        McpHeaderError::InvalidValue,
        McpHeaderError::Reserved,
        McpHeaderError::Duplicate,
        McpHeaderError::MissingHeaderEnvironment,
        McpHeaderError::MissingBearerEnvironment,
    ] {
        assert!(!format!("{error:?} {error}").contains("PRIVATE_ENV"));
    }
}
