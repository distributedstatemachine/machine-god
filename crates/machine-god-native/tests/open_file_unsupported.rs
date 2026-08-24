#![cfg(not(target_os = "linux"))]

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use machine_god_native::{
    MAX_CONCURRENT_OPEN_FILE_LAUNCHES, MAX_OPEN_FILE_PATH_BYTES,
    MAX_OPEN_FILE_PATH_COMPONENT_BYTES, MAX_OPEN_FILE_PATH_COMPONENTS,
    MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES,
    OPEN_FILE_LAUNCH_TIMEOUT, OPEN_FILE_TOOL_NAME, OpenFileTool, OpenFileToolOpenErrorKind,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(OPEN_FILE_TOOL_NAME, "open_file");
    assert_eq!(MAX_OPEN_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_OPEN_FILE_PATH_COMPONENT_BYTES, 255);
    assert_eq!(MAX_OPEN_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES, 16_384);
    assert_eq!(MAX_CONCURRENT_OPEN_FILE_LAUNCHES, 32);
    assert_eq!(OPEN_FILE_LAUNCH_TIMEOUT, Duration::from_secs(30));

    let private_root = "/PRIVATE_UNSUPPORTED_OPEN_FILE_WORKSPACE_DO_NOT_REFLECT";
    assert!(!Path::new(private_root).exists());
    let error = OpenFileTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native open_file tool");

    assert_eq!(error.kind(), OpenFileToolOpenErrorKind::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "native open_file is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "OpenFileToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
    assert!(!Path::new(private_root).exists());
}
