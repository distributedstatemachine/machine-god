#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    MAX_RENAME_FILE_PATH_BYTES, MAX_RENAME_FILE_PATH_COMPONENTS,
    MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES,
    RENAME_FILE_TOOL_NAME, RenameFileTool, RenameFileToolOpenErrorKind,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(RENAME_FILE_TOOL_NAME, "rename_file");
    assert_eq!(MAX_RENAME_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_RENAME_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES, 16_384);

    let private_root = "/PRIVATE_UNSUPPORTED_RENAME_WORKSPACE_DO_NOT_REFLECT";
    let error = RenameFileTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native rename_file tool");

    assert_eq!(
        error.kind(),
        RenameFileToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native rename_file is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "RenameFileToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
