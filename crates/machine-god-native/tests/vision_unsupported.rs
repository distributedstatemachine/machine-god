#![cfg(all(
    feature = "vision",
    not(target_family = "wasm"),
    not(any(target_os = "linux", target_os = "macos"))
))]

use std::error::Error;
use std::future;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use machine_god_core::{BoxFuture, CancellationToken, NetworkTarget};
use machine_god_native::{
    MAX_VISION_IMAGE_BYTES, MAX_VISION_IMAGES, VISION_TOOL_NAME, VisionBatchRequest,
    VisionBatchResponse, VisionConfigErrorKind, VisionDeadline, VisionTool, VisionTransport,
    VisionTransportError,
};

struct UnreachableTransport;

impl VisionTransport for UnreachableTransport {
    fn analyze(
        &self,
        _request: VisionBatchRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<VisionBatchResponse, VisionTransportError>> {
        Box::pin(future::pending())
    }
}

struct UnreachableDeadline;

impl VisionDeadline for UnreachableDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        Box::pin(future::pending())
    }
}

#[test]
fn unsupported_native_api_is_exported_redacted_and_effect_free() {
    assert_eq!(VISION_TOOL_NAME, "vision");
    assert_eq!(MAX_VISION_IMAGES, 20);
    assert_eq!(MAX_VISION_IMAGE_BYTES, 8 * 1024 * 1024);

    let private_root = "/PRIVATE_UNSUPPORTED_VISION_WORKSPACE_DO_NOT_REFLECT";
    let error = VisionTool::with_transport(
        Path::new(private_root),
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "ai-gateway.vercel.sh".to_owned(),
            port: None,
        },
        Arc::new(UnreachableTransport),
        Arc::new(UnreachableDeadline),
    )
    .expect_err("unsupported targets cannot construct the native vision tool");

    assert_eq!(error.kind(), VisionConfigErrorKind::UnsupportedPlatform);
    assert_eq!(
        error.to_string(),
        "native vision is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "VisionConfigError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));

    let invalid_target_error = VisionTool::with_transport(
        Path::new(private_root),
        NetworkTarget {
            scheme: "HTTPS".to_owned(),
            host: "PRIVATE.INVALID".to_owned(),
            port: Some(443),
        },
        Arc::new(UnreachableTransport),
        Arc::new(UnreachableDeadline),
    )
    .expect_err("unsupported classification precedes target validation");
    assert_eq!(
        invalid_target_error.kind(),
        VisionConfigErrorKind::UnsupportedPlatform
    );
    assert!(!format!("{invalid_target_error:?}").contains("PRIVATE"));
}
