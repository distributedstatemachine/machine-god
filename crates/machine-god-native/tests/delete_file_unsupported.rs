#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    DELETE_FILE_TOOL_NAME, DeleteFileTool, DeleteFileToolOpenErrorKind, MAX_DELETE_FILE_PATH_BYTES,
    MAX_DELETE_FILE_PATH_COMPONENTS, MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(DELETE_FILE_TOOL_NAME, "delete_file");
    assert_eq!(MAX_DELETE_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_DELETE_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES, 16_384);

    let private_root = "/PRIVATE_UNSUPPORTED_DELETE_WORKSPACE_DO_NOT_REFLECT";
    let error = DeleteFileTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native delete_file tool");

    assert_eq!(
        error.kind(),
        DeleteFileToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native delete_file is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "DeleteFileToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
