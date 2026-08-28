//! Bounded, permission-gated inspection of workspace images.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, NetworkTarget, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde::Serialize;
use serde_json::{Value, json};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BTreeMap;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::future::{Future, poll_fn};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::pin::Pin;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::task::Poll;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::vision_portable::{
    MAX_VISION_BATCH_IMAGES, VisionBatchRequest, VisionImage, VisionImageOutcome, VisionMediaType,
    VisionTransportError, VisionTransportErrorKind,
};
use crate::vision_portable::{
    MAX_VISION_BATCH_RAW_BYTES, MAX_VISION_FOCUS_BYTES, VisionDeadline, VisionProviderFailure,
    VisionProviderFailureCode, VisionTransport,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::OwnedFd;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{FileType, Mode, OFlags};

use crate::session_store::JsonValueOwner;

/// Model-visible tool name.
pub const VISION_TOOL_NAME: &str = "vision";
/// Maximum number of ordered images in one invocation.
pub const MAX_VISION_IMAGES: usize = 20;
/// Maximum canonical workspace-relative path size.
pub const MAX_VISION_PATH_BYTES: usize = 4 * 1024;
/// Maximum path components in one canonical workspace-relative path.
pub const MAX_VISION_PATH_COMPONENTS: usize = 256;
/// Maximum UTF-8 bytes in one canonical path component.
pub const MAX_VISION_PATH_COMPONENT_BYTES: usize = 255;
/// Inclusive raw-byte limit for one admitted image.
pub const MAX_VISION_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Inclusive aggregate image-read budget for one invocation.
pub const MAX_VISION_TOTAL_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Inclusive raw-byte limit for one provider batch.
pub const MAX_VISION_BATCH_BYTES: usize = MAX_VISION_BATCH_RAW_BYTES;
/// Maximum serialized [`ToolOutput`] size.
pub const MAX_VISION_SERIALIZED_RESULT_BYTES: usize = 48 * 1024;
/// Default cooperative invocation deadline.
///
/// Capacity and provider futures are raced against this deadline. Synchronous
/// filesystem calls cannot be preempted; cancellation and deadline state are
/// checked between those individually bounded calls.
pub const VISION_DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Default simultaneous invocation bound.
pub const VISION_DEFAULT_MAX_ACTIVE_REQUESTS: usize = 2;
/// Hard simultaneous invocation bound.
pub const VISION_MAX_ACTIVE_REQUESTS: usize = 8;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const READ_CHUNK_BYTES: usize = 64 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SIGNATURE_PROBE_BYTES: usize = 12;
const VISION_DESCRIPTION: &str = "Inspect up to 20 workspace images with a focused question. Accepts exactly one of workspace-relative paths or prior image attachment IDs; attachment history is unavailable in this host slice.";

/// Stable construction-error category for [`VisionTool`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VisionConfigErrorKind {
    /// The current platform cannot provide the descriptor-relative contract.
    UnsupportedPlatform,
    /// The configured workspace root was not absolute.
    InvalidRoot,
    /// The configured workspace root was not an opened real directory.
    InvalidRootType,
    /// The configured workspace root could not be opened or inspected.
    RootUnavailable,
    /// The configured provider target was not in canonical HTTP(S) form.
    InvalidTarget,
    /// A timeout or concurrency limit was invalid.
    InvalidLimits,
}

/// Fixed, path-free vision construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct VisionConfigError {
    kind: VisionConfigErrorKind,
}

impl VisionConfigError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> VisionConfigErrorKind {
        self.kind
    }

    const fn new(kind: VisionConfigErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for VisionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for VisionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            VisionConfigErrorKind::UnsupportedPlatform => {
                "native vision is unsupported on this platform"
            }
            VisionConfigErrorKind::InvalidRoot => "native vision workspace root is invalid",
            VisionConfigErrorKind::InvalidRootType => {
                "native vision workspace root is not a directory"
            }
            VisionConfigErrorKind::RootUnavailable => "native vision workspace root is unavailable",
            VisionConfigErrorKind::InvalidTarget => "native vision provider target is invalid",
            VisionConfigErrorKind::InvalidLimits => "native vision limits are invalid",
        })
    }
}

impl Error for VisionConfigError {}

/// Native vision cooperative deadline and simultaneous-invocation bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisionLimits {
    request_timeout: Duration,
    max_active_requests: usize,
}

impl VisionLimits {
    /// Constructs explicit nonzero limits within the hard production ceiling.
    ///
    /// # Errors
    ///
    /// Returns a fixed error when the timeout is zero or over 60 seconds, or
    /// the simultaneous invocation bound is outside `1..=8`.
    pub fn new(
        request_timeout: Duration,
        max_active_requests: usize,
    ) -> Result<Self, VisionConfigError> {
        if request_timeout.is_zero()
            || request_timeout > VISION_DEFAULT_REQUEST_TIMEOUT
            || !(1..=VISION_MAX_ACTIVE_REQUESTS).contains(&max_active_requests)
        {
            return Err(VisionConfigError::new(VisionConfigErrorKind::InvalidLimits));
        }
        Ok(Self {
            request_timeout,
            max_active_requests,
        })
    }

    /// Returns the cooperative invocation deadline duration.
    #[must_use]
    pub const fn request_timeout(self) -> Duration {
        self.request_timeout
    }

    /// Returns the simultaneous invocation bound.
    #[must_use]
    pub const fn max_active_requests(self) -> usize {
        self.max_active_requests
    }
}

impl Default for VisionLimits {
    fn default() -> Self {
        Self {
            request_timeout: VISION_DEFAULT_REQUEST_TIMEOUT,
            max_active_requests: VISION_DEFAULT_MAX_ACTIVE_REQUESTS,
        }
    }
}

/// Native, workspace-confined vision tool with explicitly injected network and
/// deadline authorities.
pub struct VisionTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
    target: NetworkTarget,
    transport: Arc<dyn VisionTransport>,
    deadline: Arc<dyn VisionDeadline>,
    limits: VisionLimits,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    permits: Arc<Semaphore>,
}

impl VisionTool {
    /// Opens and retains an absolute workspace root with production limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed failure for an unsupported platform, invalid root,
    /// malformed target, or unavailable root descriptor.
    pub fn with_transport(
        root: &Path,
        target: NetworkTarget,
        transport: Arc<dyn VisionTransport>,
        deadline: Arc<dyn VisionDeadline>,
    ) -> Result<Self, VisionConfigError> {
        Self::with_bounded_transport(root, target, transport, deadline, VisionLimits::default())
    }

