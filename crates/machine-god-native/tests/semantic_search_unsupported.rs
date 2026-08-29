#![cfg(not(target_os = "linux"))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{SemanticSearchTool, SemanticSearchToolOpenErrorKind};

#[test]
fn unsupported_constructor_is_exact_redacted_and_effect_free() {
    let private_root = "/PRIVATE_UNSUPPORTED_SEMANTIC_WORKSPACE_DO_NOT_REFLECT";
    let error = SemanticSearchTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct native semantic_search");

    assert_eq!(
        error.kind(),
        SemanticSearchToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native semantic_search is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "SemanticSearchToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
