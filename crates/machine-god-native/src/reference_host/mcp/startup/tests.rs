use super::*;
use crate::mcp::{endpoint::McpEndpoint, network::McpNetworkError};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[test]
fn authority_free_selection_skips_dns_entropy_and_trust_capture() {
    let inputs = network_inputs_with(
        NativeMcpNetworkRequirement::None,
        || panic!("no resolver capture for empty or stdio-only selection"),
        || panic!("no entropy capture for empty or stdio-only selection"),
    );
    assert!(inputs.is_none());
    assert!(
        captured_network(
            inputs,
            Arc::new(TokioMcpClock),
            CancellationToken::new(),
            || { panic!("no TLS trust construction without HTTP peers") }
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn literal_capture_keeps_entropy_and_trust_but_never_reads_system_dns() {
    let entropy_calls = Cell::new(0);
    let trust_calls = Cell::new(0);
    let inputs = network_inputs_with(
        NativeMcpNetworkRequirement::LiteralOnly,
        || panic!("literal selection must not read system resolver configuration"),
        || {
            entropy_calls.set(entropy_calls.get() + 1);
            Some([7; 32])
        },
    );
    let network = captured_network(
        inputs,
        Arc::new(TokioMcpClock),
        CancellationToken::new(),
        || {
            trust_calls.set(trust_calls.get() + 1);
            bundled_trust()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(entropy_calls.get(), 1);
    assert_eq!(trust_calls.get(), 1);
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            let cancellation = CancellationToken::new();
            let deadline = Instant::now() + Duration::from_secs(1);
            for url in [
                "https://127.0.0.1/mcp",
                "https://[::1]/mcp",
                "https://LOCALHOST/mcp",
                "http://localhost:8123/mcp",
            ] {
                let endpoint = McpEndpoint::parse(url).unwrap();
                let admitted = network
                    .admit_endpoint(&endpoint, &cancellation, deadline)
                    .await
                    .unwrap();
                assert_eq!(admitted.trust.is_some(), endpoint.is_tls());
                assert_eq!(admitted.destination.endpoint(), &endpoint);
            }
            for url in ["https://example.test/mcp", "https://localhost./mcp"] {
                assert_eq!(
                    network
                        .admit_endpoint(&McpEndpoint::parse(url).unwrap(), &cancellation, deadline)
                        .await
                        .unwrap_err(),
                    McpNetworkError::Unavailable
                );
            }
        });
}

#[test]
fn system_capture_preserves_resolver_then_entropy_order_and_failure_policy() {
    let stage = Cell::new(0);
    assert!(
        network_inputs_with(
            NativeMcpNetworkRequirement::SystemDns,
            || {
                assert_eq!(stage.replace(1), 0);
                Some(McpResolverConfig::literal_only())
            },
            || {
                assert_eq!(stage.replace(2), 1);
                Some([7; 32])
            },
        )
        .is_some()
    );
    assert_eq!(stage.get(), 2);
    assert!(
        network_inputs_with(
            NativeMcpNetworkRequirement::SystemDns,
            || None,
            || panic!("failed DNS capture must not acquire entropy"),
        )
        .is_none()
    );
    for requirement in [
        NativeMcpNetworkRequirement::LiteralOnly,
        NativeMcpNetworkRequirement::SystemDns,
    ] {
        let inputs = network_inputs_with(
            requirement,
            || Some(McpResolverConfig::literal_only()),
            || None,
        );
        assert!(inputs.is_none());
        assert!(
            captured_network(
                inputs,
                Arc::new(TokioMcpClock),
                CancellationToken::new(),
                || panic!("failed entropy capture must not construct trust")
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn required_bundled_trust_failure_is_not_silently_ignored() {
    let inputs = network_inputs_with(
        NativeMcpNetworkRequirement::LiteralOnly,
        || panic!("DNS"),
        || Some([7; 32]),
    );
    assert!(
        captured_network(
            inputs,
            Arc::new(TokioMcpClock),
            CancellationToken::new(),
            || Err(error())
        )
        .is_err()
    );
}
