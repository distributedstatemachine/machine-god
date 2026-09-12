use super::*;

fn rendered(receipt: &NativeMcpAuthenticationReceipt) -> String {
    String::from_utf8(render(7, Ok(receipt)).unwrap()).unwrap()
}

#[test]
fn confirmation_is_explicit_bounded_and_escaped() {
    let receipt = NativeMcpAuthenticationReceipt::ConfirmationRequired {
        server: "demo".into(),
    };
    let text = rendered(&receipt);
    assert!(text.contains("repeat: /mcp auth demo --open"));
    assert!(text.contains("No browser handoff or authentication completion is established"));
    assert!(!text.contains("http"));
    let escaped = rendered(&NativeMcpAuthenticationReceipt::ConfirmationRequired {
        server: "demo\u{1b}\u{202e}\n".into(),
    });
    assert!(!escaped.contains('\u{1b}') && !escaped.contains('\u{202e}'));
    assert!(escaped.contains("demo\\u001b\\u202e\\n"));
    let maximum = NativeMcpAuthenticationReceipt::ConfirmationRequired {
        server: "s".repeat(128).into(),
    };
    assert!(rendered(&maximum).len() < super::super::super::MAX_PRESENTATION_OUTPUT_BYTES);
    for server in [String::new(), "s".repeat(129)] {
        for receipt in [
            NativeMcpAuthenticationReceipt::ConfirmationRequired {
                server: server.clone().into(),
            },
            NativeMcpAuthenticationReceipt::Authenticated {
                server: server.clone().into(),
                usable: true,
                activation: None,
            },
            NativeMcpAuthenticationReceipt::LoggedOut {
                server: server.into(),
                outcome: McpAuthLogoutReceipt {
                    local: McpAuthLocalRemoval::Removed,
                    remote: McpAuthRemoteRevocation::Confirmed,
                },
            },
        ] {
            assert!(render(1, Ok(&receipt)).is_err());
        }
    }
}

#[test]
fn credential_publication_does_not_imply_usability_or_activation() {
    for usable in [false, true] {
        let text = rendered(&NativeMcpAuthenticationReceipt::Authenticated {
            server: "demo".into(),
            usable,
            activation: None,
        });
        assert!(text.contains("Credential persistence: confirmed"));
        assert!(text.contains(if usable {
            "Credential usability at observation: usable."
        } else {
            "Credential usability at observation: not usable."
        }));
        assert!(text.contains("Runtime activation: not attempted"));
        assert!(text.contains("do not establish a current connection"));
        assert!(!text.contains("Publication: published"));
    }
}

#[test]
fn every_logout_outcome_is_reported_independently_without_global_success() {
    for (local, local_text) in [
        (McpAuthLocalRemoval::Unchanged, "unchanged"),
        (McpAuthLocalRemoval::Removed, "confirmed"),
        (McpAuthLocalRemoval::Ambiguous, "ambiguous"),
        (McpAuthLocalRemoval::Failed, "failed"),
    ] {
        for (remote, remote_text) in [
            (McpAuthRemoteRevocation::NotAttempted, "not attempted"),
            (McpAuthRemoteRevocation::Confirmed, "confirmed"),
            (McpAuthRemoteRevocation::Unsupported, "unsupported"),
            (McpAuthRemoteRevocation::Ambiguous, "ambiguous"),
        ] {
            let text = rendered(&NativeMcpAuthenticationReceipt::LoggedOut {
                server: "demo".into(),
                outcome: McpAuthLogoutReceipt { local, remote },
            });
            assert!(text.contains(&format!("Local credential removal: {local_text}.")));
            assert!(text.contains(&format!("Remote token revocation: {remote_text}.")));
            assert!(text.contains("Local removal is not remote revocation. No automatic retry."));
            assert!(!text.contains("success") && !text.contains("authenticated"));
        }
    }
}

#[test]
fn issuer_mismatch_is_fixed_rejection_and_ambiguous_publication_is_not_denied() {
    let text = String::from_utf8(
        render(
            2,
            Err(&NativeMcpAuthenticationError::Authorization(
                McpAuthError::IssuerMismatch,
            )),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(text.contains("issuer mismatch was rejected"));
    assert!(text.contains("Edit oauth.issuer in the selected server's MCP configuration"));
    assert!(text.contains("retry /mcp auth NAME --open"));
    assert!(!text.contains("https:") && !text.contains("http:"));
    let text = String::from_utf8(
        render(
            3,
            Err(&NativeMcpAuthenticationError::Authorization(
                McpAuthError::AmbiguousPublication,
            )),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(text.contains("Credentials may have been published"));
    assert!(text.contains("inspect stored state before retrying"));
    assert!(!text.contains("not saved") && !text.contains("persistence: confirmed"));
}

#[test]
fn fixed_auth_errors_never_assert_credentials_or_activation() {
    for error in [
        McpAuthError::Invalid,
        McpAuthError::Limit,
        McpAuthError::Unavailable,
        McpAuthError::Denied,
        McpAuthError::Cancelled,
        McpAuthError::Deadline,
        McpAuthError::Network,
        McpAuthError::StateMismatch,
        McpAuthError::Rejected,
        McpAuthError::Missing,
        McpAuthError::Busy,
        McpAuthError::Conflict,
        McpAuthError::Persistence,
    ] {
        let text = String::from_utf8(
            render(4, Err(&NativeMcpAuthenticationError::Authorization(error))).unwrap(),
        )
        .unwrap();
        assert!(text.contains("no automatic retry"));
        assert!(
            !text.contains("Credential persistence: confirmed")
                && !text.contains("Publication: published")
        );
    }
}

mod activation;
