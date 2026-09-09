#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use futures_executor::block_on;
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeOwnedWorkerScope, TerminalTapeRecorder, TerminalTapeRecordingDestination,
    TerminalTapeRecordingError, TerminalTapeRecordingOptions, TerminalTapeRecordingRequest,
};

#[test]
fn unsupported_recording_rejects_before_native_worker_or_path_effects() {
    let scope = NativeOwnedWorkerScope::new();
    let request = TerminalTapeRecordingRequest {
        destination: TerminalTapeRecordingDestination::Explicit(
            "/PRIVATE_UNSUPPORTED_TAPE_PATH".into(),
        ),
        options: TerminalTapeRecordingOptions::new(80, 24, 0, b"test".to_vec()),
    };
    let error = block_on(TerminalTapeRecorder::start(
        request,
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(error, TerminalTapeRecordingError::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "terminal recording failed: UnsupportedPlatform"
    );
    scope.close();
    assert!(scope.completion().is_complete());
}
