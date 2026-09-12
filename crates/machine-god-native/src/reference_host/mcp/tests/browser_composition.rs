use super::*;

fn executable() -> NativeBackgroundUrlExecutable {
    NativeBackgroundUrlExecutable::new(
        "/unavailable-host-browser-composition-fixture".into(),
        fs::File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
    )
    .unwrap()
}

#[test]
fn desktop_selection_is_inert_redacted_and_requires_owned_terminal_workers() {
    let selected = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_background_url_opener(executable(), vec![("PRIVATE".into(), "secret".into())]);
    assert!(!format!("{selected:?}").contains("secret"));
    let prepared: PreparedCompositionOptions = selected.clone().into();
    assert!(prepared.background_url.is_some());
    assert_eq!(
        validate_prepared_selections(
            &LoadedNativeConfig::from_file(NativeConfig::default()),
            &prepared,
        )
        .unwrap_err()
        .kind(),
        NativeReferenceHostBuildErrorKind::TerminalConfig
    );
    let prepared: PreparedCompositionOptions = selected.with_terminal(terminal()).into();
    validate_prepared_selections(
        &LoadedNativeConfig::from_file(NativeConfig::default()),
        &prepared,
    )
    .unwrap();
}

#[test]
fn host_binds_optional_desktop_selection_without_mcp_or_launcher_effects() {
    for mcp in [false, true] {
        for invalid in [false, true] {
            let fixture = Fixture::with_options("ask", mcp, |options, _, _| {
                let environment = if invalid {
                    vec![("INVALID\0KEY".into(), "value".into())]
                } else {
                    Vec::new()
                };
                options.with_background_url_opener(executable(), environment)
            });
            assert_eq!(fixture.host().background_url_opener().is_some(), !invalid);
            assert!(fixture.host().has_background_url_selection());
            assert_eq!(fixture.host().mcp_runtime().is_some(), mcp);
            assert!(fixture.transport.requests.lock().unwrap().is_empty());
            assert_eq!(fixture.prompt.calls.load(Ordering::Relaxed), 0);
        }
    }
}

#[test]
fn absent_desktop_selection_keeps_existing_host_composition_unchanged() {
    for mcp in [false, true] {
        let fixture = Fixture::new("ask", mcp);
        assert!(fixture.host().background_url_opener().is_none());
        assert!(!fixture.host().has_background_url_selection());
    }
}