    /// Opens and retains an absolute workspace root with explicit limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed failure for an unsupported platform, invalid root,
    /// malformed target, or unavailable root descriptor.
    pub fn with_bounded_transport(
        root: &Path,
        target: NetworkTarget,
        transport: Arc<dyn VisionTransport>,
        deadline: Arc<dyn VisionDeadline>,
        limits: VisionLimits,
    ) -> Result<Self, VisionConfigError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (root, target, transport, deadline, limits);
            Err(VisionConfigError::new(
                VisionConfigErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            validate_target(&target)?;
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(VisionConfigError::new(VisionConfigErrorKind::InvalidRoot));
            }
            let descriptor = rustix::fs::open(
                &lexical_root,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(map_root_open_error)?;
            Self::from_root_descriptor(descriptor, target, transport, deadline, limits)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn from_root_descriptor(
        root: OwnedFd,
        target: NetworkTarget,
        transport: Arc<dyn VisionTransport>,
        deadline: Arc<dyn VisionDeadline>,
        limits: VisionLimits,
    ) -> Result<Self, VisionConfigError> {
        validate_target(&target)?;
        let metadata = rustix::fs::fstat(&root)
            .map_err(|_| VisionConfigError::new(VisionConfigErrorKind::RootUnavailable))?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(VisionConfigError::new(
                VisionConfigErrorKind::InvalidRootType,
            ));
        }
        Ok(Self {
            root,
            target,
            transport,
            deadline,
            limits,
            permits: Arc::new(Semaphore::new(limits.max_active_requests)),
        })
    }
}

impl fmt::Debug for VisionTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionTool")
            .field("root", &"<redacted>")
            .field("target", &"<redacted>")
            .field("transport", &"<redacted>")
            .field("deadline", &"<redacted>")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct VisionArguments {
    focus: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_ids: Option<Vec<u64>>,
}

enum VisionSources {
    Paths(Vec<String>),
    ImageIds(Vec<u64>),
}

struct CanonicalVisionRequest {
    focus: String,
    sources: VisionSources,
    arguments: Value,
}

impl Tool for VisionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: vision_name(),
            description: VISION_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "focus": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_VISION_FOCUS_BYTES,
                        "description": "Specific question or inspection focus."
                    },
                    "paths": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_VISION_PATH_BYTES
                        },
                        "minItems": 1,
                        "maxItems": MAX_VISION_IMAGES,
                        "uniqueItems": true,
                        "description": "Ordered workspace-relative image paths."
                    },
                    "image_ids": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 1 },
                        "minItems": 1,
                        "maxItems": MAX_VISION_IMAGES,
                        "uniqueItems": true,
                        "description": "Ordered prior image attachment IDs."
                    }
                },
                "required": ["focus"],
                "oneOf": [
                    { "required": ["paths"] },
                    { "required": ["image_ids"] }
                ],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let ToolCall {
            name, arguments, ..
        } = call;
        let arguments = JsonValueOwner::new(arguments);
        if name != vision_name() {
            return Err(invalid_arguments_error());
        }
        let request = canonical_request_owned(&arguments)?;
        match &request.sources {
            VisionSources::Paths(paths) => Ok(PreparedToolCall::new(
                Capability::Vision {
                    paths: paths.clone(),
                    target: self.target.clone(),
                },
                request.arguments,
            )),
            VisionSources::ImageIds(_) => {
                Ok(PreparedToolCall::without_authority(request.arguments))
            }
        }
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let arguments = JsonValueOwner::new(arguments);
        Box::pin(async move {
            let request = canonical_request_owned(&arguments)?;
            drop(arguments);
            check_cancellation_and_deadline(&cancellation, None)?;

            let CanonicalVisionRequest { focus, sources, .. } = request;
            let paths = match sources {
                VisionSources::Paths(paths) => paths,
                VisionSources::ImageIds(image_ids) => {
                    let output = render_attachment_failures(image_ids)?;
                    return publish_output(output, &cancellation, None);
                }
            };

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = (context, paths);
                Err(unsupported_platform_error())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_supported(context, focus, paths, cancellation)
                    .await
            }
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl VisionTool {
    async fn execute_supported(
        &self,
        context: ToolContext,
        focus: String,
        paths: Vec<String>,
        cancellation: CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let deadline = Instant::now()
            .checked_add(self.limits.request_timeout)
            .ok_or_else(timeout_error)?;
        let mut teardown = VisionInvocationTeardown::new(
            cancellation.cancelled(),
            self.deadline.wait_until(deadline),
        );
        let acquired = self
            .acquire_capacity(&cancellation, deadline, &mut teardown)
            .await;
        let result = match acquired {
            Ok(()) => match check_cancellation_and_deadline(&cancellation, Some(deadline)) {
                Ok(()) => {
                    let (cancelled, timeout) = teardown.waiters();
                    self.process_paths(
                        &context,
                        &focus,
                        &paths,
                        &cancellation,
                        deadline,
                        cancelled,
                        timeout,
                    )
                    .await
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        teardown.finish(&cancellation, deadline, result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_paths(
        &self,
        context: &ToolContext,
        focus: &str,
        paths: &[String],
        cancellation: &CancellationToken,
        deadline: Instant,
        mut cancelled: Pin<&mut machine_god_core::Cancelled>,
        mut timeout: Pin<&mut (dyn Future<Output = Result<(), VisionTransportError>> + Send)>,
    ) -> Result<ToolOutput, ToolError> {
        let mut results = BTreeMap::new();
        let mut batch = Vec::with_capacity(MAX_VISION_BATCH_IMAGES);
        let mut batch_bytes = 0_usize;
        let mut processed_bytes = 0_usize;
        let mut read_scratch = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            check_cancellation_and_deadline(cancellation, Some(deadline))?;
            let image_id = u64::try_from(index + 1).expect("at most 20 vision images fit u64");
            let remaining_total = MAX_VISION_TOTAL_IMAGE_BYTES.saturating_sub(processed_bytes);
            if remaining_total == 0 {
                results.insert(image_id, RenderedImage::unavailable(image_id));
                continue;
            }
            let image_limit = remaining_total.min(MAX_VISION_IMAGE_BYTES);
            match self.open_and_probe_image(path, image_limit, cancellation, deadline) {
                Ok(probe) => {
                    processed_bytes = processed_bytes.saturating_add(probe.bytes_read);
                    let probe_bytes = probe.bytes_read;
                    let Some(image) = probe.image else {
                        results.insert(image_id, RenderedImage::unavailable(image_id));
                        continue;
                    };
                    let image_bytes = image.fingerprint.size;
                    let crosses_batch =
                        vision_batch_would_overflow(batch.len(), batch_bytes, image_bytes);
                    if crosses_batch {
                        self.dispatch_batch(
                            std::mem::take(&mut batch),
                            context,
                            focus,
                            cancellation,
                            deadline,
                            cancelled.as_mut(),
                            timeout.as_mut(),
                            &mut results,
                        )
                        .await?;
                        batch_bytes = 0;
                    }
                    let read = match self.finish_image_read(
                        path,
                        image,
                        image_id,
                        cancellation,
                        deadline,
                        &mut read_scratch,
                    ) {
                        Ok(read) => read,
                        Err(LocalImageFailure::Cancelled) => return Err(cancelled_error()),
                        Err(LocalImageFailure::Timeout) => return Err(timeout_error()),
                    };
                    processed_bytes =
                        processed_bytes.saturating_add(read.bytes_read.saturating_sub(probe_bytes));
                    let Some(image) = read.image else {
                        results.insert(image_id, RenderedImage::unavailable(image_id));
                        continue;
                    };
                    batch_bytes = batch_bytes
                        .checked_add(image_bytes)
                        .filter(|bytes| *bytes <= MAX_VISION_BATCH_BYTES)
                        .expect("one admitted image fits an empty vision batch");
                    batch.push(image);
                    if batch.len() == MAX_VISION_BATCH_IMAGES
                        || batch_bytes == MAX_VISION_BATCH_BYTES
                    {
                        self.dispatch_batch(
                            std::mem::take(&mut batch),
                            context,
                            focus,
                            cancellation,
                            deadline,
                            cancelled.as_mut(),
                            timeout.as_mut(),
                            &mut results,
                        )
                        .await?;
                        batch_bytes = 0;
                    }
                }
                Err(LocalImageFailure::Cancelled) => return Err(cancelled_error()),
                Err(LocalImageFailure::Timeout) => return Err(timeout_error()),
            }
        }
        self.finish_execution(
            batch,
            paths.len(),
            context,
            focus,
            cancellation,
            deadline,
            cancelled,
            timeout.as_mut(),
            &mut results,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_execution(
        &self,
        batch: Vec<VisionImage>,
        source_count: usize,
        context: &ToolContext,
        focus: &str,
        cancellation: &CancellationToken,
        deadline: Instant,
        cancelled: Pin<&mut machine_god_core::Cancelled>,
        timeout: Pin<&mut (dyn Future<Output = Result<(), VisionTransportError>> + Send)>,
        results: &mut BTreeMap<u64, RenderedImage>,
    ) -> Result<ToolOutput, ToolError> {
        if !batch.is_empty() {
            self.dispatch_batch(
                batch,
                context,
                focus,
                cancellation,
                deadline,
                cancelled,
                timeout,
                results,
            )
            .await?;
        }

        check_cancellation_and_deadline(cancellation, Some(deadline))?;
        let output = render_results_in_source_order(source_count, results)?;
        check_cancellation_and_deadline(cancellation, Some(deadline))?;
        Ok(output)
    }

    async fn acquire_capacity(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
        teardown: &mut VisionInvocationTeardown<'_>,
    ) -> Result<(), ToolError> {
        check_cancellation_and_deadline(cancellation, Some(deadline))?;
        acquire_vision_capacity(&self.permits, teardown, cancellation, deadline).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn dispatch_batch(
        &self,
        images: Vec<VisionImage>,
        context: &ToolContext,
        focus: &str,
        cancellation: &CancellationToken,
        deadline: Instant,
        cancelled: Pin<&mut machine_god_core::Cancelled>,
        timeout: Pin<&mut (dyn Future<Output = Result<(), VisionTransportError>> + Send)>,
        results: &mut BTreeMap<u64, RenderedImage>,
    ) -> Result<(), ToolError> {
        check_cancellation_and_deadline(cancellation, Some(deadline))?;
        let requested_ids = images.iter().map(VisionImage::image_id).collect::<Vec<_>>();
        let provider_request =
            VisionBatchRequest::new(context.session_id.clone(), focus.to_owned(), images)
                .map_err(map_transport_error)?;
        let response = await_bounded(
            self.transport
                .analyze(provider_request, cancellation.clone()),
            cancellation,
            deadline,
            cancelled,
            timeout,
        )
        .await
        .map_err(map_transport_error)?;

        match response {
            Ok(response) => merge_provider_response(&requested_ids, response, results),
            Err(error)
                if matches!(
                    error.kind(),
                    VisionTransportErrorKind::Cancelled
                        | VisionTransportErrorKind::Timeout
                        | VisionTransportErrorKind::RuntimeRequired
                ) =>
            {
                return Err(map_transport_error(error));
            }
            Err(error) => {
                for image_id in requested_ids {
                    results.insert(image_id, rendered_transport_failure(image_id, error.kind()));
                }
            }
        }
        check_cancellation_and_deadline(cancellation, Some(deadline))
    }

    fn open_and_probe_image(
        &self,
        path: &str,
        max_bytes: usize,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<LocalImageProbe, LocalImageFailure> {
        local_boundary(cancellation, deadline)?;
        let Ok(file) = open_confined_image(&self.root, path) else {
            return Ok(LocalImageProbe::unavailable(0));
        };

        local_boundary(cancellation, deadline)?;
        let Ok(metadata) = rustix::fs::fstat(&file) else {
            return Ok(LocalImageProbe::unavailable(0));
        };
        if !FileType::from_raw_mode(metadata.st_mode).is_file()
            || metadata.st_nlink == 0
            || usize::try_from(metadata.st_size)
                .ok()
                .is_none_or(|size| size > max_bytes)
        {
            return Ok(LocalImageProbe::unavailable(0));
        }
        let Some(fingerprint) = ImageFingerprint::from_stat(&metadata) else {
            return Ok(LocalImageProbe::unavailable(0));
        };
        if !confined_binding_matches(&self.root, &file, path, cancellation, deadline)? {
            return Ok(LocalImageProbe::unavailable(0));
        }
        probe_verified_image(file, fingerprint, cancellation, deadline)
    }

    fn finish_image_read(
        &self,
        path: &str,
        image: ProbedImage,
        image_id: u64,
        cancellation: &CancellationToken,
        deadline: Instant,
        read_scratch: &mut Vec<u8>,
    ) -> Result<LocalImageRead, LocalImageFailure> {
        let bytes_read_before = image.probe_len;
        let read = finish_verified_image(image, image_id, cancellation, deadline, read_scratch)?;
        if read.image.is_some()
            && !confined_binding_matches(&self.root, &read.file, path, cancellation, deadline)?
        {
            return Ok(LocalImageRead::unavailable(read.bytes_read));
        }
        debug_assert!(read.bytes_read >= bytes_read_before);
        Ok(read.without_file())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImageFingerprint {
    device: i128,
    inode: i128,
    links: i128,
    mode: rustix::fs::RawMode,
    size: usize,
    modified_seconds: i128,
    modified_nanoseconds: i128,
    changed_seconds: i128,
    changed_nanoseconds: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ImageFingerprint {
    fn from_stat(metadata: &rustix::fs::Stat) -> Option<Self> {
        Some(Self {
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
            links: i128::from(metadata.st_nlink),
            mode: metadata.st_mode,
            size: usize::try_from(metadata.st_size).ok()?,
            modified_seconds: i128::from(metadata.st_mtime),
            modified_nanoseconds: i128::from(metadata.st_mtime_nsec),
            changed_seconds: i128::from(metadata.st_ctime),
            changed_nanoseconds: i128::from(metadata.st_ctime_nsec),
        })
    }

    fn matches(self, metadata: &rustix::fs::Stat) -> bool {
        metadata.st_nlink != 0 && Self::from_stat(metadata) == Some(self)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct LocalImageRead {
    image: Option<VisionImage>,
    bytes_read: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl LocalImageRead {
    const fn unavailable(bytes_read: usize) -> Self {
        Self {
            image: None,
            bytes_read,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct LocalImageProbe {
    image: Option<ProbedImage>,
    bytes_read: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl LocalImageProbe {
    fn available(image: ProbedImage) -> Self {
        Self {
            bytes_read: image.probe_len,
            image: Some(image),
        }
    }

    const fn unavailable(bytes_read: usize) -> Self {
        Self {
            image: None,
            bytes_read,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ProbedImage {
    file: OwnedFd,
    fingerprint: ImageFingerprint,
    media_type: VisionMediaType,
    probe: [u8; SIGNATURE_PROBE_BYTES],
    probe_len: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct CompletedImageRead {
    file: OwnedFd,
    image: Option<VisionImage>,
    bytes_read: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CompletedImageRead {
    fn available(file: OwnedFd, image: VisionImage, bytes_read: usize) -> Self {
        Self {
            file,
            image: Some(image),
            bytes_read,
        }
    }

    fn unavailable(file: OwnedFd, bytes_read: usize) -> Self {
        Self {
            file,
            image: None,
            bytes_read,
        }
    }

    fn without_file(self) -> LocalImageRead {
        LocalImageRead {
            image: self.image,
            bytes_read: self.bytes_read,
        }
    }
}

#[cfg(target_os = "linux")]
fn open_confined_image(root: &OwnedFd, path: &str) -> rustix::io::Result<OwnedFd> {
    rustix::fs::openat2(
        root,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
}

#[cfg(target_os = "macos")]
fn open_confined_image(root: &OwnedFd, path: &str) -> rustix::io::Result<OwnedFd> {
    let nofollow_any = OFlags::from_bits_retain(libc::O_NOFOLLOW_ANY as _);
    // `O_NOFOLLOW_ANY` asks this one whole-relative-path `openat` syscall to
    // reject a symlink in any component; there is no user-space component walk
    // containing additional uncheckpointed syscalls.
    rustix::fs::openat(
        root,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | nofollow_any,
        Mode::empty(),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn confined_binding_matches(
    root: &OwnedFd,
    file: &OwnedFd,
    path: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<bool, LocalImageFailure> {
    let Some(current_binding) =
        binding_syscall(cancellation, deadline, || open_confined_image(root, path))?
    else {
        return Ok(false);
    };
    let Some(retained_metadata) =
        binding_syscall(cancellation, deadline, || rustix::fs::fstat(file))?
    else {
        return Ok(false);
    };
    let Some(current_metadata) = binding_syscall(cancellation, deadline, || {
        rustix::fs::fstat(&current_binding)
    })?
    else {
        return Ok(false);
    };
    let Some(retained_fingerprint) = ImageFingerprint::from_stat(&retained_metadata) else {
        return Ok(false);
    };
    Ok(retained_fingerprint.matches(&current_metadata))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn binding_syscall<T>(
    cancellation: &CancellationToken,
    deadline: Instant,
    syscall: impl FnOnce() -> rustix::io::Result<T>,
) -> Result<Option<T>, LocalImageFailure> {
    local_boundary(cancellation, deadline)?;
    let result = syscall();
    local_boundary(cancellation, deadline)?;
    Ok(result.ok())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn probe_verified_image(
    file: OwnedFd,
    fingerprint: ImageFingerprint,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<LocalImageProbe, LocalImageFailure> {
    let mut probe = [0_u8; SIGNATURE_PROBE_BYTES];
    let expected = fingerprint.size.min(SIGNATURE_PROBE_BYTES);
    let mut bytes_read = 0_usize;
    while bytes_read < expected {
        local_boundary(cancellation, deadline)?;
        match rustix::io::read(&file, &mut probe[bytes_read..expected]) {
            Ok(0) => return Ok(LocalImageProbe::unavailable(bytes_read)),
            Ok(read) => {
                bytes_read += read;
                local_boundary(cancellation, deadline)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Ok(LocalImageProbe::unavailable(bytes_read)),
        }
    }
    let Some(media_type) = sniff_media_type(&probe[..bytes_read]) else {
        return Ok(LocalImageProbe::unavailable(bytes_read));
    };
    let Ok(current_metadata) = rustix::fs::fstat(&file) else {
        return Ok(LocalImageProbe::unavailable(bytes_read));
    };
    if !fingerprint.matches(&current_metadata) {
        return Ok(LocalImageProbe::unavailable(bytes_read));
    }
    Ok(LocalImageProbe::available(ProbedImage {
        file,
        fingerprint,
        media_type,
        probe,
        probe_len: bytes_read,
    }))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_verified_image(
    image: ProbedImage,
    image_id: u64,
    cancellation: &CancellationToken,
    deadline: Instant,
    read_scratch: &mut Vec<u8>,
) -> Result<CompletedImageRead, LocalImageFailure> {
    let ProbedImage {
        file,
        fingerprint,
        media_type,
        probe,
        probe_len,
    } = image;
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(fingerprint.size).is_err() {
        return Ok(CompletedImageRead::unavailable(file, probe_len));
    }
    bytes.extend_from_slice(&probe[..probe_len]);
    if bytes.len() < fingerprint.size && !prepare_read_scratch(read_scratch) {
        return Ok(CompletedImageRead::unavailable(file, probe_len));
    }
    while bytes.len() < fingerprint.size {
        local_boundary(cancellation, deadline)?;
        let remaining = fingerprint.size - bytes.len();
        match rustix::io::read(&file, &mut read_scratch[..remaining.min(READ_CHUNK_BYTES)]) {
            Ok(0) => return Ok(CompletedImageRead::unavailable(file, bytes.len())),
            Ok(read) => {
                bytes.extend_from_slice(&read_scratch[..read]);
                local_boundary(cancellation, deadline)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Ok(CompletedImageRead::unavailable(file, bytes.len())),
        }
    }

    let mut growth_witness = [0_u8; 1];
    loop {
        local_boundary(cancellation, deadline)?;
        match rustix::io::read(&file, &mut growth_witness) {
            Ok(0) => break,
            Ok(read) => {
                return Ok(CompletedImageRead::unavailable(
                    file,
                    bytes.len().saturating_add(read),
                ));
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Ok(CompletedImageRead::unavailable(file, bytes.len())),
        }
    }

    local_boundary(cancellation, deadline)?;
    let Ok(final_metadata) = rustix::fs::fstat(&file) else {
        return Ok(CompletedImageRead::unavailable(file, bytes.len()));
    };
    if !fingerprint.matches(&final_metadata) {
        return Ok(CompletedImageRead::unavailable(file, bytes.len()));
    }
    let bytes_read = bytes.len();
    let Ok(image) = VisionImage::new(image_id, media_type, bytes) else {
        return Ok(CompletedImageRead::unavailable(file, bytes_read));
    };
    Ok(CompletedImageRead::available(file, image, bytes_read))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepare_read_scratch(read_scratch: &mut Vec<u8>) -> bool {
    if read_scratch.len() == READ_CHUNK_BYTES {
        return true;
    }
    debug_assert!(read_scratch.is_empty());
    if read_scratch.try_reserve_exact(READ_CHUNK_BYTES).is_err() {
        return false;
    }
    read_scratch.resize(READ_CHUNK_BYTES, 0);
    true
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalImageFailure {
    Cancelled,
    Timeout,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn local_boundary(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), LocalImageFailure> {
    // Native reads and metadata checks are synchronous and cannot be detached or
    // forcibly interrupted safely. Each call is size/count bounded, and the
    // invocation cooperatively observes cancellation and its deadline between
    // calls while owned descriptors and permits remain scoped to the future.
    if cancellation.is_cancelled() {
        Err(LocalImageFailure::Cancelled)
    } else if deadline <= Instant::now() {
        Err(LocalImageFailure::Timeout)
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn canonical_request(arguments: Value) -> Result<CanonicalVisionRequest, ToolError> {
    canonical_request_owned(&JsonValueOwner::new(arguments))
}

fn canonical_request_owned(
    arguments: &JsonValueOwner,
) -> Result<CanonicalVisionRequest, ToolError> {
    preflight_arguments(arguments.get())?;
    let object = arguments
        .get()
        .as_object()
        .expect("vision preflight admitted an object");
    let focus = object
        .get("focus")
        .and_then(Value::as_str)
        .expect("vision preflight admitted a string focus")
        .to_owned();
    let paths = object.get("paths").map(|paths| {
        paths
            .as_array()
            .expect("vision preflight admitted a path array")
            .iter()
            .map(|path| {
                path.as_str()
                    .expect("vision preflight admitted string paths")
                    .to_owned()
            })
            .collect::<Vec<_>>()
    });
    let image_ids = object.get("image_ids").map(|image_ids| {
        image_ids
            .as_array()
            .expect("vision preflight admitted an image ID array")
            .iter()
            .map(|image_id| {
                image_id
                    .as_u64()
                    .expect("vision preflight admitted unsigned image IDs")
            })
            .collect::<Vec<_>>()
    });
    let arguments = VisionArguments {
        focus,
        paths,
        image_ids,
    };
    if arguments.focus.len() > MAX_VISION_FOCUS_BYTES
        || arguments
            .focus
            .trim_matches(is_focus_edge_whitespace)
            .is_empty()
        || arguments.focus.contains('\0')
    {
        return Err(invalid_focus_error());
    }

    let (sources, paths, image_ids) = match (arguments.paths, arguments.image_ids) {
        (Some(paths), None) => {
            if paths.is_empty() || paths.len() > MAX_VISION_IMAGES {
                return Err(invalid_sources_error());
            }
            let mut seen = BTreeSet::new();
            let mut canonical_paths = Vec::with_capacity(paths.len());
            for path in paths {
                let path = normalize_relative_path(&path)?;
                if !seen.insert(path.clone()) {
                    return Err(invalid_sources_error());
                }
                canonical_paths.push(path);
            }
            (
                VisionSources::Paths(canonical_paths.clone()),
                Some(canonical_paths),
                None,
            )
        }
        (None, Some(image_ids)) => {
            if image_ids.is_empty() || image_ids.len() > MAX_VISION_IMAGES {
                return Err(invalid_sources_error());
            }
            let mut seen = BTreeSet::new();
            if image_ids
                .iter()
                .any(|image_id| *image_id == 0 || !seen.insert(*image_id))
            {
                return Err(invalid_sources_error());
            }
            (
                VisionSources::ImageIds(image_ids.clone()),
                None,
                Some(image_ids),
            )
        }
        (None, None) | (Some(_), Some(_)) => return Err(invalid_sources_error()),
    };
    let focus = arguments.focus;
    let canonical = serde_json::to_value(VisionArguments {
        focus: focus.clone(),
        paths,
        image_ids,
    })
    .map_err(|_| invalid_arguments_error())?;
    Ok(CanonicalVisionRequest {
        focus,
        sources,
        arguments: canonical,
    })
}

fn preflight_arguments(arguments: &Value) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or_else(invalid_arguments_error)?;
    if object.len() > 3
        || object
            .keys()
            .any(|field| !matches!(field.as_str(), "focus" | "paths" | "image_ids"))
    {
        return Err(invalid_arguments_error());
    }

    let focus = object
        .get("focus")
        .and_then(Value::as_str)
        .ok_or_else(invalid_arguments_error)?;

    let paths = match object.get("paths") {
        Some(Value::Array(paths)) => Some(paths),
        Some(_) => return Err(invalid_arguments_error()),
        None => None,
    };
    let image_ids = match object.get("image_ids") {
        Some(Value::Array(image_ids)) => Some(image_ids),
        Some(_) => return Err(invalid_arguments_error()),
        None => None,
    };

    if paths.is_some_and(|paths| {
        paths.len() <= MAX_VISION_IMAGES && paths.iter().any(|path| !path.is_string())
    }) || image_ids.is_some_and(|image_ids| {
        image_ids.len() <= MAX_VISION_IMAGES
            && image_ids.iter().any(|image_id| image_id.as_u64().is_none())
    }) {
        return Err(invalid_arguments_error());
    }

    if focus.len() > MAX_VISION_FOCUS_BYTES
        || focus.trim_matches(is_focus_edge_whitespace).is_empty()
        || focus.contains('\0')
    {
        return Err(invalid_focus_error());
    }

    match (paths, image_ids) {
        (Some(paths), None) => {
            if !(1..=MAX_VISION_IMAGES).contains(&paths.len()) {
                return Err(invalid_sources_error());
            }
            for path in paths {
                let path = path.as_str().expect("path element type passed preflight");
                if path.len() > MAX_VISION_PATH_BYTES {
                    return Err(invalid_path_error());
                }
            }
            Ok(())
        }
        (None, Some(image_ids)) => {
            if !(1..=MAX_VISION_IMAGES).contains(&image_ids.len()) {
                return Err(invalid_sources_error());
            }
            for image_id in image_ids {
                let image_id = image_id
                    .as_u64()
                    .expect("image ID element type passed preflight");
                if image_id == 0 {
                    return Err(invalid_sources_error());
                }
            }
            Ok(())
        }
        _ => Err(invalid_sources_error()),
    }
}

fn is_focus_edge_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\n')
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_VISION_PATH_BYTES
        || path
            .trim_matches(|character: char| character.is_ascii_whitespace())
            .is_empty()
        || Path::new(path).is_absolute()
        || path == "~"
        || path.starts_with("~/")
        || path.chars().any(is_forbidden_path_character)
    {
        return Err(invalid_path_error());
    }
    let mut component_count = 0_usize;
    for component in path.split('/') {
        component_count += 1;
        if component.is_empty()
            || matches!(component, "." | "..")
            || component_count > MAX_VISION_PATH_COMPONENTS
            || component.len() > MAX_VISION_PATH_COMPONENT_BYTES
        {
            return Err(invalid_path_error());
        }
    }
    Ok(path.to_owned())
}

fn is_forbidden_path_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sniff_media_type(bytes: &[u8]) -> Option<VisionMediaType> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(VisionMediaType::Png)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        Some(VisionMediaType::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(VisionMediaType::Gif)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(VisionMediaType::Webp)
    } else {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn vision_batch_would_overflow(
    image_count: usize,
    current_bytes: usize,
    next_image_bytes: usize,
) -> bool {
    image_count > 0
        && (image_count == MAX_VISION_BATCH_IMAGES
            || current_bytes
                .checked_add(next_image_bytes)
                .is_none_or(|bytes| bytes > MAX_VISION_BATCH_BYTES))
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RenderedImageState {
    Ok {
        summary: String,
        visible_text: Vec<String>,
        details: Vec<String>,
    },
    Failed {
        error: RenderedFailure,
    },
}

#[derive(Serialize)]
struct RenderedImage {
    image_id: u64,
    #[serde(flatten)]
    state: RenderedImageState,
}

impl RenderedImage {
    fn unavailable(image_id: u64) -> Self {
        Self::provider_failure(image_id, VisionProviderFailureCode::ImageUnavailable)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn missing_provider_record(image_id: u64) -> Self {
        Self::provider_failure(image_id, VisionProviderFailureCode::MissingProviderRecord)
    }

    fn output_limit_exceeded(image_id: u64) -> Self {
        Self::provider_failure(image_id, VisionProviderFailureCode::OutputLimitExceeded)
    }

    fn provider_failure(image_id: u64, code: VisionProviderFailureCode) -> Self {
        let error = VisionProviderFailure::new(code);
        Self {
            image_id,
            state: RenderedImageState::Failed {
                error: RenderedFailure {
                    code: error.code().as_str().to_owned(),
                    message: error.message().to_owned(),
                    retryable: error.retryable(),
                    suggestion: error.suggestion().to_owned(),
                },
            },
        }
    }

    fn is_success(&self) -> bool {
        matches!(self.state, RenderedImageState::Ok { .. })
    }
}

#[derive(Serialize)]
struct RenderedFailure {
    code: String,
    message: String,
    retryable: bool,
    suggestion: String,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn merge_provider_response(
    requested_ids: &[u64],
    response: crate::vision_portable::VisionBatchResponse,
    results: &mut BTreeMap<u64, RenderedImage>,
) {
    let requested = requested_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut observed = BTreeMap::new();
    for image in response.into_images() {
        let image_id = image.image_id();
        if !requested.contains(&image_id)
            || observed.insert(image_id, image.into_outcome()).is_some()
        {
            for image_id in requested_ids {
                results.insert(
                    *image_id,
                    RenderedImage::provider_failure(
                        *image_id,
                        VisionProviderFailureCode::ProviderResponseInvalid,
                    ),
                );
            }
            return;
        }
    }

    for image_id in requested_ids {
        let rendered = observed.remove(image_id).map_or_else(
            || RenderedImage::missing_provider_record(*image_id),
            |outcome| rendered_provider_result(*image_id, outcome),
        );
        results.insert(*image_id, rendered);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rendered_provider_result(image_id: u64, outcome: VisionImageOutcome) -> RenderedImage {
    match outcome {
        VisionImageOutcome::Ok {
            summary,
            visible_text,
            details,
        } => RenderedImage {
            image_id,
            state: RenderedImageState::Ok {
                summary,
                visible_text,
                details,
            },
        },
        VisionImageOutcome::Failed { error } => RenderedImage {
            image_id,
            state: RenderedImageState::Failed {
                error: RenderedFailure {
                    code: error.code().as_str().to_owned(),
                    message: error.message().to_owned(),
                    retryable: error.retryable(),
                    suggestion: error.suggestion().to_owned(),
                },
            },
        },
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rendered_transport_failure(image_id: u64, kind: VisionTransportErrorKind) -> RenderedImage {
    match kind {
        VisionTransportErrorKind::ResponseTooLarge => RenderedImage::provider_failure(
            image_id,
            VisionProviderFailureCode::OutputLimitExceeded,
        ),
        VisionTransportErrorKind::InvalidResponse => RenderedImage::provider_failure(
            image_id,
            VisionProviderFailureCode::ProviderResponseInvalid,
        ),
        VisionTransportErrorKind::InvalidRequest
        | VisionTransportErrorKind::Protocol
        | VisionTransportErrorKind::Authentication
        | VisionTransportErrorKind::RateLimited
        | VisionTransportErrorKind::Unavailable
        | VisionTransportErrorKind::Cancelled
        | VisionTransportErrorKind::Timeout
        | VisionTransportErrorKind::RuntimeRequired => {
            RenderedImage::provider_failure(image_id, VisionProviderFailureCode::VisionUnavailable)
        }
    }
}

fn render_attachment_failures(image_ids: Vec<u64>) -> Result<ToolOutput, ToolError> {
    let images = image_ids
        .into_iter()
        .map(RenderedImage::unavailable)
        .collect::<Vec<_>>();
    render_ordered(images)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn render_results_in_source_order(
    image_count: usize,
    results: &mut BTreeMap<u64, RenderedImage>,
) -> Result<ToolOutput, ToolError> {
    let images = (1..=image_count)
        .map(|index| {
            let image_id = u64::try_from(index).expect("at most 20 vision images fit u64");
            results
                .remove(&image_id)
                .unwrap_or_else(|| RenderedImage::missing_provider_record(image_id))
        })
        .collect::<Vec<_>>();
    render_ordered(images)
}

fn render_ordered(mut images: Vec<RenderedImage>) -> Result<ToolOutput, ToolError> {
    loop {
        let successes = images.iter().filter(|image| image.is_success()).count();
        let output = ToolOutput {
            content: json!({ "images": &images }),
            is_error: successes == 0,
        };
        if serialized_value_fits(&output, MAX_VISION_SERIALIZED_RESULT_BYTES) {
            return Ok(output);
        }
        let Some(index) = images.iter().rposition(RenderedImage::is_success) else {
            return Err(result_too_large_error());
        };
        let image_id = images[index].image_id;
        images[index] = RenderedImage::output_limit_exceeded(image_id);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
type VisionTimeoutFuture<'a> = dyn Future<Output = Result<(), VisionTransportError>> + Send + 'a;

#[cfg(any(target_os = "linux", target_os = "macos"))]
type VisionWaiters<'borrow, 'future> = (
    Pin<&'borrow mut machine_god_core::Cancelled>,
    Pin<&'borrow mut VisionTimeoutFuture<'future>>,
);

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct VisionInvocationTeardown<'a> {
    cancellation_wait: Option<machine_god_core::Cancelled>,
    timeout: Option<BoxFuture<'a, Result<(), VisionTransportError>>>,
    acquisition: Option<BoxFuture<'static, Result<OwnedSemaphorePermit, AcquireError>>>,
    permit: Option<OwnedSemaphorePermit>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a> VisionInvocationTeardown<'a> {
    fn new(
        cancellation_wait: machine_god_core::Cancelled,
        timeout: BoxFuture<'a, Result<(), VisionTransportError>>,
    ) -> Self {
        Self {
            cancellation_wait: Some(cancellation_wait),
            timeout: Some(timeout),
            acquisition: None,
            permit: None,
        }
    }

    fn arm_capacity<F>(&mut self, acquisition: F)
    where
        F: Future<Output = Result<OwnedSemaphorePermit, AcquireError>> + Send + 'static,
    {
        debug_assert!(self.acquisition.is_none());
        self.acquisition = Some(Box::pin(acquisition));
    }

    fn waiters(&mut self) -> VisionWaiters<'_, 'a> {
        let Self {
            cancellation_wait,
            timeout,
            ..
        } = self;
        (
            Pin::new(
                cancellation_wait
                    .as_mut()
                    .expect("vision cancellation wait exists until teardown"),
            ),
            timeout
                .as_mut()
                .expect("vision deadline wait exists until teardown")
                .as_mut(),
        )
    }

    fn set_permit(&mut self, permit: OwnedSemaphorePermit) {
        debug_assert!(self.permit.is_none());
        self.permit = Some(permit);
    }

    fn finish<T>(
        mut self,
        cancellation: &CancellationToken,
        deadline: Instant,
        outcome: Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let timeout = self.timeout.take();
        let timeout_drop = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(timeout)));
        let cancellation_wait = self.cancellation_wait.take();
        let cancellation_wait_drop =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(cancellation_wait)));

        // Teardown-triggered cancellation and an elapsed cooperative deadline
        // arbitrate both successful and failed work while capacity is retained.
        let outcome = match check_cancellation_and_deadline(cancellation, Some(deadline)) {
            Ok(()) => outcome,
            Err(error) => Err(error),
        };

        // A pending Tokio semaphore acquisition may already own a reserved
        // permit after being woken. Destroy it only after both waiters, so its
        // reservation cannot re-enter work while stale waiter state survives.
        let acquisition = self.acquisition.take();
        let acquisition_drop =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(acquisition)));
        let permit = self.permit.take();
        let permit_drop = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(permit)));
        // Releasing capacity may synchronously wake arbitrary waiter code or
        // cross the absolute deadline. Re-adjudicate without retaining capacity
        // while publishing or returning an earlier error.
        let outcome = match check_cancellation_and_deadline(cancellation, Some(deadline)) {
            Ok(()) => outcome,
            Err(error) => Err(error),
        };
        settle_vision_cleanup_panics([
            timeout_drop,
            cancellation_wait_drop,
            acquisition_drop,
            permit_drop,
        ]);
        outcome
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for VisionInvocationTeardown<'_> {
    fn drop(&mut self) {
        let preserve_existing_primary = std::thread::panicking();
        let timeout = self.timeout.take();
        let timeout_drop = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(timeout)));
        let cancellation_wait = self.cancellation_wait.take();
        let cancellation_wait_drop =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(cancellation_wait)));
        let acquisition = self.acquisition.take();
        let acquisition_drop =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(acquisition)));
        let permit = self.permit.take();
        let permit_drop = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(permit)));
        settle_vision_cleanup_panics_preserving(
            preserve_existing_primary,
            [
                timeout_drop,
                cancellation_wait_drop,
                acquisition_drop,
                permit_drop,
            ],
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn settle_vision_cleanup_panics<const N: usize>(cleanups: [std::thread::Result<()>; N]) {
    settle_vision_cleanup_panics_preserving(false, cleanups);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn settle_vision_cleanup_panics_preserving<const N: usize>(
    preserve_existing_primary: bool,
    cleanups: [std::thread::Result<()>; N],
) {
    let mut selected = None;
    for cleanup in cleanups {
        if let Err(payload) = cleanup {
            if preserve_existing_primary || selected.is_some() {
                // Opaque panic-payload destruction is arbitrary work. Forget
                // suppressed payloads so they cannot replace the primary panic.
                std::mem::forget(payload);
            } else {
                selected = Some(payload);
            }
        }
    }
    if let Some(payload) = selected {
        std::panic::resume_unwind(payload);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn await_bounded<F: Future>(
    future: F,
    cancellation: &CancellationToken,
    deadline: Instant,
    mut cancelled: Pin<&mut machine_god_core::Cancelled>,
    mut timeout: Pin<&mut (dyn Future<Output = Result<(), VisionTransportError>> + Send)>,
) -> Result<F::Output, VisionTransportError> {
    let mut future = std::pin::pin!(future);
    poll_fn(|context| {
        if cancelled.as_mut().poll(context).is_ready() || cancellation.is_cancelled() {
            return Poll::Ready(Err(VisionTransportError::new(
                VisionTransportErrorKind::Cancelled,
            )));
        }
        let timeout_result = timeout.as_mut().poll(context);
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(VisionTransportError::new(
                VisionTransportErrorKind::Cancelled,
            )));
        }
        if deadline <= Instant::now() {
            return Poll::Ready(Err(VisionTransportError::new(
                VisionTransportErrorKind::Timeout,
            )));
        }
        match timeout_result {
            Poll::Ready(Ok(())) => {
                return Poll::Ready(Err(VisionTransportError::new(
                    VisionTransportErrorKind::Timeout,
                )));
            }
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => {}
        }
        match future.as_mut().poll(context) {
            Poll::Ready(_) if cancellation.is_cancelled() => Poll::Ready(Err(
                VisionTransportError::new(VisionTransportErrorKind::Cancelled),
            )),
            Poll::Ready(_) if deadline <= Instant::now() => Poll::Ready(Err(
                VisionTransportError::new(VisionTransportErrorKind::Timeout),
            )),
            Poll::Ready(output) => Poll::Ready(Ok(output)),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn acquire_vision_capacity(
    permits: &Arc<Semaphore>,
    teardown: &mut VisionInvocationTeardown<'_>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), ToolError> {
    poll_fn(|context| {
        let cancellation_ready = Pin::new(
            teardown
                .cancellation_wait
                .as_mut()
                .expect("vision cancellation wait exists until teardown"),
        )
        .poll(context)
        .is_ready();
        if cancellation_ready || cancellation.is_cancelled() {
            return Poll::Ready(Err(cancelled_error()));
        }

        let timeout_result = teardown
            .timeout
            .as_mut()
            .expect("vision deadline wait exists until teardown")
            .as_mut()
            .poll(context);
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(cancelled_error()));
        }
        if deadline <= Instant::now() {
            return Poll::Ready(Err(timeout_error()));
        }
        match timeout_result {
            Poll::Ready(Ok(())) => return Poll::Ready(Err(timeout_error())),
            Poll::Ready(Err(error)) => return Poll::Ready(Err(map_transport_error(error))),
            Poll::Pending => {}
        }

        if teardown.acquisition.is_none() {
            match Arc::clone(permits).try_acquire_owned() {
                Ok(permit) => {
                    // Keep the uncontended path allocation-free while
                    // transferring capacity to teardown before post-acquisition
                    // adjudication.
                    teardown.set_permit(permit);
                    return Poll::Ready(check_cancellation_and_deadline(
                        cancellation,
                        Some(deadline),
                    ));
                }
                Err(TryAcquireError::Closed) => {
                    return Poll::Ready(
                        check_cancellation_and_deadline(cancellation, Some(deadline))
                            .and_then(|()| Err(unavailable_error(true))),
                    );
                }
                Err(TryAcquireError::NoPermits) => {
                    // Only a contended acquisition needs stable teardown
                    // ownership: Tokio may reserve capacity in this future
                    // between wake and the invocation's next poll.
                    teardown.arm_capacity(Arc::clone(permits).acquire_owned());
                }
            }
        }

        match teardown
            .acquisition
            .as_mut()
            .expect("vision capacity acquisition is armed before polling")
            .as_mut()
            .poll(context)
        {
            Poll::Ready(Ok(permit)) => {
                // Capacity is teardown-owned before post-poll adjudication, so
                // same-poll cancellation or expiry cannot release it while
                // the cancellation and deadline waiters are still live.
                teardown.set_permit(permit);
                Poll::Ready(check_cancellation_and_deadline(
                    cancellation,
                    Some(deadline),
                ))
            }
            Poll::Ready(Err(_)) => Poll::Ready(
                check_cancellation_and_deadline(cancellation, Some(deadline))
                    .and_then(|()| Err(unavailable_error(true))),
            ),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

fn check_cancellation_and_deadline(
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled_error())
    } else if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
        Err(timeout_error())
    } else {
        Ok(())
    }
}

fn publish_output(
    output: ToolOutput,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<ToolOutput, ToolError> {
    check_cancellation_and_deadline(cancellation, deadline)?;
    Ok(output)
}

fn vision_name() -> ToolName {
    ToolName::new(VISION_TOOL_NAME).expect("vision is a valid tool name")
}

fn validate_target(target: &NetworkTarget) -> Result<(), VisionConfigError> {
    let default_port = matches!(
        (target.scheme.as_str(), target.port),
        ("http", Some(80)) | ("https", Some(443))
    );
    if !matches!(target.scheme.as_str(), "http" | "https")
        || !canonical_network_host(&target.host)
        || target.port == Some(0)
        || default_port
    {
        Err(VisionConfigError::new(VisionConfigErrorKind::InvalidTarget))
    } else {
        Ok(())
    }
}

fn canonical_network_host(host: &str) -> bool {
    if host.is_empty()
        || host.len() > 253
        || !host.is_ascii()
        || host.ends_with('.')
        || host.bytes().any(|byte| {
            byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || matches!(byte, b'/' | b'@' | b'?' | b'#' | b'[' | b']')
        })
    {
        return false;
    }
    match url_ipv4_host(host) {
        UrlIpv4Host::Address(address) => return address.to_string() == host,
        UrlIpv4Host::Invalid => return false,
        UrlIpv4Host::NotIpv4 => {}
    }
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return address.to_string() == host;
    }
    if host.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return false;
    }
    let labels = host.split('.').collect::<Vec<_>>();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UrlIpv4Host {
    NotIpv4,
    Address(std::net::Ipv4Addr),
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UrlIpv4Number {
    NotNumber,
    Value(u64),
    Overflow,
}

/// Classifies the numeric spellings that URL parsers interpret as IPv4 so an
/// authorized textual target cannot silently resolve as a different host.
fn url_ipv4_host(host: &str) -> UrlIpv4Host {
    let final_part = host
        .strip_suffix('.')
        .unwrap_or(host)
        .rsplit('.')
        .next()
        .unwrap_or_default();
    if matches!(parse_url_ipv4_number(final_part), UrlIpv4Number::NotNumber)
        && !final_part.bytes().all(|byte| byte.is_ascii_digit())
    {
        return UrlIpv4Host::NotIpv4;
    }

    let host = host.strip_suffix('.').unwrap_or(host);
    let mut numbers = [0_u64; 4];
    let mut count = 0_usize;
    for part in host.split('.') {
        let Some(slot) = numbers.get_mut(count) else {
            return UrlIpv4Host::Invalid;
        };
        let number = match parse_url_ipv4_number(part) {
            UrlIpv4Number::Value(number) => number,
            UrlIpv4Number::NotNumber | UrlIpv4Number::Overflow => {
                return UrlIpv4Host::Invalid;
            }
        };
        *slot = number;
        count += 1;
    }
    if count == 0
        || numbers[..count.saturating_sub(1)]
            .iter()
            .any(|number| *number > u64::from(u8::MAX))
    {
        return UrlIpv4Host::Invalid;
    }

    let last_limit = 1_u64 << (8 * (5 - count));
    let last = numbers[count - 1];
    if last >= last_limit {
        return UrlIpv4Host::Invalid;
    }
    let mut address = last;
    for (index, number) in numbers[..count - 1].iter().copied().enumerate() {
        address += number << (8 * (3 - index));
    }
    let Ok(address) = u32::try_from(address) else {
        return UrlIpv4Host::Invalid;
    };
    UrlIpv4Host::Address(std::net::Ipv4Addr::from(address))
}

fn parse_url_ipv4_number(part: &str) -> UrlIpv4Number {
    if part.is_empty() {
        return UrlIpv4Number::NotNumber;
    }
    let (digits, radix) =
        if let Some(digits) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            (digits, 16_u64)
        } else if let Some(digits) = part.strip_prefix('0') {
            (digits, 8_u64)
        } else {
            (part, 10_u64)
        };
    if digits.is_empty() {
        return UrlIpv4Number::Value(0);
    }
    let mut number = 0_u64;
    let mut overflowed = false;
    for byte in digits.bytes() {
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'f' => u64::from(byte - b'a' + 10),
            b'A'..=b'F' => u64::from(byte - b'A' + 10),
            _ => return UrlIpv4Number::NotNumber,
        };
        if digit >= radix {
            return UrlIpv4Number::NotNumber;
        }
        if !overflowed {
            if let Some(value) = number
                .checked_mul(radix)
                .and_then(|value| value.checked_add(digit))
            {
                number = value;
            } else {
                overflowed = true;
            }
        }
    }
    if overflowed {
        UrlIpv4Number::Overflow
    } else {
        UrlIpv4Number::Value(number)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> VisionConfigError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        VisionConfigErrorKind::InvalidRootType
    } else {
        VisionConfigErrorKind::RootUnavailable
    };
    VisionConfigError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_transport_error(error: VisionTransportError) -> ToolError {
    match error.kind() {
        VisionTransportErrorKind::Cancelled => cancelled_error(),
        VisionTransportErrorKind::Timeout => timeout_error(),
        VisionTransportErrorKind::RuntimeRequired => ToolError::new(
            ToolErrorKind::Unavailable,
            "vision_runtime_required",
            "vision requires an active timer runtime",
            false,
        ),
        VisionTransportErrorKind::InvalidRequest => invalid_arguments_error(),
        VisionTransportErrorKind::Authentication => ToolError::new(
            ToolErrorKind::PermissionDenied,
            "vision_authentication",
            "vision provider authentication failed",
            false,
        ),
        VisionTransportErrorKind::RateLimited | VisionTransportErrorKind::Unavailable => {
            unavailable_error(true)
        }
        VisionTransportErrorKind::InvalidResponse | VisionTransportErrorKind::Protocol => {
            ToolError::new(
                ToolErrorKind::Execution,
                "vision_provider_response_invalid",
                "vision provider response was invalid",
                false,
            )
        }
        VisionTransportErrorKind::ResponseTooLarge => result_too_large_error(),
    }
}

fn invalid_arguments_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "vision_invalid_arguments",
        "vision arguments are invalid",
        false,
    )
}

fn invalid_focus_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "vision_invalid_focus",
        "vision focus is invalid",
        false,
    )
}

fn invalid_sources_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "vision_invalid_sources",
        "vision requires exactly one nonempty unique image source list",
        false,
    )
}

fn invalid_path_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "vision_invalid_path",
        "vision path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "vision_unsupported_platform",
        "native vision is unsupported on this platform",
        false,
    )
}

fn cancelled_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "vision_cancelled",
        "vision execution was cancelled",
        false,
    )
}

fn timeout_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "vision_timeout",
        "vision request timed out",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable_error(retryable: bool) -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "vision_unavailable",
        "vision provider is unavailable",
        retryable,
    )
}

fn result_too_large_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "vision_result_too_large",
        "vision result exceeded its size limit",
        false,
    )
}

fn serialized_value_fits(value: &(impl Serialize + ?Sized), limit: usize) -> bool {
    let mut writer = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut writer, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(written) = self.written.checked_add(bytes.len()) else {
            return Err(io::Error::other("serialized JSON byte count overflowed"));
        };
        if written > self.limit {
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.written = written;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::cell::Cell;
    use std::future::{self, Future as _};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::Context;
    use std::time::{Duration, Instant};

    use futures_executor::block_on;
    use futures_util::stream;
    use machine_god_core::{
        BoxFuture, CancellationToken, Capability, NetworkTarget, PreparedToolAuthorization,
        ProviderError, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId, ToolContext,
        ToolName, TurnId,
    };
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    use serde_json::{Value, json};

    use crate::vision_portable::{
        VisionBatchRequest, VisionBatchResponse, VisionImageOutcome, VisionImageResult,
        VisionMediaType, VisionTransport, VisionTransportError, VisionTransportErrorKind,
    };
    use crate::{
        AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest,
        AiGatewayVisionTransport,
    };

    use super::{
        ImageFingerprint, LocalImageFailure, MAX_VISION_FOCUS_BYTES, MAX_VISION_PATH_COMPONENTS,
        MAX_VISION_SERIALIZED_RESULT_BYTES, VisionConfigErrorKind, VisionDeadline,
        VisionInvocationTeardown, VisionLimits, VisionTool, acquire_vision_capacity,
        binding_syscall, canonical_request, finish_verified_image, probe_verified_image,
        publish_output, render_attachment_failures, render_ordered, sniff_media_type,
    };
    use tokio::sync::Semaphore;

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TestRoot {
        path: PathBuf,
    }

    impl TestRoot {
        fn new() -> Self {
            let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-vision-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("create isolated vision test root");
            Self { path }
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).expect("remove isolated vision test root");
        }
    }

    struct NeverDeadline;

    impl VisionDeadline for NeverDeadline {
        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            Box::pin(future::pending())
        }
    }

    struct CancelReadyDeadline {
        cancellation: CancellationToken,
    }

    impl VisionDeadline for CancelReadyDeadline {
        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            let cancellation = self.cancellation.clone();
            Box::pin(std::future::poll_fn(move |_| {
                assert!(cancellation.cancel());
                std::task::Poll::Ready(Ok(()))
            }))
        }
    }

    struct ErrorAfterDeadline;

    impl VisionDeadline for ErrorAfterDeadline {
        fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            Box::pin(std::future::poll_fn(move |_| {
                std::thread::sleep(
                    deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5),
                );
                std::task::Poll::Ready(Err(VisionTransportError::new(
                    VisionTransportErrorKind::RuntimeRequired,
                )))
            }))
        }
    }

    struct CancelOnDropDeadline {
        cancellation: CancellationToken,
    }

    impl VisionDeadline for CancelOnDropDeadline {
        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            Box::pin(CancelOnDropFuture {
                cancellation: self.cancellation.clone(),
            })
        }
    }

    struct CancelOnDropFuture {
        cancellation: CancellationToken,
    }

    impl std::future::Future for CancelOnDropFuture {
        type Output = Result<(), VisionTransportError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    impl Drop for CancelOnDropFuture {
        fn drop(&mut self) {
            self.cancellation.cancel();
        }
    }

    struct ObservePermitOnDropDeadline {
        cancellation: CancellationToken,
        permits: Arc<Mutex<Option<Arc<Semaphore>>>>,
    }

    impl VisionDeadline for ObservePermitOnDropDeadline {
        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            Box::pin(ObservePermitOnDropFuture {
                cancellation: self.cancellation.clone(),
                permits: Arc::clone(&self.permits),
            })
        }
    }

    struct ObservePermitOnDropFuture {
        cancellation: CancellationToken,
        permits: Arc<Mutex<Option<Arc<Semaphore>>>>,
    }

    impl std::future::Future for ObservePermitOnDropFuture {
        type Output = Result<(), VisionTransportError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    impl Drop for ObservePermitOnDropFuture {
        fn drop(&mut self) {
            let permits = self
                .permits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(
                permits
                    .as_ref()
                    .expect("test installs the vision semaphore")
                    .available_permits(),
                0,
                "deadline teardown must retain active vision capacity",
            );
            assert!(self.cancellation.cancel());
        }
    }

    struct ObserveCapacityOnDropFuture {
        permits: Arc<Semaphore>,
        drops: Arc<AtomicUsize>,
    }

    impl std::future::Future for ObserveCapacityOnDropFuture {
        type Output = Result<(), VisionTransportError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    impl Drop for ObserveCapacityOnDropFuture {
        fn drop(&mut self) {
            assert_eq!(
                self.permits.available_permits(),
                0,
                "deadline future must be destroyed before acquired vision capacity",
            );
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct TriggeredTimeoutFuture {
        ready: Arc<AtomicBool>,
        _observation: ObserveCapacityOnDropFuture,
    }

    impl std::future::Future for TriggeredTimeoutFuture {
        type Output = Result<(), VisionTransportError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            if self.ready.load(Ordering::SeqCst) {
                std::task::Poll::Ready(Ok(()))
            } else {
                std::task::Poll::Pending
            }
        }
    }

    struct TriggeredInvocationDeadline {
        ready: Arc<AtomicBool>,
        drops: Arc<AtomicUsize>,
        permits: Arc<Mutex<Option<Arc<Semaphore>>>>,
    }

    impl VisionDeadline for TriggeredInvocationDeadline {
        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            let permits = self
                .permits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .cloned()
                .expect("test installs the vision semaphore before execution");
            Box::pin(TriggeredTimeoutFuture {
                ready: Arc::clone(&self.ready),
                _observation: ObserveCapacityOnDropFuture {
                    permits,
                    drops: Arc::clone(&self.drops),
                },
            })
        }
    }

    struct PanicOnDropDeadline;

    impl VisionDeadline for PanicOnDropDeadline {
        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> BoxFuture<'_, Result<(), VisionTransportError>> {
            Box::pin(PanicOnDropFuture)
        }
    }

    struct PanicOnDropFuture;

    impl std::future::Future for PanicOnDropFuture {
        type Output = Result<(), VisionTransportError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    impl Drop for PanicOnDropFuture {
        fn drop(&mut self) {
            panic!("injected vision deadline teardown panic");
        }
    }

    #[derive(Clone, Copy)]
    enum TransportMode {
        Success,
        LargeEvidence,
        CancelThenSuccess,
        InvalidRequest,
        ResponseTooLarge,
        RuntimeRequired,
        Pending,
    }

    struct FakeTransport {
        calls: AtomicUsize,
        batches: Mutex<Vec<Vec<(u64, VisionMediaType, usize)>>>,
        mode: TransportMode,
        mutate_on_first_call: Option<PathBuf>,
    }

    struct DeadlineActivatingGatewayTransport {
        calls: AtomicUsize,
        deadline_ready: Arc<AtomicBool>,
        cancel_on_first_attempt: bool,
    }

    impl AiGatewayTransport for DeadlineActivatingGatewayTransport {
        fn stream(
            &self,
            _request: AiGatewayTransportRequest,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            if call_index == 0 {
                self.deadline_ready.store(true, Ordering::SeqCst);
                if self.cancel_on_first_attempt {
                    assert!(cancellation.cancel());
                }
            }
            let response = semantic_invalid_gateway_vision_response();
            Box::pin(
                async move { Ok(Box::pin(stream::iter([Ok(response)])) as AiGatewayByteStream) },
            )
        }
    }

    impl FakeTransport {
        fn new(mode: TransportMode) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                batches: Mutex::new(Vec::new()),
                mode,
                mutate_on_first_call: None,
            }
        }

        fn mutating(mode: TransportMode, path: PathBuf) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                batches: Mutex::new(Vec::new()),
                mode,
                mutate_on_first_call: Some(path),
            }
        }
    }

    impl VisionTransport for FakeTransport {
        fn analyze(
            &self,
            request: VisionBatchRequest,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<VisionBatchResponse, VisionTransportError>> {
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            if call_index == 0
                && let Some(path) = &self.mutate_on_first_call
            {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .expect("open image mutation target")
                    .write_all(b"x")
                    .expect("mutate image during prior batch dispatch");
            }
            let batch = request
                .images()
                .iter()
                .map(|image| (image.image_id(), image.media_type(), image.bytes().len()))
                .collect::<Vec<_>>();
            self.batches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(batch);
            let large_evidence = matches!(self.mode, TransportMode::LargeEvidence);
            let results = request
                .images()
                .iter()
                .rev()
                .map(|image| {
                    VisionImageResult::new(
                        image.image_id(),
                        VisionImageOutcome::Ok {
                            summary: if large_evidence {
                                "x".repeat(2_400)
                            } else {
                                format!("summary-{}", image.image_id())
                            },
                            visible_text: Vec::new(),
                            details: vec![request.focus().to_owned()],
                        },
                    )
                    .expect("construct fake vision result")
                })
                .collect();
            let response = VisionBatchResponse::new(results);
            let cancel = matches!(self.mode, TransportMode::CancelThenSuccess);
            let invalid_request = matches!(self.mode, TransportMode::InvalidRequest);
            let response_too_large = matches!(self.mode, TransportMode::ResponseTooLarge);
            let runtime_required = matches!(self.mode, TransportMode::RuntimeRequired);
            let pending = matches!(self.mode, TransportMode::Pending);
            Box::pin(async move {
                if pending {
                    future::pending::<()>().await;
                }
                if cancel {
                    assert!(cancellation.cancel());
                }
                if invalid_request {
                    return Err(VisionTransportError::new(
                        VisionTransportErrorKind::InvalidRequest,
                    ));
                }
                if response_too_large {
                    return Err(VisionTransportError::new(
                        VisionTransportErrorKind::ResponseTooLarge,
                    ));
                }
                if runtime_required {
                    return Err(VisionTransportError::new(
                        VisionTransportErrorKind::RuntimeRequired,
                    ));
                }
                response
            })
        }
    }

    fn target() -> NetworkTarget {
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "ai-gateway.vercel.sh".to_owned(),
            port: None,
        }
    }

    fn tool(root: &TestRoot, transport: Arc<FakeTransport>) -> VisionTool {
        VisionTool::with_transport(
            root.path.as_path(),
            target(),
            transport,
            Arc::new(NeverDeadline),
        )
        .expect("construct test vision tool")
    }

    fn call(arguments: Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::new("vision-call").expect("valid test call ID"),
            name: ToolName::new("vision").expect("valid tool name"),
            arguments,
        }
    }

    fn assert_path_rejected_without_effects(
        tool: &VisionTool,
        transport: &FakeTransport,
        path: &str,
    ) {
        let arguments = json!({"focus": "Inspect", "paths": [path]});
        let available_permits = tool.permits.available_permits();
        let prepare_error = tool
            .prepare(call(arguments.clone()))
            .expect_err("forbidden path character must fail preparation");
        let execute_error = block_on(tool.execute(context(), arguments, CancellationToken::new()))
            .expect_err("forbidden path character must fail direct execution");

        assert_eq!(prepare_error.code, "vision_invalid_path");
        assert_eq!(execute_error.code, "vision_invalid_path");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(tool.permits.available_permits(), available_permits);
    }

    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("vision-session").expect("valid session ID"),
            session_incarnation_id: SessionIncarnationId::new("vision-incarnation")
                .expect("valid incarnation ID"),
            turn_id: TurnId::new("vision-turn").expect("valid turn ID"),
            call_id: ToolCallId::new("vision-call").expect("valid test call ID"),
        }
    }

    fn semantic_invalid_gateway_vision_response() -> Vec<u8> {
        let mut response = Vec::new();
        for event in [
            json!({"type": "stream-start", "warnings": []}),
            json!({"type": "text-start", "id": "vision-text"}),
            json!({
                "type": "text-delta",
                "id": "vision-text",
                "delta": r#"{"images":[]}"#,
            }),
            json!({"type": "text-end", "id": "vision-text"}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ] {
            response.extend_from_slice(b"data: ");
            response.extend_from_slice(
                serde_json::to_string(&event)
                    .expect("serialize scripted Gateway event")
                    .as_bytes(),
            );
            response.extend_from_slice(b"\n\n");
        }
        response.extend_from_slice(b"data: [DONE]\n\n");
        response
    }

    fn assert_gateway_semantic_retry_observes_outer_deadline(
        cancel_on_first_attempt: bool,
        expected_code: &'static str,
    ) {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let deadline_ready = Arc::new(AtomicBool::new(false));
        let deadline_drops = Arc::new(AtomicUsize::new(0));
        let observed_permits = Arc::new(Mutex::new(None));
        let transport = Arc::new(DeadlineActivatingGatewayTransport {
            calls: AtomicUsize::new(0),
            deadline_ready: Arc::clone(&deadline_ready),
            cancel_on_first_attempt,
        });
        let worker = AiGatewayVisionTransport::new(
            "test/configured-vision-model",
            Arc::clone(&transport) as Arc<dyn AiGatewayTransport>,
        )
        .expect("construct Gateway vision worker");
        let tool = VisionTool::with_bounded_transport(
            root.path.as_path(),
            target(),
            Arc::new(worker),
            Arc::new(TriggeredInvocationDeadline {
                ready: deadline_ready,
                drops: Arc::clone(&deadline_drops),
                permits: Arc::clone(&observed_permits),
            }),
            VisionLimits::new(Duration::from_secs(60), 1).expect("capacity-one vision limits"),
        )
        .expect("construct composed Gateway vision tool");
        *observed_permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&tool.permits));
        let cancellation = CancellationToken::new();

        let error = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            cancellation.clone(),
        ))
        .expect_err("the outer vision arbiter must stop the semantic retry");

        assert_eq!(error.code, expected_code);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(deadline_drops.load(Ordering::SeqCst), 1);
        assert_eq!(tool.permits.available_permits(), 1);
        assert_eq!(cancellation.is_cancelled(), cancel_on_first_attempt);
    }

    fn deeply_nested_value(depth: usize) -> Value {
        let mut value = Value::Null;
        for _ in 0..depth {
            value = Value::Array(vec![value]);
        }
        value
    }

    fn run_on_small_stack(operation: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .name("vision-small-stack".to_owned())
            .stack_size(64 * 1024)
            .spawn(operation)
            .expect("spawn small-stack vision test")
            .join()
            .expect("small-stack vision operation completes");
    }

    #[test]
    fn raw_json_ownership_is_nonrecursive_on_every_early_boundary() {
        const DEPTH: usize = 32_000;

        run_on_small_stack(|| {
            let root = TestRoot::new();
            let transport = Arc::new(FakeTransport::new(TransportMode::Success));
            let tool = tool(&root, transport);
            let mut wrong_name = call(deeply_nested_value(DEPTH));
            wrong_name.name = ToolName::new("not_vision").expect("valid wrong tool name");
            let error = tool
                .prepare(wrong_name)
                .expect_err("wrong-name preparation rejects raw arguments");
            assert_eq!(error.code, "vision_invalid_arguments");
        });

        run_on_small_stack(|| {
            let root = TestRoot::new();
            let transport = Arc::new(FakeTransport::new(TransportMode::Success));
            let tool = tool(&root, transport);
            let error = block_on(tool.execute(
                context(),
                deeply_nested_value(DEPTH),
                CancellationToken::new(),
            ))
            .expect_err("direct execution preflight rejects nested arguments");
            assert_eq!(error.code, "vision_invalid_arguments");
        });

        run_on_small_stack(|| {
            let root = TestRoot::new();
            let transport = Arc::new(FakeTransport::new(TransportMode::Success));
            let tool = tool(&root, transport);
            let error = tool
                .prepare(call(deeply_nested_value(DEPTH)))
                .expect_err("borrowed preflight rejects nested arguments");
            assert_eq!(error.code, "vision_invalid_arguments");
        });

        run_on_small_stack(|| {
            let root = TestRoot::new();
            let transport = Arc::new(FakeTransport::new(TransportMode::Success));
            let tool = tool(&root, transport);
            drop(tool.execute(
                context(),
                deeply_nested_value(DEPTH),
                CancellationToken::new(),
            ));
        });
    }

    #[test]
    fn canonical_arguments_preserve_valid_paths_and_reject_ambiguous_sources() {
        let canonical = canonical_request(json!({
            "focus": "Inspect this",
            "paths": ["images/one.png"]
        }))
        .expect("canonical vision arguments");
        assert_eq!(
            canonical.arguments,
            json!({"focus": "Inspect this", "paths": ["images/one.png"]})
        );

        for invalid in [
            json!({"focus": "Inspect this"}),
            json!({"focus": "Inspect this", "paths": [], "image_ids": [1]}),
            json!({"focus": "Inspect this", "paths": ["one.png"], "image_ids": [1]}),
            json!({"focus": "Inspect this", "paths": ["one.png", "one.png"]}),
            json!({"focus": "Inspect this", "paths": null, "image_ids": [1]}),
            json!({"focus": "Inspect this", "paths": ["one.png"], "image_ids": null}),
            json!({"focus": "Inspect this", "image_ids": [1, 1]}),
            json!({"focus": "Inspect this", "image_ids": [0]}),
            json!({"focus": "Inspect this", "paths": ["../one.png"]}),
            json!({"focus": "Inspect this", "paths": ["./one.png"]}),
            json!({"focus": "Inspect this", "paths": ["~"]}),
            json!({"focus": "Inspect this", "paths": ["~/one.png"]}),
            json!({"focus": "Inspect this", "paths": [" \t"]}),
            json!({"focus": "Inspect this", "paths": ["images//one.png"]}),
            json!({"focus": "Inspect this", "paths": ["one.png"], "extra": true}),
        ] {
            assert!(canonical_request(invalid).is_err());
        }
    }

    #[test]
    fn malformed_element_error_codes_and_direct_execution_match_preparation() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));

        for (arguments, expected_code) in [
            (
                json!({"focus": "Inspect", "paths": [1]}),
                "vision_invalid_arguments",
            ),
            (
                json!({"focus": "Inspect", "image_ids": ["1"]}),
                "vision_invalid_arguments",
            ),
            (
                json!({"focus": "Inspect", "paths": ["one.png"], "image_ids": ["1"]}),
                "vision_invalid_arguments",
            ),
            (
                json!({"focus": "Inspect", "image_ids": [0]}),
                "vision_invalid_sources",
            ),
            (
                json!({"focus": "Inspect", "paths": ["p".repeat(super::MAX_VISION_PATH_BYTES + 1)]}),
                "vision_invalid_path",
            ),
        ] {
            let prepare_error = tool
                .prepare(call(arguments.clone()))
                .expect_err("malformed arguments must fail preparation");
            let execute_error =
                block_on(tool.execute(context(), arguments, CancellationToken::new()))
                    .expect_err("malformed arguments must fail direct execution");
            assert_eq!(prepare_error.code, expected_code);
            assert_eq!(execute_error.code, expected_code);
        }

        let canonical = json!({"focus": "Inspect", "paths": ["one.png"]});
        let prepared = tool
            .prepare(call(canonical.clone()))
            .expect("prepare exact canonical arguments");
        assert_eq!(prepared.arguments(), &canonical);
        let output = block_on(tool.execute(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ))
        .expect("execute the exact prepared arguments");
        assert!(!output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn control_character_path_rejection_matches_preparation_without_effects() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));

        for path in ["images/fi\tle.png", "images/fi\u{85}le.png"] {
            assert_path_rejected_without_effects(&tool, transport.as_ref(), path);
        }
    }

    #[test]
    fn bidi_format_path_rejection_matches_preparation_without_effects() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));

        for character in [
            '\u{061c}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}', '\u{202a}', '\u{202b}',
            '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            let path = format!("images/fi{character}le.png");
            assert_path_rejected_without_effects(&tool, transport.as_ref(), &path);
        }
    }

    #[test]
    fn focus_and_path_component_limits_are_byte_exact() {
        assert!(
            canonical_request(json!({
                "focus": "x".repeat(MAX_VISION_FOCUS_BYTES),
                "paths": ["one.png"]
            }))
            .is_ok()
        );
        assert!(
            canonical_request(json!({
                "focus": "x".repeat(MAX_VISION_FOCUS_BYTES + 1),
                "paths": ["one.png"]
            }))
            .is_err()
        );
        assert!(canonical_request(json!({"focus": " \t\n", "paths": ["one.png"]})).is_err());
        assert!(canonical_request(json!({"focus": "\u{a0}", "paths": ["one.png"]})).is_ok());
        let maximum_components = vec!["a"; MAX_VISION_PATH_COMPONENTS].join("/");
        let excess_components = vec!["a"; MAX_VISION_PATH_COMPONENTS + 1].join("/");
        assert!(canonical_request(json!({"focus": "x", "paths": [maximum_components]})).is_ok());
        assert!(canonical_request(json!({"focus": "x", "paths": [excess_components]})).is_err());
        assert!(canonical_request(json!({"focus": "x", "paths": ["a".repeat(256)]})).is_err());
    }

    #[test]
    fn oversized_prebuilt_arguments_are_rejected_without_deep_clone_or_deserialization() {
        let hostile_arguments = [
            json!({
                "focus": "x".repeat(MAX_VISION_FOCUS_BYTES + 1),
                "paths": vec!["p".repeat(super::MAX_VISION_PATH_BYTES); super::MAX_VISION_IMAGES]
            }),
            json!({
                "focus": "inspect",
                "paths": vec!["p".repeat(super::MAX_VISION_PATH_BYTES); super::MAX_VISION_IMAGES + 1]
            }),
            json!({
                "focus": "inspect",
                "paths": vec!["p".repeat(super::MAX_VISION_PATH_BYTES + 1); super::MAX_VISION_IMAGES]
            }),
        ];

        for arguments in hostile_arguments {
            let mut arguments = Some(arguments);
            let mut result = None;
            allocation_counter::measure(|| {});
            let allocations = allocation_counter::measure(|| {
                result = Some(canonical_request(
                    arguments.take().expect("prebuilt hostile arguments"),
                ));
            });

            assert!(result.is_some_and(|result| result.is_err()));
            assert!(
                allocations.count_total <= 3,
                "raw argument rejection cloned or deserialized the prebuilt value: {allocations:?}"
            );
        }
    }

    #[test]
    fn vertical_tab_and_form_feed_focus_pass_prepare_and_execute() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));

        for focus in ["\u{b}", "\u{c}"] {
            let arguments = json!({"focus": focus, "image_ids": [1]});
            let prepared = tool
                .prepare(call(arguments.clone()))
                .expect("protocol-nonblank focus must pass preparation");
            assert_eq!(prepared.arguments(), &arguments);
            let output = block_on(tool.execute(context(), arguments, CancellationToken::new()))
                .expect("protocol-nonblank focus must pass execution validation");
            assert!(output.is_error);
        }
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn non_ascii_whitespace_focus_agrees_across_permission_and_execution() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let arguments = json!({"focus": "\u{a0}", "paths": ["one.png"]});
        let prepared = tool
            .prepare(call(arguments.clone()))
            .expect("prepare non-ASCII whitespace focus");
        assert!(matches!(
            prepared.authorization(),
            PreparedToolAuthorization::PermissionRequired(Capability::Vision { .. })
        ));
        let output = block_on(tool.execute(context(), arguments, CancellationToken::new()))
            .expect("execute the same approved focus");
        assert!(!output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn preparation_returns_exact_composite_capability_without_transport() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let prepared = tool
            .prepare(call(json!({
                "focus": "Read labels",
                "paths": ["one.png", "nested/two.jpg"]
            })))
            .expect("prepare vision call");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            prepared.authorization(),
            &PreparedToolAuthorization::PermissionRequired(Capability::Vision {
                paths: vec!["one.png".to_owned(), "nested/two.jpg".to_owned()],
                target: target(),
            })
        );
        assert_eq!(
            prepared.arguments(),
            &json!({"focus": "Read labels", "paths": ["one.png", "nested/two.jpg"]})
        );

        let attachments = tool
            .prepare(call(json!({"focus": "Compare", "image_ids": [9, 3]})))
            .expect("prepare deferred attachment call");
        assert_eq!(
            attachments.authorization(),
            &PreparedToolAuthorization::NoAuthorityRequired
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn attachment_ids_are_ordered_path_free_failures_without_transport() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Compare", "image_ids": [9, 3]}),
            CancellationToken::new(),
        ))
        .expect("render deferred attachment failures");
        assert!(output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(output.content["images"][0]["image_id"], 9);
        assert_eq!(output.content["images"][1]["image_id"], 3);
        assert_eq!(
            output.content["images"][0]["error"]["message"],
            "Vision could not safely load or verify this image."
        );
        assert_eq!(
            output.content["images"][0]["error"]["suggestion"],
            "Explain the local image failure; do not retry the same snapshot unchanged."
        );
        assert!(!format!("{output:?}").contains(root.path.to_string_lossy().as_ref()));
    }

    #[test]
    fn magic_sniffing_is_extension_independent_and_complete() {
        assert_eq!(
            sniff_media_type(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
            Some(VisionMediaType::Png)
        );
        assert_eq!(
            sniff_media_type(&[0xff, 0xd8, 0xff]),
            Some(VisionMediaType::Jpeg)
        );
        assert_eq!(sniff_media_type(b"GIF87a"), Some(VisionMediaType::Gif));
        assert_eq!(sniff_media_type(b"GIF89a"), Some(VisionMediaType::Gif));
        assert_eq!(
            sniff_media_type(b"RIFF....WEBP"),
            Some(VisionMediaType::Webp)
        );
        assert_eq!(sniff_media_type(b"not an image"), None);
    }

    #[test]
    fn invalid_signatures_charge_only_the_fixed_probe() {
        let root = TestRoot::new();
        let path = root.path.join("invalid.bin");
        let file = std::fs::File::create(&path).expect("create invalid image");
        file.set_len(1024 * 1024).expect("size invalid image");
        drop(file);
        let descriptor = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("open invalid image");
        let fingerprint = ImageFingerprint::from_stat(
            &rustix::fs::fstat(&descriptor).expect("inspect invalid image"),
        )
        .expect("fingerprint invalid image");
        let cancellation = CancellationToken::new();
        let mut read = None;
        let allocations = allocation_counter::measure(|| {
            read = Some(probe_verified_image(
                descriptor,
                fingerprint,
                &cancellation,
                Instant::now() + Duration::from_secs(1),
            ));
        });
        let read = read
            .expect("measured invalid image result")
            .expect("read invalid image");
        assert!(read.image.is_none());
        assert_eq!(read.bytes_read, super::SIGNATURE_PROBE_BYTES);
        assert!(
            allocations.bytes_total < 1024,
            "signature rejection allocated for advertised file size: {allocations:?}"
        );
    }

    #[test]
    fn maximum_invalid_signature_set_never_reaches_transport() {
        let root = TestRoot::new();
        let mut paths = Vec::new();
        for index in 0..super::MAX_VISION_IMAGES {
            let name = format!("invalid-{index}.bin");
            let file =
                std::fs::File::create(root.path.join(&name)).expect("create sparse invalid image");
            file.set_len(super::MAX_VISION_IMAGE_BYTES as u64)
                .expect("size sparse invalid image");
            paths.push(name);
        }
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": paths}),
            CancellationToken::new(),
        ))
        .expect("project invalid signature failures");

        assert!(output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        let images = output.content["images"]
            .as_array()
            .expect("ordered image results");
        assert_eq!(images.len(), super::MAX_VISION_IMAGES);
        assert!(images.iter().all(|image| {
            image["status"] == "failed" && image["error"]["code"] == "image_unavailable"
        }));
    }

    #[test]
    fn image_snapshot_growth_after_initial_metadata_is_rejected() {
        let root = TestRoot::new();
        let path = root.path.join("growing.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n").expect("write initial image");
        let descriptor = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("open initial image");
        let fingerprint = ImageFingerprint::from_stat(
            &rustix::fs::fstat(&descriptor).expect("inspect initial image"),
        )
        .expect("fingerprint initial image");
        let probe = probe_verified_image(
            descriptor,
            fingerprint,
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(1),
        )
        .expect("probe initial image")
        .image
        .expect("admit initial image");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open writer")
            .write_all(b"growth")
            .expect("grow image after metadata");
        let mut read_scratch = Vec::new();
        let read = finish_verified_image(
            probe,
            1,
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(1),
            &mut read_scratch,
        )
        .expect("read growing image")
        .without_file();
        assert!(read.image.is_none());
        assert_eq!(read.bytes_read, fingerprint.size + 1);
    }

    #[test]
    fn probe_resident_images_do_not_allocate_the_read_scratch() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("resident.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write probe-resident image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, transport);
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let probe = tool
            .open_and_probe_image(
                "resident.png",
                super::MAX_VISION_IMAGE_BYTES,
                &cancellation,
                deadline,
            )
            .expect("probe resident image")
            .image
            .expect("admit resident image");
        let mut read_scratch = Vec::new();
        let mut read = None;
        let allocations = allocation_counter::measure(|| {
            read = Some(finish_verified_image(
                probe,
                1,
                &cancellation,
                deadline,
                &mut read_scratch,
            ));
        });
        let read = read
            .expect("measured resident read")
            .expect("finish resident image");

        assert!(read.image.is_some());
        assert!(read_scratch.is_empty());
        assert!(
            allocations.bytes_total < 1024,
            "probe-resident image allocated a read scratch: {allocations:?}"
        );
    }

    #[test]
    fn nonresident_images_reuse_one_call_scoped_read_scratch() {
        let root = TestRoot::new();
        let image = [b"\xff\xd8".as_slice(), &[0_u8; 30]].concat();
        std::fs::write(root.path.join("first.jpg"), &image).expect("write first image");
        std::fs::write(root.path.join("second.jpg"), &image).expect("write second image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, transport);
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let first_probe = tool
            .open_and_probe_image(
                "first.jpg",
                super::MAX_VISION_IMAGE_BYTES,
                &cancellation,
                deadline,
            )
            .expect("probe first image")
            .image
            .expect("admit first image");
        let second_probe = tool
            .open_and_probe_image(
                "second.jpg",
                super::MAX_VISION_IMAGE_BYTES,
                &cancellation,
                deadline,
            )
            .expect("probe second image")
            .image
            .expect("admit second image");
        let mut read_scratch = Vec::new();
        let mut first_read = None;
        let first_allocations = allocation_counter::measure(|| {
            first_read = Some(finish_verified_image(
                first_probe,
                1,
                &cancellation,
                deadline,
                &mut read_scratch,
            ));
        });
        assert!(
            first_read
                .expect("measured first read")
                .expect("finish first image")
                .image
                .is_some()
        );
        assert_eq!(read_scratch.len(), super::READ_CHUNK_BYTES);
        assert!(
            first_allocations.bytes_total
                >= u64::try_from(super::READ_CHUNK_BYTES).expect("scratch size fits u64"),
            "first nonresident image did not allocate the scratch: {first_allocations:?}"
        );

        let mut second_read = None;
        let second_allocations = allocation_counter::measure(|| {
            second_read = Some(finish_verified_image(
                second_probe,
                2,
                &cancellation,
                deadline,
                &mut read_scratch,
            ));
        });
        assert!(
            second_read
                .expect("measured second read")
                .expect("finish second image")
                .image
                .is_some()
        );
        assert!(
            second_allocations.bytes_total < 1024,
            "second image allocated another scratch: {second_allocations:?}"
        );
    }

    #[test]
    fn local_failures_merge_with_reordered_provider_success() {
        let root = TestRoot::new();
        std::fs::write(
            root.path.join("actually-image.txt"),
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        )
        .expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({
                "focus": "Read labels",
                "paths": ["missing.png", "actually-image.txt"]
            }),
            CancellationToken::new(),
        ))
        .expect("execute mixed vision call");
        assert!(!output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *transport
                .batches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![vec![(2, VisionMediaType::Png, 8)]]
        );
        assert_eq!(output.content["images"][0]["image_id"], 1);
        assert_eq!(output.content["images"][0]["status"], "failed");
        assert_eq!(
            output.content["images"][0]["error"]["code"],
            "image_unavailable"
        );
        assert_eq!(output.content["images"][1]["image_id"], 2);
        assert_eq!(output.content["images"][1]["status"], "ok");
        assert_eq!(output.content["images"][1]["summary"], "summary-2");
    }

    #[test]
    fn symbolic_links_are_local_failures_and_never_reach_transport() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new();
        std::fs::write(root.path.join("target.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write symlink target image");
        symlink("target.png", root.path.join("linked.png")).expect("create image symlink");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["linked.png"]}),
            CancellationToken::new(),
        ))
        .expect("map symlink to local failure");
        assert!(output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            output.content["images"][0]["error"]["code"],
            "image_unavailable"
        );
    }

    #[test]
    fn relocated_intermediate_directory_cannot_redirect_the_whole_lookup() {
        let root = TestRoot::new();
        let outside = TestRoot::new();
        std::fs::create_dir(root.path.join("nested")).expect("create nested directory");
        let retained_intermediate = rustix::fs::openat(
            rustix::fs::CWD,
            root.path.join("nested"),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("retain intermediate directory like the rejected walker");
        let moved = outside.path.join("moved");
        std::fs::rename(root.path.join("nested"), &moved).expect("relocate intermediate");
        std::fs::write(moved.join("outside.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write outside-only image");

        let escaped = rustix::fs::openat(
            &retained_intermediate,
            "outside.png",
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("the rejected sequential walker would follow the relocated descriptor");
        drop(escaped);

        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        assert!(super::open_confined_image(&tool.root, "nested/outside.png").is_err());
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["nested/outside.png"]}),
            CancellationToken::new(),
        ))
        .expect("project relocated path as a local failure");
        assert!(output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn post_probe_revalidation_rejects_an_intermediate_relocated_outside() {
        let root = TestRoot::new();
        let outside = TestRoot::new();
        std::fs::create_dir(root.path.join("nested")).expect("create nested directory");
        std::fs::write(root.path.join("nested/image.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write nested image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let probe = tool
            .open_and_probe_image(
                "nested/image.png",
                super::MAX_VISION_IMAGE_BYTES,
                &cancellation,
                deadline,
            )
            .expect("probe nested image")
            .image
            .expect("admit nested image");
        std::fs::rename(root.path.join("nested"), outside.path.join("relocated"))
            .expect("relocate intermediate after probe");

        let read = tool
            .finish_image_read(
                "nested/image.png",
                probe,
                1,
                &cancellation,
                deadline,
                &mut Vec::new(),
            )
            .expect("finish relocated image as a local outcome");
        assert!(read.image.is_none());
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn post_read_revalidation_rejects_a_same_name_replacement() {
        let root = TestRoot::new();
        let outside = TestRoot::new();
        std::fs::write(root.path.join("image.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write original image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let probe = tool
            .open_and_probe_image(
                "image.png",
                super::MAX_VISION_IMAGE_BYTES,
                &cancellation,
                deadline,
            )
            .expect("probe original image")
            .image
            .expect("admit original image");
        std::fs::rename(
            root.path.join("image.png"),
            outside.path.join("original.png"),
        )
        .expect("retain the original file under a different binding");
        std::fs::write(root.path.join("image.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write same-name replacement");

        let read = tool
            .finish_image_read(
                "image.png",
                probe,
                1,
                &cancellation,
                deadline,
                &mut Vec::new(),
            )
            .expect("finish replaced image as a local outcome");
        assert!(read.image.is_none());
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn binding_syscall_propagates_cancellation_after_the_kernel_boundary() {
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let failure = binding_syscall(&cancellation, deadline, || {
            assert!(cancellation.cancel());
            Ok::<(), rustix::io::Errno>(())
        })
        .expect_err("post-syscall cancellation must not collapse into image unavailable");
        assert_eq!(failure, LocalImageFailure::Cancelled);
    }

    #[test]
    fn binding_syscall_checks_cancellation_before_entering_the_kernel() {
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());
        let called = Cell::new(false);
        let failure = binding_syscall(
            &cancellation,
            Instant::now() + Duration::from_secs(1),
            || {
                called.set(true);
                Ok::<(), rustix::io::Errno>(())
            },
        )
        .expect_err("pre-syscall cancellation must stop the lookup");
        assert_eq!(failure, LocalImageFailure::Cancelled);
        assert!(!called.get());
    }

    #[test]
    fn binding_syscall_propagates_a_deadline_crossed_in_the_kernel_call() {
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_millis(1);
        let failure = binding_syscall(&cancellation, deadline, || {
            std::thread::sleep(Duration::from_millis(10));
            Ok::<(), rustix::io::Errno>(())
        })
        .expect_err("post-syscall deadline must not collapse into image unavailable");
        assert_eq!(failure, LocalImageFailure::Timeout);
    }

    #[test]
    fn batches_are_sequentially_bounded_by_count_and_raw_bytes() {
        let count_root = TestRoot::new();
        let mut paths = Vec::new();
        for index in 0..9 {
            let name = format!("count-{index}.png");
            std::fs::write(count_root.path.join(&name), b"\x89PNG\r\n\x1a\n")
                .expect("write count-batch image");
            paths.push(name);
        }
        let count_transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let count_tool = tool(&count_root, Arc::clone(&count_transport));
        let output = block_on(count_tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": paths}),
            CancellationToken::new(),
        ))
        .expect("execute count-batched call");
        assert!(!output.is_error);
        let batches = count_transport
            .batches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 8);
        assert_eq!(batches[1].len(), 1);
        drop(batches);

        let byte_root = TestRoot::new();
        let first_size = super::MAX_VISION_BATCH_BYTES / 2 + 1;
        let second_size = super::MAX_VISION_BATCH_BYTES / 2;
        let mut first = vec![0_u8; first_size];
        let mut second = vec![0_u8; second_size];
        first[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        second[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        std::fs::write(byte_root.path.join("first.png"), first)
            .expect("write first byte-batch image");
        std::fs::write(byte_root.path.join("second.png"), second)
            .expect("write second byte-batch image");
        let byte_transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let byte_tool = tool(&byte_root, Arc::clone(&byte_transport));
        let output = block_on(byte_tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["first.png", "second.png"]}),
            CancellationToken::new(),
        ))
        .expect("execute byte-batched call");
        assert!(!output.is_error);
        let batches = byte_transport
            .batches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], vec![(1, VisionMediaType::Png, first_size)]);
        assert_eq!(batches[1], vec![(2, VisionMediaType::Png, second_size)]);
    }

    #[test]
    fn byte_crossing_dispatches_before_allocating_the_next_full_snapshot() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("first.png"), b"\x89PNG\r\n\x1a\n")
            .expect("write first image");
        let second_path = root.path.join("second.png");
        let mut second = std::fs::File::create(&second_path).expect("create second image");
        second
            .write_all(b"\x89PNG\r\n\x1a\n")
            .expect("write second image signature");
        second
            .set_len(super::MAX_VISION_IMAGE_BYTES as u64)
            .expect("size second image");
        drop(second);

        let transport = Arc::new(FakeTransport::mutating(TransportMode::Success, second_path));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["first.png", "second.png"]}),
            CancellationToken::new(),
        ))
        .expect("execute byte-crossing call");

        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.content["images"][0]["status"], "ok");
        assert_eq!(output.content["images"][1]["status"], "failed");
        assert_eq!(
            output.content["images"][1]["error"]["code"],
            "image_unavailable"
        );
    }

    #[test]
    fn legal_multi_batch_evidence_projects_a_bounded_ordered_result() {
        let root = TestRoot::new();
        let mut paths = Vec::new();
        for index in 0..20 {
            let name = format!("evidence-{index}.png");
            std::fs::write(root.path.join(&name), b"\x89PNG\r\n\x1a\n")
                .expect("write evidence image");
            paths.push(name);
        }
        let transport = Arc::new(FakeTransport::new(TransportMode::LargeEvidence));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": paths}),
            CancellationToken::new(),
        ))
        .expect("project legal multi-batch evidence");
        assert!(!output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
        let images = output.content["images"]
            .as_array()
            .expect("ordered projected images");
        assert_eq!(images.len(), 20);
        let first_failure = images
            .iter()
            .position(|image| image["status"] == "failed")
            .expect("combined legal evidence must require projection");
        assert!(
            first_failure > 0,
            "the source-order prefix remains successful"
        );
        assert!(
            images[..first_failure]
                .iter()
                .all(|image| image["status"] == "ok")
        );
        assert!(images[first_failure..].iter().all(|image| {
            image["status"] == "failed" && image["error"]["code"] == "output_limit_exceeded"
        }));
        assert!(super::serialized_value_fits(
            &output,
            MAX_VISION_SERIALIZED_RESULT_BYTES
        ));
    }

    #[test]
    fn execution_future_is_inert_until_polled() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = tool(&root, Arc::clone(&transport));
        let future = tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["created-after-future.png"]}),
            CancellationToken::new(),
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        std::fs::write(
            root.path.join("created-after-future.png"),
            [0xff, 0xd8, 0xff],
        )
        .expect("write image after future construction");
        let output = block_on(future).expect("poll inert vision future");
        assert!(!output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancellation_wins_when_transport_cancels_and_completes_same_poll() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.jpg"), [0xff, 0xd8, 0xff]).expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::CancelThenSuccess));
        let tool = tool(&root, transport);
        let cancellation = CancellationToken::new();
        let error = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.jpg"]}),
            cancellation,
        ))
        .expect_err("same-poll cancellation must reject provider success");
        assert_eq!(error.code, "vision_cancelled");
    }

    #[test]
    fn cancellation_wins_when_deadline_becomes_ready_in_the_same_poll() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let cancellation = CancellationToken::new();
        let tool = VisionTool::with_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(CancelReadyDeadline {
                cancellation: cancellation.clone(),
            }),
        )
        .expect("construct cancelling deadline tool");
        let error = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["never-opened.png"]}),
            cancellation,
        ))
        .expect_err("same-poll cancellation must precede deadline readiness");
        assert_eq!(error.code, "vision_cancelled");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn same_poll_capacity_cancellation_retains_permit_through_waiter_teardown() {
        let cancellation = CancellationToken::new();
        let permits = Arc::new(Semaphore::new(1));
        let deadline_drops = Arc::new(AtomicUsize::new(0));
        let mut teardown = VisionInvocationTeardown::new(
            cancellation.cancelled(),
            Box::pin(ObserveCapacityOnDropFuture {
                permits: Arc::clone(&permits),
                drops: Arc::clone(&deadline_drops),
            }),
        );
        let acquisition_permits = Arc::clone(&permits);
        let acquisition_cancellation = cancellation.clone();
        let acquisition = std::future::poll_fn(move |_| {
            let permit = Arc::clone(&acquisition_permits)
                .try_acquire_owned()
                .expect("acquire sole vision permit");
            assert!(acquisition_cancellation.cancel());
            std::task::Poll::Ready(Ok::<_, tokio::sync::AcquireError>(permit))
        });
        teardown.arm_capacity(acquisition);
        let deadline = Instant::now() + Duration::from_secs(1);

        let error = block_on(acquire_vision_capacity(
            &permits,
            &mut teardown,
            &cancellation,
            deadline,
        ))
        .expect_err("same-poll cancellation must reject acquired capacity");
        assert_eq!(error.code, "vision_cancelled");
        assert_eq!(permits.available_permits(), 0);

        let error = teardown
            .finish(&cancellation, deadline, Err::<(), _>(error))
            .expect_err("teardown retains cancellation outcome");
        assert_eq!(error.code, "vision_cancelled");
        assert_eq!(deadline_drops.load(Ordering::SeqCst), 1);
        assert_eq!(permits.available_permits(), 1);
    }

    #[test]
    fn uncontended_capacity_does_not_arm_heap_acquisition() {
        let root = TestRoot::new();
        let tool = tool(&root, Arc::new(FakeTransport::new(TransportMode::Success)));
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut teardown =
            VisionInvocationTeardown::new(cancellation.cancelled(), Box::pin(future::pending()));

        block_on(tool.acquire_capacity(&cancellation, deadline, &mut teardown))
            .expect("acquire uncontended vision capacity");

        assert!(teardown.acquisition.is_none());
        assert!(teardown.permit.is_some());
        teardown
            .finish(&cancellation, deadline, Ok(()))
            .expect("finish uncontended capacity acquisition");
    }

    #[test]
    fn same_poll_capacity_deadline_retains_permit_through_waiter_teardown() {
        let cancellation = CancellationToken::new();
        let permits = Arc::new(Semaphore::new(1));
        let deadline_drops = Arc::new(AtomicUsize::new(0));
        let mut teardown = VisionInvocationTeardown::new(
            cancellation.cancelled(),
            Box::pin(ObserveCapacityOnDropFuture {
                permits: Arc::clone(&permits),
                drops: Arc::clone(&deadline_drops),
            }),
        );
        let acquisition_permits = Arc::clone(&permits);
        let deadline = Instant::now() + Duration::from_millis(100);
        let acquisition = std::future::poll_fn(move |_| {
            let permit = Arc::clone(&acquisition_permits)
                .try_acquire_owned()
                .expect("acquire sole vision permit");
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5),
            );
            std::task::Poll::Ready(Ok::<_, tokio::sync::AcquireError>(permit))
        });
        teardown.arm_capacity(acquisition);

        let error = block_on(acquire_vision_capacity(
            &permits,
            &mut teardown,
            &cancellation,
            deadline,
        ))
        .expect_err("same-poll deadline must reject acquired capacity");
        assert_eq!(error.code, "vision_timeout");
        assert_eq!(permits.available_permits(), 0);

        let error = teardown
            .finish(&cancellation, deadline, Err::<(), _>(error))
            .expect_err("teardown retains deadline outcome");
        assert_eq!(error.code, "vision_timeout");
        assert_eq!(deadline_drops.load(Ordering::SeqCst), 1);
        assert_eq!(permits.available_permits(), 1);
    }

    #[test]
    fn absolute_deadline_wins_when_deadline_future_crosses_it_then_errors() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = VisionTool::with_bounded_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(ErrorAfterDeadline),
            VisionLimits::new(Duration::from_millis(1), 1).expect("short vision deadline"),
        )
        .expect("construct erroring deadline tool");

        let error = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["never-opened.png"]}),
            CancellationToken::new(),
        ))
        .expect_err("the crossed absolute deadline must override the waiter error");

        assert_eq!(error.code, "vision_timeout");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn gateway_semantic_retry_yields_to_outer_deadline_before_request_two() {
        assert_gateway_semantic_retry_observes_outer_deadline(false, "vision_timeout");
    }

    #[test]
    fn cancellation_wins_when_gateway_retry_also_activates_outer_deadline() {
        assert_gateway_semantic_retry_observes_outer_deadline(true, "vision_cancelled");
    }

    #[test]
    fn cancellation_from_final_deadline_teardown_prevents_success_publication() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let cancellation = CancellationToken::new();
        let observed_permits = Arc::new(Mutex::new(None));
        let tool = VisionTool::with_bounded_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(ObservePermitOnDropDeadline {
                cancellation: cancellation.clone(),
                permits: Arc::clone(&observed_permits),
            }),
            VisionLimits::new(Duration::from_secs(60), 1).expect("capacity-one vision limits"),
        )
        .expect("construct drop-cancelling deadline tool");
        *observed_permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&tool.permits));

        let error = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            cancellation,
        ))
        .expect_err("teardown cancellation must reject completed visual output");

        assert_eq!(error.code, "vision_cancelled");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_pending_execution_tears_down_waiters_before_releasing_capacity() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Pending));
        let cancellation = CancellationToken::new();
        let observed_permits = Arc::new(Mutex::new(None));
        let tool = VisionTool::with_bounded_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(ObservePermitOnDropDeadline {
                cancellation: cancellation.clone(),
                permits: Arc::clone(&observed_permits),
            }),
            VisionLimits::new(Duration::from_secs(60), 1).expect("capacity-one vision limits"),
        )
        .expect("construct pending drop vision tool");
        *observed_permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&tool.permits));

        let mut execution = tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            cancellation.clone(),
        );
        let waker = futures_util::task::noop_waker();
        let mut poll_context = Context::from_waker(&waker);
        assert!(execution.as_mut().poll(&mut poll_context).is_pending());
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(tool.permits.available_permits(), 0);

        drop(execution);

        assert!(cancellation.is_cancelled());
        assert_eq!(tool.permits.available_permits(), 1);
    }

    #[test]
    fn dropping_reserved_capacity_waiter_tears_down_waiters_before_returning_capacity() {
        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let cancellation = CancellationToken::new();
        let observed_permits = Arc::new(Mutex::new(None));
        let tool = VisionTool::with_bounded_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(ObservePermitOnDropDeadline {
                cancellation: cancellation.clone(),
                permits: Arc::clone(&observed_permits),
            }),
            VisionLimits::new(Duration::from_secs(60), 1).expect("capacity-one vision limits"),
        )
        .expect("construct capacity-waiting vision tool");
        *observed_permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&tool.permits));
        let active = block_on(Arc::clone(&tool.permits).acquire_owned())
            .expect("hold sole active vision permit");

        let mut execution = tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["never-opened.png"]}),
            cancellation.clone(),
        );
        let waker = futures_util::task::noop_waker();
        let mut poll_context = Context::from_waker(&waker);
        assert!(execution.as_mut().poll(&mut poll_context).is_pending());
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(tool.permits.available_permits(), 0);

        drop(active);
        assert_eq!(
            tool.permits.available_permits(),
            0,
            "the woken acquisition owns the reserved permit before repoll",
        );
        drop(execution);

        assert!(cancellation.is_cancelled());
        assert_eq!(tool.permits.available_permits(), 1);
    }

    #[test]
    fn cancellation_after_capacity_reservation_retains_it_through_waiter_teardown() {
        let cancellation = CancellationToken::new();
        let permits = Arc::new(Semaphore::new(1));
        let active =
            block_on(Arc::clone(&permits).acquire_owned()).expect("hold sole active vision permit");
        let deadline_drops = Arc::new(AtomicUsize::new(0));
        let mut teardown = VisionInvocationTeardown::new(
            cancellation.cancelled(),
            Box::pin(ObserveCapacityOnDropFuture {
                permits: Arc::clone(&permits),
                drops: Arc::clone(&deadline_drops),
            }),
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut acquisition = Box::pin(acquire_vision_capacity(
            &permits,
            &mut teardown,
            &cancellation,
            deadline,
        ));
        let waker = futures_util::task::noop_waker();
        let mut poll_context = Context::from_waker(&waker);
        assert!(acquisition.as_mut().poll(&mut poll_context).is_pending());

        drop(active);
        assert_eq!(permits.available_permits(), 0);
        assert!(cancellation.cancel());
        let error = block_on(acquisition.as_mut())
            .expect_err("cancellation must win when reserved capacity is repolled");
        assert_eq!(error.code, "vision_cancelled");
        drop(acquisition);
        assert_eq!(permits.available_permits(), 0);

        let error = teardown
            .finish(&cancellation, deadline, Err::<(), _>(error))
            .expect_err("teardown retains cancellation outcome");
        assert_eq!(error.code, "vision_cancelled");
        assert_eq!(deadline_drops.load(Ordering::SeqCst), 1);
        assert_eq!(permits.available_permits(), 1);
    }

    #[test]
    fn timeout_after_capacity_reservation_retains_it_through_waiter_teardown() {
        let cancellation = CancellationToken::new();
        let permits = Arc::new(Semaphore::new(1));
        let active =
            block_on(Arc::clone(&permits).acquire_owned()).expect("hold sole active vision permit");
        let timeout_ready = Arc::new(AtomicBool::new(false));
        let deadline_drops = Arc::new(AtomicUsize::new(0));
        let mut teardown = VisionInvocationTeardown::new(
            cancellation.cancelled(),
            Box::pin(TriggeredTimeoutFuture {
                ready: Arc::clone(&timeout_ready),
                _observation: ObserveCapacityOnDropFuture {
                    permits: Arc::clone(&permits),
                    drops: Arc::clone(&deadline_drops),
                },
            }),
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut acquisition = Box::pin(acquire_vision_capacity(
            &permits,
            &mut teardown,
            &cancellation,
            deadline,
        ));
        let waker = futures_util::task::noop_waker();
        let mut poll_context = Context::from_waker(&waker);
        assert!(acquisition.as_mut().poll(&mut poll_context).is_pending());

        drop(active);
        assert_eq!(permits.available_permits(), 0);
        timeout_ready.store(true, Ordering::SeqCst);
        let error = block_on(acquisition.as_mut())
            .expect_err("timeout must win when reserved capacity is repolled");
        assert_eq!(error.code, "vision_timeout");
        drop(acquisition);
        assert_eq!(permits.available_permits(), 0);

        let error = teardown
            .finish(&cancellation, deadline, Err::<(), _>(error))
            .expect_err("teardown retains timeout outcome");
        assert_eq!(error.code, "vision_timeout");
        assert_eq!(deadline_drops.load(Ordering::SeqCst), 1);
        assert_eq!(permits.available_permits(), 1);
    }

    #[test]
    fn deadline_teardown_cancellation_overrides_fatal_transport_and_capacity_errors() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");

        let transport = Arc::new(FakeTransport::new(TransportMode::RuntimeRequired));
        let fatal_cancellation = CancellationToken::new();
        let fatal_tool = VisionTool::with_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(CancelOnDropDeadline {
                cancellation: fatal_cancellation.clone(),
            }),
        )
        .expect("construct fatal-transport vision tool");
        let fatal = block_on(fatal_tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            fatal_cancellation,
        ))
        .expect_err("deadline teardown cancellation overrides fatal transport");
        assert_eq!(fatal.code, "vision_cancelled");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);

        let capacity_cancellation = CancellationToken::new();
        let capacity_tool = VisionTool::with_transport(
            root.path.as_path(),
            target(),
            Arc::new(FakeTransport::new(TransportMode::Success)),
            Arc::new(CancelOnDropDeadline {
                cancellation: capacity_cancellation.clone(),
            }),
        )
        .expect("construct capacity-error vision tool");
        capacity_tool.permits.close();
        let capacity = block_on(capacity_tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            capacity_cancellation,
        ))
        .expect_err("deadline teardown cancellation overrides capacity error");
        assert_eq!(capacity.code, "vision_cancelled");
    }

    #[test]
    fn capacity_release_reentrant_cancellation_wins_after_waiter_teardown() {
        let cancellation = CancellationToken::new();
        let semaphore = Arc::new(Semaphore::new(1));
        let active = block_on(Arc::clone(&semaphore).acquire_owned())
            .expect("acquire sole active vision permit");
        let mut teardown =
            VisionInvocationTeardown::new(cancellation.cancelled(), Box::pin(future::pending()));
        teardown.set_permit(active);

        let mut waiting = Box::pin(Arc::clone(&semaphore).acquire_owned());
        let reentrant_cancellation = cancellation.clone();
        let (waker, wake_handle) = reentrant_waker(Callback::Wake, move || {
            reentrant_cancellation.cancel();
        });
        let mut poll_context = Context::from_waker(&waker);
        assert!(waiting.as_mut().poll(&mut poll_context).is_pending());

        let error = teardown
            .finish(
                &cancellation,
                Instant::now() + Duration::from_secs(1),
                Ok(()),
            )
            .expect_err("permit-release wake cancellation must reject success");
        assert_eq!(error.code, "vision_cancelled");
        assert!(wake_handle.calls() >= 1);
        drop(waiting);
    }

    #[test]
    fn panicking_deadline_teardown_releases_capacity_before_resuming_unwind() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let tool = VisionTool::with_transport(
            root.path.as_path(),
            target(),
            Arc::clone(&transport) as Arc<dyn VisionTransport>,
            Arc::new(PanicOnDropDeadline),
        )
        .expect("construct panic-on-drop deadline tool");

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = block_on(tool.execute(
                context(),
                json!({"focus": "Inspect", "paths": ["one.png"]}),
                CancellationToken::new(),
            ));
        }));

        assert!(panic.is_err());
        assert_eq!(tool.permits.available_permits(), 2);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn provider_invalid_request_is_an_unavailable_per_image_failure() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::InvalidRequest));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            CancellationToken::new(),
        ))
        .expect("render provider invalid request");
        assert!(output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            output.content["images"][0]["error"]["code"],
            "vision_unavailable"
        );
    }

    #[test]
    fn provider_response_too_large_is_an_output_limit_per_image_failure() {
        let root = TestRoot::new();
        std::fs::write(root.path.join("one.png"), b"\x89PNG\r\n\x1a\n").expect("write test image");
        let transport = Arc::new(FakeTransport::new(TransportMode::ResponseTooLarge));
        let tool = tool(&root, Arc::clone(&transport));
        let output = block_on(tool.execute(
            context(),
            json!({"focus": "Inspect", "paths": ["one.png"]}),
            CancellationToken::new(),
        ))
        .expect("render provider response limit");

        assert!(output.is_error);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.content["images"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            output.content["images"][0]["error"]["code"],
            "output_limit_exceeded"
        );
        assert_eq!(output.content["images"][0]["error"]["retryable"], true);
    }

    #[test]
    fn limits_targets_and_serialized_output_are_bounded() {
        assert!(VisionLimits::new(Duration::ZERO, 1).is_err());
        assert!(VisionLimits::new(Duration::from_secs(60), 8).is_ok());
        assert!(VisionLimits::new(Duration::from_secs(61), 1).is_err());
        assert!(VisionLimits::new(Duration::from_secs(1), 9).is_err());

        let root = TestRoot::new();
        let transport = Arc::new(FakeTransport::new(TransportMode::Success));
        let invalid_target = NetworkTarget {
            scheme: "HTTPS".to_owned(),
            host: "Secret.Example".to_owned(),
            port: None,
        };
        assert_eq!(
            VisionTool::with_transport(
                root.path.as_path(),
                invalid_target,
                transport,
                Arc::new(NeverDeadline),
            )
            .expect_err("reject noncanonical target")
            .kind(),
            VisionConfigErrorKind::InvalidTarget
        );

        let noncanonical_url_ipv4_hosts = [
            "127.1",
            "127.0.1",
            "127.65537",
            "2130706433",
            "127.0.0.01",
            "0177.0.0.1",
            "0300.0000.0002.0001",
            "017700000001",
            "0x7f.0.0.1",
            "0X7f.0.0.1",
            "0x7f.1",
            "0x7f000001",
            "127.0.0.0x",
            "1.0xffffff",
            "1.2.0xffff",
            "1.2.3.0377",
            "0xffffffff",
            "4294967295",
        ];
        let mut hostile_hosts = noncanonical_url_ipv4_hosts
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        hostile_hosts.extend([
            "1.18446744073709551616".to_owned(),
            "1.02000000000000000000000".to_owned(),
            "1.0x10000000000000000".to_owned(),
            format!("1.{}", "9".repeat(63)),
            format!("1.0{}", "7".repeat(62)),
            format!("1.0x{}", "f".repeat(61)),
            format!("1.0X{}", "F".repeat(61)),
        ]);
        for host in hostile_hosts {
            let failure = VisionTool::with_transport(
                root.path.as_path(),
                NetworkTarget {
                    scheme: "https".to_owned(),
                    host,
                    port: None,
                },
                Arc::new(FakeTransport::new(TransportMode::Success)),
                Arc::new(NeverDeadline),
            )
            .expect_err("reject URL-standard IPv4 alias");
            assert_eq!(failure.kind(), VisionConfigErrorKind::InvalidTarget);
        }
        for host in [
            "127.0.0.1".to_owned(),
            format!("search.{}x", "9".repeat(62)),
            format!("search.0x{}g", "f".repeat(60)),
        ] {
            VisionTool::with_transport(
                root.path.as_path(),
                NetworkTarget {
                    scheme: "https".to_owned(),
                    host,
                    port: None,
                },
                Arc::new(FakeTransport::new(TransportMode::Success)),
                Arc::new(NeverDeadline),
            )
            .expect("accept a canonical IP or numeric-looking DNS name");
        }

        let oversized = super::RenderedImage {
            image_id: 1,
            state: super::RenderedImageState::Ok {
                summary: "x".repeat(super::MAX_VISION_SERIALIZED_RESULT_BYTES),
                visible_text: Vec::new(),
                details: Vec::new(),
            },
        };
        let output = render_ordered(vec![oversized]).expect("project oversized tool output");
        assert!(output.is_error);
        assert_eq!(
            output.content["images"][0]["error"]["code"],
            "output_limit_exceeded"
        );
    }

    #[test]
    fn stable_attachment_failure_renderer_is_total_failure() {
        let output = render_attachment_failures(vec![7, 2]).expect("render attachment failures");
        assert!(output.is_error);
        assert_eq!(output.content["images"][0]["image_id"], 7);
        assert_eq!(output.content["images"][1]["image_id"], 2);
    }

    #[test]
    fn attachment_output_rechecks_cancellation_immediately_before_publication() {
        let output = render_attachment_failures(vec![7, 2]).expect("render attachment failures");
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());
        let error = publish_output(output, &cancellation, None)
            .expect_err("cancelled attachment output must not be published");
        assert_eq!(error.code, "vision_cancelled");
    }
}
