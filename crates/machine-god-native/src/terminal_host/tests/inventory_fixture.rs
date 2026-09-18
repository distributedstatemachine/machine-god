//! Pure fixture generation; selection is captured once by the caller.
use std::fmt::Write;
use std::path::Path;

pub(super) fn dispatch(test_executable: &Path, selected: Option<&Path>) -> String {
    let quote = |path: &Path| {
        path.to_str()
            .expect("fixture helper executable path must be UTF-8")
            .replace('\'', "'\\''")
    };
    let mut script = String::new();
    for (flag, entry) in [
        (
            crate::PROCESS_INVENTORY_HELPER_ARGUMENT,
            "process_inventory_helper::tests::helper_entry",
        ),
        (
            crate::PROCESS_INVENTORY_SERVICE_ARGUMENT,
            "process_inventory_protocol::tests::service_entry",
        ),
    ] {
        writeln!(script, "if [ \"$1\" = '{flag}' ]; then").unwrap();
        if let Some(program) = selected {
            // Preserve the sole original private flag and production stdout.
            // A bad explicit program must fail, never retry through libtest.
            writeln!(script, "exec '{}' '{flag}'", quote(program)).unwrap();
        } else {
            writeln!(
                script,
                "exec '{}' --exact {entry} --nocapture 2>&1 1>/dev/null",
                quote(test_executable)
            )
            .unwrap();
        }
        script.push_str("fi\n");
    }
    script
}

#[test]
fn explicit_production_inventory_and_service_keep_raw_stdout_and_exact_flags() {
    let script = dispatch(
        Path::new("/debug/test binary"),
        Some(Path::new("/selected/日本語's helper")),
    );
    assert_eq!(
        script,
        concat!(
            "if [ \"$1\" = '--machine-god-process-inventory-helper' ]; then\n",
            "exec '/selected/日本語'\\''s helper' '--machine-god-process-inventory-helper'\n",
            "fi\n",
            "if [ \"$1\" = '--machine-god-process-inventory-service' ]; then\n",
            "exec '/selected/日本語'\\''s helper' '--machine-god-process-inventory-service'\n",
            "fi\n",
        )
    );
}

#[test]
fn absent_selection_keeps_only_explicit_registered_debug_entries() {
    let script = dispatch(Path::new("/debug/test's binary"), None);
    assert_eq!(
        script,
        concat!(
            "if [ \"$1\" = '--machine-god-process-inventory-helper' ]; then\n",
            "exec '/debug/test'\\''s binary' --exact process_inventory_helper::tests::helper_entry --nocapture 2>&1 1>/dev/null\n",
            "fi\n",
            "if [ \"$1\" = '--machine-god-process-inventory-service' ]; then\n",
            "exec '/debug/test'\\''s binary' --exact process_inventory_protocol::tests::service_entry --nocapture 2>&1 1>/dev/null\n",
            "fi\n",
        )
    );
}

#[test]
fn invalid_explicit_selection_never_generates_a_debug_fallback() {
    for program in ["", "relative-missing-helper", "/missing/inventory-helper"] {
        let script = dispatch(Path::new("/debug/test-binary"), Some(Path::new(program)));
        assert!(script.contains(&format!("exec '{program}' ")));
        assert!(!script.contains("/debug/test-binary"));
        assert!(!script.contains("--exact"));
        assert!(!script.contains("1>/dev/null"));
        assert!(!script.contains("2>&1"));
    }
}
