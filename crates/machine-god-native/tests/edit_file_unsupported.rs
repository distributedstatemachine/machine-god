#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    EDIT_FILE_TOOL_NAME, EditFileTool, EditFileToolOpenErrorKind, MAX_EDIT_FILE_CHUNK_BYTES,
    MAX_EDIT_FILE_EXISTING_BYTES, MAX_EDIT_FILE_MATCH_WORK_STEPS, MAX_EDIT_FILE_NEW_STRING_BYTES,
    MAX_EDIT_FILE_OLD_STRING_BYTES, MAX_EDIT_FILE_PATH_BYTES, MAX_EDIT_FILE_PATH_COMPONENTS,
    MAX_EDIT_FILE_RESULTING_BYTES, MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES, MAX_EDIT_FILE_TEMP_ATTEMPTS,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(EDIT_FILE_TOOL_NAME, "edit_file");
    assert_eq!(MAX_EDIT_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_EDIT_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_EDIT_FILE_OLD_STRING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_NEW_STRING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES, 64 * 1_024);
    assert_eq!(MAX_EDIT_FILE_EXISTING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_RESULTING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_CHUNK_BYTES, 8 * 1_024);
    assert_eq!(MAX_EDIT_FILE_MATCH_WORK_STEPS, 393_216);
    assert_eq!(MAX_EDIT_FILE_TEMP_ATTEMPTS, 8);
    assert_eq!(MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES, 16 * 1_024);

    let private_root = "/PRIVATE_UNSUPPORTED_EDIT_WORKSPACE_DO_NOT_REFLECT";
    let error = EditFileTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native edit_file tool");

    assert_eq!(error.kind(), EditFileToolOpenErrorKind::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "native edit_file is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "EditFileToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
