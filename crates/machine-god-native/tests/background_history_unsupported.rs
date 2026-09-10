#![cfg(any(
    not(feature = "ai-gateway-http"),
    not(any(target_os = "linux", target_os = "macos"))
))]

use machine_god_native::{
    NativeBackgroundHistoryQuery, NativeBackgroundInspectionErrorKind, NativeEnvironment,
    inspect_native_background_history,
};

#[test]
fn unavailable_terminal_graph_never_reports_a_complete_legacy_only_union() {
    for query in [
        NativeBackgroundHistoryQuery::List,
        NativeBackgroundHistoryQuery::Last,
        NativeBackgroundHistoryQuery::Terminal(
            machine_god_core::TerminalSessionId::new("terminal-00000000000000000000000000000001")
                .unwrap(),
        ),
    ] {
        let result = futures_executor::block_on(inspect_native_background_history(
            NativeEnvironment::new(None, None, None),
            "/absent-workspace".into(),
            query,
        ));
        assert_eq!(
            result.unwrap_err().kind(),
            NativeBackgroundInspectionErrorKind::UnsupportedPlatform
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn legacy_exact_preserves_its_original_reader_without_terminal_feature() {
    let result = futures_executor::block_on(inspect_native_background_history(
        NativeEnvironment::new(None, None, None),
        "/absent-workspace".into(),
        NativeBackgroundHistoryQuery::Legacy(1),
    ));
    assert_eq!(
        result.unwrap_err().kind(),
        NativeBackgroundInspectionErrorKind::Unavailable
    );
}
