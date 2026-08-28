//! Portable, provider-independent contracts for bounded native vision batches.

use machine_god_core::{BoxFuture, CancellationToken, SessionId};
use std::fmt;
use std::time::Instant;

/// Maximum UTF-8 bytes in a vision focus.
pub const MAX_VISION_FOCUS_BYTES: usize = 4 * 1024;
/// Maximum images sent in one provider request.
pub const MAX_VISION_BATCH_IMAGES: usize = 8;
/// Maximum aggregate decoded image bytes in one provider request.
pub const MAX_VISION_BATCH_RAW_BYTES: usize = 8 * 1024 * 1024;
/// Maximum serialized provider request bytes.
pub const MAX_VISION_REQUEST_BYTES: usize = 12 * 1024 * 1024;
/// Maximum model-produced evidence retained from one attempt.
pub const MAX_VISION_ATTEMPT_EVIDENCE_BYTES: usize = 20 * 1024;
/// Maximum one successful summary or evidence-list string.
pub const MAX_VISION_EVIDENCE_STRING_BYTES: usize = 8 * 1024;
/// Maximum strings in either evidence list for one image.
pub const MAX_VISION_EVIDENCE_LIST_ITEMS: usize = 128;
/// Maximum complete provider response stream.
pub const MAX_VISION_RESPONSE_BYTES: usize = 64 * 1024;
/// Maximum one provider response record.
pub const MAX_VISION_RESPONSE_RECORD_BYTES: usize = 64 * 1024;
/// Maximum strict JSON nodes decoded across one provider response stream.
pub const MAX_VISION_RESPONSE_JSON_NODES: usize = 4_096;
/// Maximum response records decoded from one provider response stream.
pub const MAX_VISION_RESPONSE_RECORDS: usize = 128;

/// Supported image media types after local magic-byte verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VisionMediaType {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl VisionMediaType {
    /// Returns the provider wire MIME type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

/// One verified, fully owned image snapshot.
#[derive(Eq, PartialEq)]
pub struct VisionImage {
    image_id: u64,
    media_type: VisionMediaType,
    bytes: Vec<u8>,
}

impl VisionImage {
    /// Constructs a nonempty image with a positive stable identifier.
    ///
    /// Aggregate batch size is checked by [`VisionBatchRequest::new`].
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-request error for ID zero or empty bytes.
    pub fn new(
        image_id: u64,
        media_type: VisionMediaType,
        bytes: Vec<u8>,
    ) -> Result<Self, VisionTransportError> {
        if image_id == 0 || bytes.is_empty() {
            return Err(VisionTransportError::new(
                VisionTransportErrorKind::InvalidRequest,
            ));
        }
        Ok(Self {
            image_id,
            media_type,
            bytes,
        })
    }

    #[must_use]
    pub const fn image_id(&self) -> u64 {
        self.image_id
    }

    #[must_use]
    pub const fn media_type(&self) -> VisionMediaType {
        self.media_type
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for VisionImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionImage")
            .field("image_id", &self.image_id)
            .field("media_type", &self.media_type)
            .field(
                "bytes",
                &format_args!("<redacted:{} bytes>", self.bytes.len()),
            )
            .finish()
    }
}

/// One validated, fully owned provider batch.
#[derive(Eq, PartialEq)]
pub struct VisionBatchRequest {
    session_id: SessionId,
    focus: String,
    images: Vec<VisionImage>,
}

impl VisionBatchRequest {
    /// Constructs a bounded request without performing I/O.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-request error when the focus is blank or too
    /// long, the image count is outside `1..=8`, image IDs repeat, or aggregate
    /// raw image bytes exceed 8 MiB.
    pub fn new(
        session_id: SessionId,
        focus: String,
        images: Vec<VisionImage>,
    ) -> Result<Self, VisionTransportError> {
        if focus.trim().is_empty()
            || focus.len() > MAX_VISION_FOCUS_BYTES
            || !(1..=MAX_VISION_BATCH_IMAGES).contains(&images.len())
        {
            return Err(VisionTransportError::new(
                VisionTransportErrorKind::InvalidRequest,
            ));
        }
        let mut total = 0_usize;
        for (index, image) in images.iter().enumerate() {
            if images[..index]
                .iter()
                .any(|previous| previous.image_id == image.image_id)
            {
                return Err(VisionTransportError::new(
                    VisionTransportErrorKind::InvalidRequest,
                ));
            }
            total = total
                .checked_add(image.bytes.len())
                .filter(|total| *total <= MAX_VISION_BATCH_RAW_BYTES)
                .ok_or_else(|| {
                    VisionTransportError::new(VisionTransportErrorKind::InvalidRequest)
                })?;
        }
        Ok(Self {
            session_id,
            focus,
            images,
        })
    }

    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    #[must_use]
    pub fn focus(&self) -> &str {
        &self.focus
    }

