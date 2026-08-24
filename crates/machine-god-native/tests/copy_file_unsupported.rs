#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    COPY_FILE_TOOL_NAME, CopyFileTool, CopyFileToolOpenErrorKind, MAX_COPY_FILE_CHUNK_BYTES,
    MAX_COPY_FILE_IO_CALLS, MAX_COPY_FILE_PATH_BYTES, MAX_COPY_FILE_PATH_COMPONENTS,
    MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_COPY_FILE_SERIALIZED_RESULT_BYTES,
    MAX_COPY_FILE_SOURCE_BYTES, MAX_COPY_FILE_TEMP_ATTEMPTS,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(COPY_FILE_TOOL_NAME, "copy_file");
    assert_eq!(MAX_COPY_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_COPY_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_COPY_FILE_SOURCE_BYTES, 16_777_216);
    assert_eq!(MAX_COPY_FILE_CHUNK_BYTES, 65_536);
    assert_eq!(MAX_COPY_FILE_IO_CALLS, 4_096);
    assert_eq!(MAX_COPY_FILE_TEMP_ATTEMPTS, 8);
    assert_eq!(MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_COPY_FILE_SERIALIZED_RESULT_BYTES, 16_384);

    let private_root = "/PRIVATE_UNSUPPORTED_COPY_WORKSPACE_DO_NOT_REFLECT";
    let error = CopyFileTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native copy_file tool");

    assert_eq!(error.kind(), CopyFileToolOpenErrorKind::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "native copy_file is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "CopyFileToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
