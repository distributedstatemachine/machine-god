#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    CREATE_FOLDER_TOOL_NAME, CreateFolderTool, CreateFolderToolOpenErrorKind,
    MAX_CREATE_FOLDER_MKDIR_CALLS, MAX_CREATE_FOLDER_PATH_BYTES, MAX_CREATE_FOLDER_PATH_COMPONENTS,
    MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES, MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES,
    MAX_CREATE_FOLDER_SYNC_CALLS,
};

#[test]
fn unsupported_api_constants_and_constructor_are_exact_redacted_and_effect_free() {
    assert_eq!(CREATE_FOLDER_TOOL_NAME, "create_folder");
    assert_eq!(MAX_CREATE_FOLDER_PATH_BYTES, 4_096);
    assert_eq!(MAX_CREATE_FOLDER_PATH_COMPONENTS, 256);
    assert_eq!(MAX_CREATE_FOLDER_MKDIR_CALLS, 256);
    assert_eq!(MAX_CREATE_FOLDER_SYNC_CALLS, 4_112);
    assert_eq!(MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES, 16_384);

    let private_root = "/PRIVATE_UNSUPPORTED_CREATE_FOLDER_WORKSPACE_DO_NOT_REFLECT";
    assert!(!Path::new(private_root).exists());
    let error = CreateFolderTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct the native create_folder tool");

    assert_eq!(
        error.kind(),
        CreateFolderToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native create_folder is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "CreateFolderToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
    assert!(!Path::new(private_root).exists());
}
