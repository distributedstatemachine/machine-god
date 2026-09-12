use super::{close, executor, owner, support::Fixture};
use crate::{
    NativeBackgroundUrlExecutable, NativeInteractiveInitialSession, NativeInteractiveSession,
    NativeInteractiveSessionOptions, NativeModelPreferences, NativeReasoningEffort,
};
use std::{fs::File, path::PathBuf};

#[test]
fn absent_desktop_capture_keeps_both_launchers_unavailable() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let session = owner(&fixture).await;
        assert!(session.background_opener.is_none());
        assert!(session.mcp_browser_launcher.is_none());
        close(session, fixture).await;
    });
}

#[test]
fn existing_optional_capture_composes_both_paths_without_inspecting_the_launcher() {
    executor().block_on(async {
        for invalid_environment in [false, true] {
            let fixture = Fixture::new();
            // Only spelling is admitted during construction. This nonexistent
            // installation must never be inspected or launched by composition.
            let executable = NativeBackgroundUrlExecutable::new(
                "/unavailable-interactive-browser-composition-fixture".into(),
                File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
            )
            .unwrap();
            let environment = if invalid_environment {
                vec![("INVALID\0KEY".into(), "value".into())]
            } else {
                Vec::new()
            };
            let options = NativeInteractiveSessionOptions::new(
                fixture.workspace.clone(),
                NativeModelPreferences::new(
                    "workspace/default",
                    NativeReasoningEffort::default(),
                    false,
                )
                .unwrap(),
            )
            .unwrap()
            .with_background_url_opener(executable, environment);
            let session = NativeInteractiveSession::open(
                fixture.host.clone(),
                options,
                NativeInteractiveInitialSession::Fresh,
                100,
            )
            .await
            .unwrap();
            assert!(session.options.background_url.is_none());
            assert_eq!(session.background_opener.is_some(), !invalid_environment);
            assert_eq!(session.mcp_browser_launcher.is_some(), !invalid_environment);
            close(session, fixture).await;
        }
    });
}
