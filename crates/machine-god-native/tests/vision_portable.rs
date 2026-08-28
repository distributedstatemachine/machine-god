use machine_god_core::{BoxFuture, SessionId};
use machine_god_native::{
    MAX_VISION_ATTEMPT_EVIDENCE_BYTES, MAX_VISION_BATCH_IMAGES, MAX_VISION_BATCH_RAW_BYTES,
    MAX_VISION_EVIDENCE_LIST_ITEMS, MAX_VISION_EVIDENCE_STRING_BYTES, MAX_VISION_FOCUS_BYTES,
    VisionBatchRequest, VisionBatchResponse, VisionDeadline, VisionImage, VisionImageOutcome,
    VisionImageResult, VisionMediaType, VisionProviderFailure, VisionProviderFailureCode,
    VisionTransportError, VisionTransportErrorKind,
};
use std::time::Instant;

struct PortableDeadline;

impl VisionDeadline for PortableDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        Box::pin(async { Ok(()) })
    }
}

fn session() -> SessionId {
    SessionId::new("vision-portable-session").unwrap()
}

fn image(id: u64, bytes: usize) -> VisionImage {
    VisionImage::new(
        id,
        VisionMediaType::Png,
        vec![u8::try_from(id).unwrap(); bytes],
    )
    .unwrap()
}

fn success(id: u64, summary: String) -> VisionImageResult {
    VisionImageResult::new(
        id,
        VisionImageOutcome::Ok {
            summary,
            visible_text: Vec::new(),
            details: Vec::new(),
        },
    )
    .unwrap()
}

#[test]
fn request_accepts_exact_bounds_and_preserves_order() {
    let request = VisionBatchRequest::new(
        session(),
        "f".repeat(MAX_VISION_FOCUS_BYTES),
        (1..=MAX_VISION_BATCH_IMAGES as u64)
            .map(|id| image(id, MAX_VISION_BATCH_RAW_BYTES / MAX_VISION_BATCH_IMAGES))
            .collect(),
    )
    .unwrap();

    assert_eq!(request.images().len(), MAX_VISION_BATCH_IMAGES);
    assert_eq!(request.images()[0].image_id(), 1);
    assert_eq!(request.images()[7].image_id(), 8);
    assert_eq!(request.focus().len(), MAX_VISION_FOCUS_BYTES);
}

#[test]
fn request_rejects_blank_or_oversized_focus_and_invalid_image_sets() {
    let cases = [
        VisionBatchRequest::new(session(), " \n".to_owned(), vec![image(1, 1)]),
        VisionBatchRequest::new(
            session(),
            "f".repeat(MAX_VISION_FOCUS_BYTES + 1),
            vec![image(1, 1)],
        ),
        VisionBatchRequest::new(session(), "focus".to_owned(), Vec::new()),
        VisionBatchRequest::new(
            session(),
            "focus".to_owned(),
            (1..=MAX_VISION_BATCH_IMAGES as u64 + 1)
                .map(|id| image(id, 1))
                .collect(),
        ),
        VisionBatchRequest::new(
            session(),
            "focus".to_owned(),
            vec![image(1, 1), image(1, 1)],
        ),
        VisionBatchRequest::new(
            session(),
            "focus".to_owned(),
            vec![image(1, MAX_VISION_BATCH_RAW_BYTES), image(2, 1)],
        ),
    ];

    for result in cases {
        assert_eq!(
            result.unwrap_err().kind(),
            VisionTransportErrorKind::InvalidRequest
        );
    }
    assert_eq!(
        VisionImage::new(0, VisionMediaType::Png, vec![1])
            .unwrap_err()
            .kind(),
        VisionTransportErrorKind::InvalidRequest
    );
    assert_eq!(
        VisionImage::new(1, VisionMediaType::Png, Vec::new())
            .unwrap_err()
            .kind(),
        VisionTransportErrorKind::InvalidRequest
    );
}

