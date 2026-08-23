#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    MAX_WRITE_FILE_CHUNK_BYTES, MAX_WRITE_FILE_CONTENT_BYTES, MAX_WRITE_FILE_PATH_BYTES,
    MAX_WRITE_FILE_PATH_COMPONENTS, MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES, MAX_WRITE_FILE_TEMP_ATTEMPTS, WRITE_FILE_TOOL_NAME,
    WriteFileTool, WriteFileToolOpenErrorKind,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(WRITE_FILE_TOOL_NAME, "write_file");
    assert_eq!(MAX_WRITE_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_WRITE_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_WRITE_FILE_CONTENT_BYTES, 48 * 1_024);
    assert_eq!(MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES, 64 * 1_024);
    assert_eq!(MAX_WRITE_FILE_CHUNK_BYTES, 8 * 1_024);
    assert_eq!(MAX_WRITE_FILE_TEMP_ATTEMPTS, 8);
    assert_eq!(MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES, 16 * 1_024);

    let private_root = "/PRIVATE_UNSUPPORTED_WRITE_WORKSPACE_DO_NOT_REFLECT";
    let error = WriteFileTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native write_file tool");

    assert_eq!(
        error.kind(),
        WriteFileToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native write_file is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "WriteFileToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