    #[must_use]
    pub fn images(&self) -> &[VisionImage] {
        &self.images
    }
}

impl fmt::Debug for VisionBatchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionBatchRequest")
            .field("session_id", &"<redacted>")
            .field("focus", &"<redacted>")
            .field("image_count", &self.images.len())
            .field(
                "image_bytes",
                &self
                    .images
                    .iter()
                    .map(|image| image.bytes.len())
                    .sum::<usize>(),
            )
            .finish()
    }
}

/// Stable per-image failure code projected by the vision tool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VisionProviderFailureCode {
    ImageUnavailable,
    ProviderResponseInvalid,
    OutputLimitExceeded,
    VisionUnavailable,
    MissingProviderRecord,
}

impl VisionProviderFailureCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ImageUnavailable => "image_unavailable",
            Self::ProviderResponseInvalid => "provider_response_invalid",
            Self::OutputLimitExceeded => "output_limit_exceeded",
            Self::VisionUnavailable => "vision_unavailable",
            Self::MissingProviderRecord => "missing_provider_record",
        }
    }
}

/// Fixed, path-free diagnostic for one failed image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisionProviderFailure {
    code: VisionProviderFailureCode,
}

impl VisionProviderFailure {
    #[must_use]
    pub const fn new(code: VisionProviderFailureCode) -> Self {
        Self { code }
    }

    #[must_use]
    pub const fn code(self) -> VisionProviderFailureCode {
        self.code
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self.code {
            VisionProviderFailureCode::ImageUnavailable => {
                "Vision could not safely load or verify this image."
            }
            VisionProviderFailureCode::ProviderResponseInvalid => {
                "Vision received an invalid provider response after one retry."
            }
            VisionProviderFailureCode::OutputLimitExceeded => {
                "Vision exceeded the configured evidence budget."
            }
            VisionProviderFailureCode::VisionUnavailable => {
                "Vision could not obtain a usable provider response."
            }
            VisionProviderFailureCode::MissingProviderRecord => {
                "Vision returned no record for this image."
            }
        }
    }

    #[must_use]
    pub const fn retryable(self) -> bool {
        !matches!(self.code, VisionProviderFailureCode::ImageUnavailable)
    }

    #[must_use]
    pub const fn suggestion(self) -> &'static str {
        match self.code {
            VisionProviderFailureCode::ImageUnavailable => {
                "Supply an available PNG, JPEG, GIF, or WebP image within the documented limits."
            }
            VisionProviderFailureCode::ProviderResponseInvalid => {
                "Try a later explicit Vision call if useful, or continue without visual claims."
            }
            VisionProviderFailureCode::OutputLimitExceeded => {
                "Call Vision again with a narrower focus or fewer images."
            }
            VisionProviderFailureCode::VisionUnavailable => {
                "Retry later, change model or strategy, or continue without visual claims."
            }
            VisionProviderFailureCode::MissingProviderRecord => {
                "Call Vision again for only this image if its evidence is still needed."
            }
        }
    }
}

/// Provider outcome for one requested image.
#[derive(Clone, Eq, PartialEq)]
pub enum VisionImageOutcome {
    Ok {
        summary: String,
        visible_text: Vec<String>,
        details: Vec<String>,
    },
    Failed {
        error: VisionProviderFailure,
    },
}

impl fmt::Debug for VisionImageOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok {
                visible_text,
                details,
                ..
            } => formatter
                .debug_struct("Ok")
                .field("summary", &"<redacted>")
                .field("visible_text_items", &visible_text.len())
                .field("detail_items", &details.len())
                .finish(),
            Self::Failed { error } => formatter
                .debug_struct("Failed")
                .field("code", &error.code())
                .finish(),
        }
    }
}

/// One provider record, retaining its stable image ID.
#[derive(Clone, Eq, PartialEq)]
pub struct VisionImageResult {
    image_id: u64,
    outcome: VisionImageOutcome,
}

