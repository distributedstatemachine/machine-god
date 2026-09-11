use super::*;
use native::mcp::{
    commands::McpCommand,
    management::{McpManagementActivation, NativeMcpManagementError, NativeMcpManagementReceipt},
    store::McpConfigCommitDurability,
};
use native::{
    NativeInteractiveControl, NativeInteractiveControlError, NativeInteractiveControlReceipt,
};

async fn receipt(owner: &mut NativeInteractiveSession) -> native::NativeInteractiveControlOutcome {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 150);
            owner
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}

fn command(owner: &mut NativeInteractiveSession, input: &str) {
    owner
        .request_control(
            NativeInteractiveControl::Mcp {
                command: input.parse().unwrap(),
            },
            150,
        )
        .unwrap();
}

#[test]
fn profile_control_is_inert_exact_owned_and_does_not_activate_servers() {
    executor().block_on(async {
        let fixture = Fixture::new_with_mcp();
        let profile = fixture.workspace.parent().unwrap().join("mcp-profile");
        let mut owner = fresh(&fixture).await;
        command(&mut owner, "add selected /never-run literal-argument");
        assert!(!profile.exists());
        assert!(matches!(
            owner.request_control(
                NativeInteractiveControl::Mcp {
                    command: McpCommand::List,
                },
                150
            ),
            Err(NativeInteractiveError::Busy)
        ));
        let outcome = receipt(&mut owner).await;
        assert!(!outcome.failed());
        let Ok(NativeInteractiveControlReceipt::Mcp(NativeMcpManagementReceipt::Saved {
            commit,
            activation,
        })) = outcome.result
        else {
            panic!("exact save receipt")
        };
        assert!(commit.changed());
        assert_eq!(commit.durability(), McpConfigCommitDurability::Confirmed);
        assert_eq!(activation, McpManagementActivation::NotAttempted);
        assert!(profile.join("mcp.json").is_file());
        assert!(!fixture.workspace.join("mcp.json").exists());
        command(&mut owner, "list");
        let outcome = receipt(&mut owner).await;
        let Ok(NativeInteractiveControlReceipt::Mcp(NativeMcpManagementReceipt::Configured(rows))) =
            outcome.result
        else {
            panic!("configured metadata")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(&*rows[0].name, "selected");
        assert!(fixture.transport.requests().is_empty());
        shutdown(&mut owner, 190).await;
        drop(owner);
        fixture.finish();
    });
}

#[test]
fn cancelled_and_unsupported_controls_preserve_absent_profile() {
    executor().block_on(async {
        let fixture = Fixture::new_with_mcp();
        let profile = fixture.workspace.parent().unwrap().join("mcp-profile");
        let mut owner = fresh(&fixture).await;
        command(&mut owner, "add selected /never-run");
        assert!(owner.request_cancel());
        assert!(matches!(
            receipt(&mut owner).await.result,
            Err(NativeInteractiveControlError::Mcp(
                NativeMcpManagementError::Cancelled
            ))
        ));
        for input in ["reload", "auth selected --open", "resource list selected"] {
            command(&mut owner, input);
            assert!(matches!(
                receipt(&mut owner).await.result,
                Err(NativeInteractiveControlError::Mcp(
                    NativeMcpManagementError::RuntimeUnavailable
                ))
            ));
        }
        assert!(!profile.exists());
        assert!(fixture.transport.requests().is_empty());
        shutdown(&mut owner, 190).await;
        drop(owner);
        fixture.finish();
    });
}

#[test]
fn absent_authority_and_forged_commands_cannot_create_profile_authority() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = fresh(&fixture).await;
        assert!(matches!(
            owner.request_control(
                NativeInteractiveControl::Mcp {
                    command: McpCommand::Add {
                        server: "selected".into(),
                        command: "/never-run".into(),
                        arguments: vec!["x".repeat(4097)]
                    },
                },
                150
            ),
            Err(NativeInteractiveError::Configuration)
        ));
        command(&mut owner, "path");
        assert!(matches!(
            receipt(&mut owner).await.result,
            Err(NativeInteractiveControlError::Unavailable)
        ));
        assert!(
            !fixture
                .workspace
                .parent()
                .unwrap()
                .join("mcp-profile")
                .exists()
        );
        shutdown(&mut owner, 190).await;
        drop(owner);
        fixture.finish();
    });
}
