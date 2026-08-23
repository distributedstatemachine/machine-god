#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{GrepFilesTool, GrepFilesToolOpenErrorKind};

#[test]
fn unsupported_constructor_is_exact_redacted_and_effect_free() {
    let private_root = "/PRIVATE_UNSUPPORTED_GREP_WORKSPACE_DO_NOT_REFLECT";
    let error = GrepFilesTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native grep_files tool");

    assert_eq!(
        error.kind(),
        GrepFilesToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native grep_files is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "GrepFilesToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
