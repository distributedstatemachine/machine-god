use super::*;

fn row(name: &str, transport: McpConfiguredTransport) -> McpConfiguredServer {
    McpConfiguredServer {
        name: name.into(),
        transport,
        enabled: true,
        required: false,
    }
}

#[test]
fn configured_receipts_are_metadata_only_and_terminal_safe() {
    let receipt = NativeMcpManagementReceipt::Configured(
        vec![
            row("name\u{1b}[31m\n", McpConfiguredTransport::Stdio),
            row("remote", McpConfiguredTransport::Http),
            McpConfiguredServer {
                enabled: false,
                required: true,
                ..row("legacy", McpConfiguredTransport::Sse)
            },
        ]
        .into_boxed_slice(),
    );
    let text = String::from_utf8(render(7, Ok(&receipt)).unwrap()).unwrap();
    assert!(text.contains("3 configured servers (configured, not connected)"));
    assert!(text.contains("stdio; enabled; optional"));
    assert!(text.contains("http; enabled; optional"));
    assert!(text.contains("sse; disabled; required"));
    assert!(!text.contains('\u{1b}'));
    assert!(text.contains("name\\u001b[31m\\n"));
}

#[test]
fn receipt_bounds_reject_oversized_values_without_partial_output() {
    let servers: Vec<_> = (0..64)
        .map(|_| row(&"n".repeat(128), McpConfiguredTransport::Stdio))
        .collect();
    let receipt = NativeMcpManagementReceipt::Configured(servers.into_boxed_slice());
    assert!(render(1, Ok(&receipt)).unwrap().len() < super::super::MAX_PRESENTATION_OUTPUT_BYTES);
    for servers in [
        vec![row(&"n".repeat(129), McpConfiguredTransport::Stdio)],
        vec![row("", McpConfiguredTransport::Stdio)],
        (0..65)
            .map(|_| row("n", McpConfiguredTransport::Stdio))
            .collect(),
    ] {
        assert!(
            render(
                1,
                Ok(&NativeMcpManagementReceipt::Configured(
                    servers.into_boxed_slice()
                ))
            )
            .is_err()
        );
    }
    assert!(
        render(
            1,
            Ok(&NativeMcpManagementReceipt::Path("p".repeat(4106).into()))
        )
        .is_err()
    );
}

#[test]
fn path_rendering_escapes_terminal_controls_and_bounds_conversion() {
    let receipt = NativeMcpManagementReceipt::Path("/selected/\u{1b}[2J\n/mcp.json".into());
    let text = String::from_utf8(render(2, Ok(&receipt)).unwrap()).unwrap();
    assert!(text.contains("/selected/\\u001b[2J\\n/mcp.json"));
    assert!(!text.contains('\u{1b}'));
    let maximum = NativeMcpManagementReceipt::Path("x".repeat(4105).into());
    assert!(render(2, Ok(&maximum)).is_ok());
}

#[test]
fn saved_receipts_keep_durability_change_and_activation_independent() {
    for durability in [
        McpConfigCommitDurability::Confirmed,
        McpConfigCommitDurability::Ambiguous,
    ] {
        for changed in [false, true] {
            let mut text = super::super::bounded_output();
            saved(
                &mut text,
                changed,
                durability,
                McpManagementActivation::NotAttempted,
            )
            .unwrap();
            let text = text.finish();
            assert!(text.contains(if changed {
                "; changed."
            } else {
                "; unchanged."
            }));
            assert!(text.contains("Runtime activation not attempted."));
            assert!(!text.contains("reloaded"));
            assert_eq!(
                text.contains("No automatic retry"),
                durability == McpConfigCommitDurability::Ambiguous
            );
            assert!(text.contains(match durability {
                McpConfigCommitDurability::Confirmed => "save: confirmed",
                McpConfigCommitDurability::Ambiguous => "save: ambiguous",
            }));
        }
    }
}

#[test]
fn unavailable_and_cancelled_errors_never_claim_success_or_retry() {
    for error in [
        NativeMcpManagementError::RuntimeUnavailable,
        NativeMcpManagementError::InvalidCommand,
        NativeMcpManagementError::Cancelled,
    ] {
        let text = String::from_utf8(render(3, Err(&error)).unwrap()).unwrap();
        assert!(text.contains("no automatic retry"));
        assert!(!text.contains("save: confirmed"));
        assert!(!text.contains("reloaded"));
    }
}
