use super::{close, executor, owner, support::Fixture};
use crate::{
    NativeBackgroundUrlExecutable, NativeInteractiveInitialSession, NativeInteractiveSession,
    NativeInteractiveSessionOptions, NativeModelPreferences, NativeReasoningEffort,
};
use std::{fs::File, path::PathBuf};

fn executable() -> NativeBackgroundUrlExecutable {
    NativeBackgroundUrlExecutable::new(
        "/unavailable-interactive-browser-composition-fixture".into(),
        File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
    )
    .unwrap()
}

fn environment(invalid: bool) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    if invalid {
        vec![("INVALID\0KEY".into(), "value".into())]
    } else {
        Vec::new()
    }
}

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
            .with_background_url_opener(executable(), environment(invalid_environment));
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

#[test]
fn session_inherits_host_desktop_selection_without_a_second_capture() {
    executor().block_on(async {
        for invalid in [false, true] {
            let fixture = Fixture::with_host_options(
                |options| options.with_background_url_opener(executable(), environment(invalid)),
                None,
            );
            let session = owner(&fixture).await;
            assert!(session.host.has_background_url_selection());
            assert_eq!(session.host.background_url_opener().is_some(), !invalid);
            assert_eq!(session.background_opener.is_some(), !invalid);
            assert_eq!(session.mcp_browser_launcher.is_some(), !invalid);
            assert!(session.options.background_url.is_none());
            close(session, fixture).await;
        }
    });
}

#[test]
fn conflicting_explicit_desktop_selections_fail_before_conversation_preparation() {
    executor().block_on(async {
        for invalid in [false, true] {
            let fixture = Fixture::with_host_options(
                |options| options.with_background_url_opener(executable(), environment(invalid)),
                None,
            );
            let options = NativeInteractiveSessionOptions::new(
                fixture.workspace.clone(),
                NativeModelPreferences::default(),
            )
            .unwrap()
            .with_background_url_opener(executable(), Vec::new());
            let catalog = crate::NativeSessionCatalog::new(fixture.host.session_store().clone());
            let before = catalog
                .list(crate::NativeSessionCatalogQuery::default())
                .await
                .unwrap();
            assert!(before.scan_complete() && before.entries().is_empty());
            let result = NativeInteractiveSession::open(
                fixture.host.clone(),
                options,
                NativeInteractiveInitialSession::Fresh,
                100,
            )
            .await;
            assert!(matches!(
                result,
                Err(crate::NativeInteractiveError::Configuration)
            ));
            let after = catalog
                .list(crate::NativeSessionCatalogQuery::default())
                .await
                .unwrap();
            assert!(after.scan_complete() && after.entries().is_empty());
            drop(catalog);
            fixture.finish();
        }
    });
}