impl VisionImageResult {
    /// Constructs one positive-ID result.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-response error for ID zero or evidence outside
    /// the explicit string and list bounds.
    pub fn new(image_id: u64, outcome: VisionImageOutcome) -> Result<Self, VisionTransportError> {
        if image_id == 0 || !valid_outcome(&outcome) {
            return Err(VisionTransportError::new(
                VisionTransportErrorKind::InvalidResponse,
            ));
        }
        Ok(Self { image_id, outcome })
    }

    #[must_use]
    pub const fn image_id(&self) -> u64 {
        self.image_id
    }

    #[must_use]
    pub const fn outcome(&self) -> &VisionImageOutcome {
        &self.outcome
    }

    #[must_use]
    pub fn into_outcome(self) -> VisionImageOutcome {
        self.outcome
    }
}

impl fmt::Debug for VisionImageResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionImageResult")
            .field("image_id", &self.image_id)
            .field("outcome", &self.outcome)
            .finish()
    }
}

/// One bounded provider batch response.
#[derive(Clone, Eq, PartialEq)]
pub struct VisionBatchResponse {
    images: Vec<VisionImageResult>,
}

impl VisionBatchResponse {
    /// Constructs a nonempty batch containing at most eight unique records.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-response error for invalid count, duplicate IDs,
    /// or more than 20 KiB of aggregate retained evidence.
    pub fn new(images: Vec<VisionImageResult>) -> Result<Self, VisionTransportError> {
        if !(1..=MAX_VISION_BATCH_IMAGES).contains(&images.len())
            || images.iter().enumerate().any(|(index, image)| {
                images[..index]
                    .iter()
                    .any(|previous| previous.image_id == image.image_id)
            })
            || retained_evidence_bytes(&images)
                .is_none_or(|bytes| bytes > MAX_VISION_ATTEMPT_EVIDENCE_BYTES)
        {
            return Err(VisionTransportError::new(
                VisionTransportErrorKind::InvalidResponse,
            ));
        }
        Ok(Self { images })
    }

    #[must_use]
    pub fn images(&self) -> &[VisionImageResult] {
        &self.images
    }

    #[must_use]
    pub fn into_images(self) -> Vec<VisionImageResult> {
        self.images
    }
}

fn valid_outcome(outcome: &VisionImageOutcome) -> bool {
    let VisionImageOutcome::Ok {
        summary,
        visible_text,
        details,
    } = outcome
    else {
        return true;
    };
    !summary.trim().is_empty()
        && summary.len() <= MAX_VISION_EVIDENCE_STRING_BYTES
        && visible_text.len() <= MAX_VISION_EVIDENCE_LIST_ITEMS
        && details.len() <= MAX_VISION_EVIDENCE_LIST_ITEMS
        && visible_text
            .iter()
            .chain(details)
            .all(|value| value.len() <= MAX_VISION_EVIDENCE_STRING_BYTES)
}

fn retained_evidence_bytes(images: &[VisionImageResult]) -> Option<usize> {
    images.iter().try_fold(0_usize, |total, image| {
        let VisionImageOutcome::Ok {
            summary,
            visible_text,
            details,
        } = &image.outcome
        else {
            return Some(total);
        };
        visible_text
            .iter()
            .chain(details)
            .try_fold(total.checked_add(summary.len())?, |subtotal, value| {
                subtotal.checked_add(value.len())
            })
    })
}

impl fmt::Debug for VisionBatchResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionBatchResponse")
            .field("image_count", &self.images.len())
            .finish()
    }
}

/// Stable failure category produced by a [`VisionTransport`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VisionTransportErrorKind {
    InvalidRequest,
    Authentication,
    RateLimited,
    Timeout,
    Unavailable,
    InvalidResponse,
    Protocol,
    ResponseTooLarge,
    RuntimeRequired,
    Cancelled,
}

/// Fixed, data-free vision transport failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct VisionTransportError {
    kind: VisionTransportErrorKind,
}

impl VisionTransportError {
    #[must_use]
    pub const fn new(kind: VisionTransportErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> VisionTransportErrorKind {
        self.kind
    }
}

impl fmt::Debug for VisionTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionTransportError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for VisionTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("vision transport failed")
    }
}

impl std::error::Error for VisionTransportError {}

/// Provider-independent vision transport over already verified image bytes.
pub trait VisionTransport: Send + Sync + 'static {
    fn analyze(
        &self,
        request: VisionBatchRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<VisionBatchResponse, VisionTransportError>>;
}

/// Runtime-neutral absolute deadline source injected by a native host.
pub trait VisionDeadline: Send + Sync + 'static {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>>;
}
