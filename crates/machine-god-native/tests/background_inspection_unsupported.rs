#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::PathBuf;

use futures_executor::block_on;
use machine_god_native::{
    NativeBackgroundInspectionErrorKind, NativeBackgroundQuery, NativeEnvironment,
    inspect_native_background, inspect_process_background, run_background_process_helper,
};

#[test]
fn unsupported_target_is_active_and_fixed() {
    let injected = block_on(inspect_native_background(
        NativeEnvironment::new(None, None, None),
        PathBuf::from("ignored"),
        NativeBackgroundQuery::List,
    ))
    .unwrap_err();
    assert_eq!(
        injected.kind(),
        NativeBackgroundInspectionErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        injected.to_string(),
        "native background inspection is unsupported on this platform"
    );

    let process = block_on(inspect_process_background(NativeBackgroundQuery::Last)).unwrap_err();
    assert_eq!(
        process.kind(),
        NativeBackgroundInspectionErrorKind::UnsupportedPlatform
    );
}

#[test]
fn process_helper_refuses_unsupported_targets_without_reading_input() {
    let error = run_background_process_helper().unwrap_err();
    assert_eq!(format!("{:?}", error.kind()), "Unsupported");
    assert_eq!(error.to_string(), "background process operation failed");
    assert_eq!(
        format!("{error:?}"),
        "BackgroundProcessError { kind: Unsupported }"
    );
    assert!(error.source().is_none());
}