#[test]
fn result_enforces_leaf_list_and_aggregate_evidence_bounds() {
    assert!(
        success(1, "s".repeat(MAX_VISION_EVIDENCE_STRING_BYTES))
            .outcome()
            .eq(&VisionImageOutcome::Ok {
                summary: "s".repeat(MAX_VISION_EVIDENCE_STRING_BYTES),
                visible_text: Vec::new(),
                details: Vec::new(),
            })
    );

    for outcome in [
        VisionImageOutcome::Ok {
            summary: String::new(),
            visible_text: Vec::new(),
            details: Vec::new(),
        },
        VisionImageOutcome::Ok {
            summary: "s".repeat(MAX_VISION_EVIDENCE_STRING_BYTES + 1),
            visible_text: Vec::new(),
            details: Vec::new(),
        },
        VisionImageOutcome::Ok {
            summary: "ok".to_owned(),
            visible_text: vec![String::new(); MAX_VISION_EVIDENCE_LIST_ITEMS + 1],
            details: Vec::new(),
        },
        VisionImageOutcome::Ok {
            summary: "ok".to_owned(),
            visible_text: vec!["v".repeat(MAX_VISION_EVIDENCE_STRING_BYTES + 1)],
            details: Vec::new(),
        },
    ] {
        assert_eq!(
            VisionImageResult::new(1, outcome).unwrap_err().kind(),
            VisionTransportErrorKind::InvalidResponse
        );
    }

    let first = success(1, "a".repeat(7_000));
    let second = success(2, "b".repeat(7_000));
    let third = success(3, "c".repeat(MAX_VISION_ATTEMPT_EVIDENCE_BYTES - 13_999));
    assert_eq!(
        VisionBatchResponse::new(vec![first, second, third])
            .unwrap_err()
            .kind(),
        VisionTransportErrorKind::InvalidResponse
    );
}

#[test]
fn stable_failure_diagnostics_match_upstream_codes() {
    let cases = [
        (
            VisionProviderFailureCode::ImageUnavailable,
            "image_unavailable",
            false,
        ),
        (
            VisionProviderFailureCode::ProviderResponseInvalid,
            "provider_response_invalid",
            true,
        ),
        (
            VisionProviderFailureCode::OutputLimitExceeded,
            "output_limit_exceeded",
            true,
        ),
        (
            VisionProviderFailureCode::VisionUnavailable,
            "vision_unavailable",
            true,
        ),
        (
            VisionProviderFailureCode::MissingProviderRecord,
            "missing_provider_record",
            true,
        ),
    ];
    for (code, rendered, retryable) in cases {
        let failure = VisionProviderFailure::new(code);
        assert_eq!(failure.code().as_str(), rendered);
        assert_eq!(failure.retryable(), retryable);
        assert!(!failure.message().is_empty());
        assert!(!failure.suggestion().is_empty());
    }
    assert_eq!(
        VisionProviderFailure::new(VisionProviderFailureCode::ImageUnavailable).suggestion(),
        "Supply an available PNG, JPEG, GIF, or WebP image within the documented limits."
    );
    futures_executor::block_on(PortableDeadline.wait_until(Instant::now())).unwrap();
}

#[test]
fn debug_is_redacted_for_images_focus_session_and_evidence() {
    let request = VisionBatchRequest::new(
        SessionId::new("PRIVATE_SESSION_SENTINEL").unwrap(),
        "PRIVATE_FOCUS_SENTINEL".to_owned(),
        vec![
            VisionImage::new(1, VisionMediaType::Webp, b"PRIVATE_IMAGE_SENTINEL".to_vec()).unwrap(),
        ],
    )
    .unwrap();
    let result = success(1, "PRIVATE_EVIDENCE_SENTINEL".to_owned());
    let rendered = format!("{request:?} {:?}", request.images()[0]);
    let result_rendered = format!("{result:?}");
    for private in [
        "PRIVATE_SESSION_SENTINEL",
        "PRIVATE_FOCUS_SENTINEL",
        "PRIVATE_IMAGE_SENTINEL",
        "PRIVATE_EVIDENCE_SENTINEL",
    ] {
        assert!(!rendered.contains(private));
        assert!(!result_rendered.contains(private));
    }
}
