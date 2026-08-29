#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{
    MAX_MEMORY_FACT_BYTES, MAX_MEMORY_FACTS, MAX_MEMORY_FILE_BYTES, MAX_MEMORY_IO_ATTEMPTS,
    MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES, MAX_MEMORY_SERIALIZED_RESULT_BYTES,
    MAX_MEMORY_TOTAL_FACT_BYTES, MEMORY_SCHEMA_VERSION, MEMORY_TOOL_NAME, MemoryTool,
    MemoryToolOpenErrorKind,
};

#[test]
fn unsupported_api_is_exact_redacted_and_effect_free() {
    assert_eq!(MEMORY_SCHEMA_VERSION, 1);
    assert_eq!(MEMORY_TOOL_NAME, "memory");
    assert_eq!(MAX_MEMORY_FACT_BYTES, 4_096);
    assert_eq!(MAX_MEMORY_FACTS, 128);
    assert_eq!(MAX_MEMORY_TOTAL_FACT_BYTES, 32_768);
    assert_eq!(MAX_MEMORY_FILE_BYTES, 49_152);
    assert_eq!(MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES, 32_768);
    assert_eq!(MAX_MEMORY_SERIALIZED_RESULT_BYTES, 65_536);
    assert_eq!(MAX_MEMORY_IO_ATTEMPTS, 65_536);

    let private_root = "/PRIVATE_UNSUPPORTED_MEMORY_STATE_DO_NOT_REFLECT";
    assert!(!Path::new(private_root).exists());
    let error = MemoryTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct native memory");
    assert_eq!(error.kind(), MemoryToolOpenErrorKind::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "native memory is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "MemoryToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
    assert!(!Path::new(private_root).exists());
}
