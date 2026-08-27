#![cfg(not(target_os = "linux"))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{TerminalConfigErrorKind, TerminalTool};

#[test]
fn system_executor_is_fixed_redacted_and_effect_free_on_unsupported_targets() {
    let private_root = "/PRIVATE_UNSUPPORTED_TERMINAL_WORKSPACE_DO_NOT_REFLECT";
    assert!(!Path::new(private_root).exists());

    let error = TerminalTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the system terminal executor");

    assert_eq!(error.kind(), TerminalConfigErrorKind::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "native terminal execution is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "TerminalConfigError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
    assert!(!Path::new(private_root).exists());
}
