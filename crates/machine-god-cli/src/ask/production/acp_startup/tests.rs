use super::*;
use machine_god_native::{AiGatewayCredentialSource, discover_ai_gateway_credential};

#[test]
fn acp_profile_branch_is_never_entered_even_for_invalid_profile_selection() {
    for directory in ["relative", "/", "/invalid/../profile", "/invalid\0profile"] {
        assert!(
            McpSelection::Ephemeral(NativeMcpNetworkRequirement::None)
                .prepare_management(Some(Path::new(directory)))
                .unwrap()
                .is_none()
        );
        assert!(
            McpSelection::Profile
                .prepare_management(Some(Path::new(directory)))
                .is_err()
        );
    }
    assert!(
        McpSelection::Ephemeral(NativeMcpNetworkRequirement::None)
            .prepare_management(None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn captured_environment_is_the_only_gateway_credential_input() {
    let absent = credential_environment(&[]);
    assert!(discover_ai_gateway_credential(absent).is_err());
    let captured =
        credential_environment(&[("AI_GATEWAY_API_KEY".into(), "captured-test-key".into())]);
    let selected = discover_ai_gateway_credential(captured).unwrap();
    assert_eq!(
        selected.source(),
        AiGatewayCredentialSource::AiGatewayApiKey
    );
    assert!(!format!("{selected:?}").contains("captured-test-key"));
}

#[test]
fn request_workspace_does_not_use_or_change_ambient_cwd() {
    let actual = std::env::current_dir().unwrap();
    let environment = super::super::skills_startup::environment(&[(
        "XDG_STATE_HOME".into(),
        "/captured/state".into(),
    )]);
    let roots =
        NativeRootSelection::from_environment(&environment, Path::new("/request/workspace"))
            .unwrap();
    assert_eq!(roots.workspace_root(), Path::new("/request/workspace"));
    assert_eq!(roots.state_root(), Path::new("/captured/state/machine-god"));
    assert_eq!(std::env::current_dir().unwrap(), actual);
    assert!(NativeRootSelection::from_environment(&environment, Path::new("relative")).is_err());
}

#[test]
fn cancelled_acquisition_is_rejected_before_selection() {
    let cancel = CancellationToken::new();
    assert!(check_cancelled(&cancel).is_ok());
    cancel.cancel();
    assert!(check_cancelled(&cancel).is_err());
}
