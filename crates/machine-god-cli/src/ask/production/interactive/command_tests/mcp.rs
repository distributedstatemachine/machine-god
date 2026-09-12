use super::*;
use native::mcp::{
    management::{McpManagementActivation, NativeMcpManagementError, NativeMcpManagementReceipt},
    store::McpConfigCommitDurability,
};
use std::os::unix::fs::PermissionsExt;

#[path = "mcp/runtime.rs"]
mod runtime;

#[test]
fn mcp_slash_recognition_and_help_preserve_the_global_envelope() {
    for command in [
        "/mcp",
        "/mcp list",
        "/mcp add demo /bin/echo literal",
        "/mcp auth demo",
        "/mcp auth demo --open",
        "/mcp logout demo",
    ] {
        assert!(matches!(
            submission(command),
            Ok(Submission::Slash(NativeSlashCommand::Mcp, _))
        ));
    }
    let maximum = format!(
        "/mcp{}",
        " ".repeat(native::MAX_NATIVE_SLASH_INPUT_BYTES - 4)
    );
    assert!(matches!(
        submission(&maximum),
        Ok(Submission::Slash(NativeSlashCommand::Mcp, ""))
    ));
    assert!(submission(&(maximum + " ")).is_err());
    let help = std::str::from_utf8(super::super::HELP).unwrap();
    assert!(help.contains("/mcp [list|path|add NAME COMMAND [ARGS...]|remove NAME|reload]"));
    assert!(help.contains("Add replaces an existing name"));
    assert!(help.contains("resource complete SERVER TEMPLATE ARGUMENT [VALUE]"));
    assert!(help.contains("prompt get") || help.contains("get SERVER NAME [ARGUMENTS_JSON]"));
    assert!(help.contains("require selected native runtime authority"));
    assert!(help.contains("/mcp auth NAME [--open] | /mcp logout NAME"));
    assert!(help.contains("--open to confirm browser handoff"));
    assert!(help.contains("local removal and remote revocation are separate observations"));
    assert!(!help.contains("auth/logout are currently unavailable"));
}

#[test]
fn mcp_management_uses_native_controls_and_keeps_save_activation_separate() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_mcp();
        let profile = fixture.workspace.parent().unwrap().join("mcp-profile");
        let mut driver = driver(&fixture).await;
        for command in ["/mcp", "/mcp list", "/mcp path"] {
            driver.command(command, 200);
            let outcome = control(&mut driver).await;
            assert!(!outcome.failed());
            let text = String::from_utf8(super::super::super::driver::render_control(&outcome).unwrap()).unwrap();
            assert!(text.contains(if command == "/mcp path" { "mcp.json" } else { "configured, not connected" }));
            assert!(!profile.exists());
        }
        for (command, changed) in [
            ("/mcp add demo /never-executed/private-command SECRET_ARG", true),
            ("/mcp add demo /never-executed/private-command SECRET_ARG", false),
            ("/mcp add demo /replacement/not-executed SECOND_SECRET", true),
        ] {
            driver.command(command, 210);
            let outcome = control(&mut driver).await;
            assert!(!outcome.failed());
            assert!(matches!(&outcome.result, Ok(NativeInteractiveControlReceipt::Mcp(
                NativeMcpManagementReceipt::Saved { commit, activation: McpManagementActivation::NotAttempted }
            )) if commit.changed() == changed && commit.durability() == McpConfigCommitDurability::Confirmed));
            let text = String::from_utf8(super::super::super::driver::render_control(&outcome).unwrap()).unwrap();
            assert!(text.contains("Runtime activation not attempted"));
            assert!(!text.contains("SECRET"));
            assert!(!text.contains("never-executed"));
        }
        driver.command("/mcp list", 220);
        let outcome = control(&mut driver).await;
        let text = String::from_utf8(super::super::super::driver::render_control(&outcome).unwrap()).unwrap();
        assert!(text.contains("demo: stdio; enabled; optional"));
        assert!(!text.contains("SECOND_SECRET"));
        driver.control_outcome = Some(outcome);
        driver.command("/mcp remove demo", 230);
        assert!(String::from_utf8(driver.notice.take().unwrap()).unwrap().contains("previous control"));
        driver.control_outcome.take();
        driver.command("/mcp remove demo", 240);
        assert!(!control(&mut driver).await.failed());
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn malformed_and_unavailable_mcp_commands_never_fall_back_to_prompts() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_mcp();
        let profile = fixture.workspace.parent().unwrap().join("mcp-profile");
        let mut driver = driver(&fixture).await;
        for command in ["/mcp add", "/mcp remove", "/mcp list extra", "/mcp unknown"] {
            driver.command(command, 200);
            let text = String::from_utf8(driver.notice.take().unwrap()).unwrap();
            assert!(text.contains("usage: /mcp") || text.contains("command rejected"));
            assert!(driver.owner.take_control_outcome().is_none());
        }
        for command in [
            "/mcp reload",
            "/mcp auth demo --open",
            "/mcp logout demo",
            "/mcp resource list demo",
            "/mcp prompt list demo",
        ] {
            driver.command(command, 210);
            let outcome = control(&mut driver).await;
            assert!(matches!(
                outcome.result,
                Err(native::NativeInteractiveControlError::Mcp(
                    NativeMcpManagementError::RuntimeUnavailable
                ))
            ));
        }
        assert!(!profile.exists());
        Box::pin(finish(driver, fixture)).await;
    });
}

#[test]
fn cancelled_mcp_mutation_is_inert_and_malformed_selected_store_is_not_hidden() {
    executor().block_on(async {
        let fixture = support::Fixture::new_with_mcp();
        let profile = fixture.workspace.parent().unwrap().join("mcp-profile");
        let mut driver = driver(&fixture).await;
        driver.command("/mcp add demo /not-executed", 200);
        assert!(!profile.exists());
        assert!(driver.owner.request_cancel());
        assert!(matches!(
            control(&mut driver).await.result,
            Err(native::NativeInteractiveControlError::Mcp(
                NativeMcpManagementError::Cancelled
            ))
        ));
        assert!(!profile.exists());
        std::fs::create_dir(&profile).unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = profile.join("mcp.json");
        std::fs::write(&file, b"invalid selected configuration").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        driver.command("/mcp list", 210);
        assert!(matches!(
            control(&mut driver).await.result,
            Err(native::NativeInteractiveControlError::Mcp(
                NativeMcpManagementError::Store(_)
            ))
        ));
        assert_eq!(
            std::fs::read(&file).unwrap(),
            b"invalid selected configuration"
        );
        Box::pin(finish(driver, fixture)).await;
    });
}
