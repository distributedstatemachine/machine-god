//! Bounded, permission-gated inspection of workspace images.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::future::{Future, poll_fn};
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, NetworkTarget, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::vision_portable::{
    MAX_VISION_BATCH_IMAGES, MAX_VISION_BATCH_RAW_BYTES, MAX_VISION_FOCUS_BYTES,
    VisionBatchRequest, VisionDeadline, VisionImage, VisionImageOutcome, VisionMediaType,
    VisionProviderFailure, VisionProviderFailureCode, VisionTransport, VisionTransportError,
    VisionTransportErrorKind,
};

#[cfg(unix)]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};

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
/// Inclusive aggregate raw-byte limit for one invocation.
pub const MAX_VISION_TOTAL_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Inclusive raw-byte limit for one provider batch.
pub const MAX_VISION_BATCH_BYTES: usize = MAX_VISION_BATCH_RAW_BYTES;
/// Maximum serialized [`ToolOutput`] size.
pub const MAX_VISION_SERIALIZED_RESULT_BYTES: usize = 48 * 1024;
/// Default absolute invocation timeout, including capacity waiting and reads.
pub const VISION_DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Default simultaneous invocation bound.
pub const VISION_DEFAULT_MAX_ACTIVE_REQUESTS: usize = 2;
/// Hard simultaneous invocation bound.
pub const VISION_MAX_ACTIVE_REQUESTS: usize = 8;

const READ_CHUNK_BYTES: usize = 64 * 1024;
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

/// Native vision timeout and simultaneous-invocation bounds.
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

    /// Returns the absolute invocation timeout.
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
    #[cfg(unix)]
    root: OwnedFd,
    #[cfg(not(unix))]
    _unsupported: std::convert::Infallible,
    target: NetworkTarget,
    transport: Arc<dyn VisionTransport>,
    deadline: Arc<dyn VisionDeadline>,
    limits: VisionLimits,
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
        validate_target(&target)?;

        #[cfg(not(unix))]
        {
            let _ = (root, transport, deadline, limits);
            Err(VisionConfigError::new(
                VisionConfigErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(unix)]
        {
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

    #[cfg(unix)]
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

#[derive(Deserialize, Serialize)]
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
        if call.name != vision_name() {
            return Err(invalid_arguments_error());
        }
        let request = canonical_request(call.arguments)?;
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
        Box::pin(async move {
            let request = canonical_request(arguments.clone())?;
            if arguments != request.arguments {
                return Err(invalid_arguments_error());
            }
            check_cancellation_and_deadline(&cancellation, None)?;

            let CanonicalVisionRequest { focus, sources, .. } = request;
            let paths = match sources {
                VisionSources::Paths(paths) => paths,
                VisionSources::ImageIds(image_ids) => {
                    return render_attachment_failures(image_ids);
                }
            };

            #[cfg(not(unix))]
            {
                let _ = (context, paths);
                Err(unsupported_platform_error())
            }

            #[cfg(unix)]
            {
                self.execute_supported(context, focus, paths, cancellation)
                    .await
            }
        })
    }
}

#[cfg(unix)]
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
        let mut cancelled = cancellation.cancelled();
        let mut timeout = self.deadline.wait_until(deadline);
        let permit = self
            .acquire_capacity(
                &cancellation,
                deadline,
                Pin::new(&mut cancelled),
                timeout.as_mut(),
            )
            .await?;
        check_cancellation_and_deadline(&cancellation, Some(deadline))?;

        let mut results = BTreeMap::new();
        let mut batch = Vec::with_capacity(MAX_VISION_BATCH_IMAGES);
        let mut batch_bytes = 0_usize;
        let mut admitted_bytes = 0_usize;
        for (index, path) in paths.iter().enumerate() {
            check_cancellation_and_deadline(&cancellation, Some(deadline))?;
            let image_id = u64::try_from(index + 1).expect("at most 20 vision images fit u64");
            let remaining_total = MAX_VISION_TOTAL_IMAGE_BYTES - admitted_bytes;
            if remaining_total == 0 {
                results.insert(image_id, RenderedImage::unavailable(image_id));
                continue;
            }
            let image_limit = remaining_total.min(MAX_VISION_IMAGE_BYTES);
            match self.read_image(path, image_id, image_limit, &cancellation, deadline) {
                Ok(image) => {
                    let image_bytes = image.bytes().len();
                    admitted_bytes = admitted_bytes
                        .checked_add(image_bytes)
                        .filter(|total| *total <= MAX_VISION_TOTAL_IMAGE_BYTES)
                        .expect("the image reader enforces the remaining aggregate budget");
                    let crosses_batch =
                        vision_batch_would_overflow(batch.len(), batch_bytes, image_bytes);
                    if crosses_batch {
                        self.dispatch_batch(
                            std::mem::take(&mut batch),
                            &context,
                            &focus,
                            &cancellation,
                            deadline,
                            Pin::new(&mut cancelled),
                            timeout.as_mut(),
                            &mut results,
                        )
                        .await?;
                        batch_bytes = 0;
                    }
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
                            &context,
                            &focus,
                            &cancellation,
                            deadline,
                            Pin::new(&mut cancelled),
                            timeout.as_mut(),
                            &mut results,
                        )
                        .await?;
                        batch_bytes = 0;
                    }
                }
                Err(LocalImageFailure::Cancelled) => return Err(cancelled_error()),
                Err(LocalImageFailure::Timeout) => return Err(timeout_error()),
                Err(LocalImageFailure::Unavailable) => {
                    results.insert(image_id, RenderedImage::unavailable(image_id));
                }
            }
        }
        if !batch.is_empty() {
            self.dispatch_batch(
                std::mem::take(&mut batch),
                &context,
                &focus,
                &cancellation,
                deadline,
                Pin::new(&mut cancelled),
                timeout.as_mut(),
                &mut results,
            )
            .await?;
        }

        check_cancellation_and_deadline(&cancellation, Some(deadline))?;
        let output = render_results_in_source_order(paths.len(), &mut results)?;
        check_cancellation_and_deadline(&cancellation, Some(deadline))?;

        // `batch` has been moved into and released by the final provider call.
        // Release its allocation before invocation capacity on the success path.
        drop(batch);
        drop(permit);
        Ok(output)
    }

    async fn acquire_capacity(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
        cancelled: Pin<&mut machine_god_core::Cancelled>,
        timeout: Pin<&mut (dyn Future<Output = Result<(), VisionTransportError>> + Send)>,
    ) -> Result<OwnedSemaphorePermit, ToolError> {
        check_cancellation_and_deadline(cancellation, Some(deadline))?;
        await_bounded(
            Arc::clone(&self.permits).acquire_owned(),
            cancellation,
            deadline,
            cancelled,
            timeout,
        )
        .await
        .map_err(map_transport_error)?
        .map_err(|_| unavailable_error(true))
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

    fn read_image(
        &self,
        path: &str,
        image_id: u64,
        max_bytes: usize,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<VisionImage, LocalImageFailure> {
        local_boundary(cancellation, deadline)?;
        let mut directory: Option<OwnedFd> = None;
        let mut components = path.split('/').peekable();
        let file = loop {
            local_boundary(cancellation, deadline)?;
            let component = components.next().ok_or(LocalImageFailure::Unavailable)?;
            let directory_fd = directory
                .as_ref()
                .map_or_else(|| self.root.as_fd(), AsFd::as_fd);
            if components.peek().is_some() {
                directory = Some(
                    rustix::fs::openat(
                        directory_fd,
                        component,
                        OFlags::RDONLY
                            | OFlags::DIRECTORY
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC
                            | OFlags::NONBLOCK,
                        Mode::empty(),
                    )
                    .map_err(|_| LocalImageFailure::Unavailable)?,
                );
            } else {
                break rustix::fs::openat(
                    directory_fd,
                    component,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|_| LocalImageFailure::Unavailable)?;
            }
        };

        local_boundary(cancellation, deadline)?;
        let metadata = rustix::fs::fstat(&file).map_err(|_| LocalImageFailure::Unavailable)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file()
            || u64::try_from(metadata.st_size)
                .ok()
                .is_none_or(|size| size > max_bytes as u64)
        {
            return Err(LocalImageFailure::Unavailable);
        }

        let capacity = usize::try_from(metadata.st_size)
            .unwrap_or(0)
            .min(max_bytes);
        let mut bytes = Vec::with_capacity(capacity);
        let mut chunk = vec![0_u8; READ_CHUNK_BYTES].into_boxed_slice();
        loop {
            local_boundary(cancellation, deadline)?;
            let remaining = (max_bytes + 1).saturating_sub(bytes.len());
            if remaining == 0 {
                return Err(LocalImageFailure::Unavailable);
            }
            match rustix::io::read(&file, &mut chunk[..remaining.min(READ_CHUNK_BYTES)]) {
                Ok(0) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&chunk[..read]);
                    local_boundary(cancellation, deadline)?;
                    if bytes.len() > max_bytes {
                        return Err(LocalImageFailure::Unavailable);
                    }
                }
                Err(error) if error == rustix::io::Errno::INTR => {}
                Err(_) => return Err(LocalImageFailure::Unavailable),
            }
        }
        let media_type = sniff_media_type(&bytes).ok_or(LocalImageFailure::Unavailable)?;
        VisionImage::new(image_id, media_type, bytes).map_err(|_| LocalImageFailure::Unavailable)
    }
}

#[derive(Clone, Copy)]
enum LocalImageFailure {
    Cancelled,
    Timeout,
    Unavailable,
}

fn local_boundary(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), LocalImageFailure> {
    if cancellation.is_cancelled() {
        Err(LocalImageFailure::Cancelled)
    } else if deadline <= Instant::now() {
        Err(LocalImageFailure::Timeout)
    } else {
        Ok(())
    }
}

fn canonical_request(arguments: Value) -> Result<CanonicalVisionRequest, ToolError> {
    let arguments: VisionArguments =
        serde_json::from_value(arguments).map_err(|_| invalid_arguments_error())?;
    if arguments.focus.len() > MAX_VISION_FOCUS_BYTES
        || arguments
            .focus
            .trim_matches(|character: char| character.is_ascii_whitespace())
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

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_VISION_PATH_BYTES
        || Path::new(path).is_absolute()
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

    fn missing_provider_record(image_id: u64) -> Self {
        Self::provider_failure(image_id, VisionProviderFailureCode::MissingProviderRecord)
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

fn rendered_transport_failure(image_id: u64, kind: VisionTransportErrorKind) -> RenderedImage {
    match kind {
        VisionTransportErrorKind::ResponseTooLarge => RenderedImage::provider_failure(
            image_id,
            VisionProviderFailureCode::OutputLimitExceeded,
        ),
        VisionTransportErrorKind::InvalidRequest
        | VisionTransportErrorKind::InvalidResponse
        | VisionTransportErrorKind::Protocol => RenderedImage::provider_failure(
            image_id,
            VisionProviderFailureCode::ProviderResponseInvalid,
        ),
        VisionTransportErrorKind::Authentication
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
    render_ordered(&images)
}

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
    render_ordered(&images)
}

fn render_ordered(images: &[RenderedImage]) -> Result<ToolOutput, ToolError> {
    let successes = images.iter().filter(|image| image.is_success()).count();
    let output = ToolOutput {
        content: json!({ "images": images }),
        is_error: successes == 0,
    };
    if !serialized_value_fits(&output, MAX_VISION_SERIALIZED_RESULT_BYTES) {
        return Err(result_too_large_error());
    }
    Ok(output)
}

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
        match timeout.as_mut().poll(context) {
            Poll::Ready(Ok(())) => {
                return Poll::Ready(Err(VisionTransportError::new(
                    VisionTransportErrorKind::Timeout,
                )));
            }
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => {}
        }
        if deadline <= Instant::now() {
            return Poll::Ready(Err(VisionTransportError::new(
                VisionTransportErrorKind::Timeout,
            )));
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
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return address.to_string() == host;
    }
    if host.bytes().any(|byte| byte.is_ascii_uppercase())
        || host
            .split('.')
            .all(|label| label.bytes().all(|byte| byte.is_ascii_digit()))
    {
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

#[cfg(unix)]
fn map_root_open_error(error: rustix::io::Errno) -> VisionConfigError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        VisionConfigErrorKind::InvalidRootType
    } else {
        VisionConfigErrorKind::RootUnavailable
    };
    VisionConfigError::new(kind)
}

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

#[cfg(not(unix))]
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

#[cfg(all(test, unix))]
mod tests {
    use std::future;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use futures_executor::block_on;
    use machine_god_core::{
        BoxFuture, CancellationToken, Capability, NetworkTarget, PreparedToolAuthorization,
        SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId, ToolContext, ToolName, TurnId,
    };
    use serde_json::{Value, json};

    use crate::vision_portable::{
        VisionBatchRequest, VisionBatchResponse, VisionImageOutcome, VisionImageResult,
        VisionMediaType, VisionTransport, VisionTransportError,
    };

    use super::{
        MAX_VISION_FOCUS_BYTES, MAX_VISION_PATH_COMPONENTS, VisionConfigErrorKind, VisionDeadline,
        VisionLimits, VisionTool, canonical_request, render_attachment_failures, render_ordered,
        sniff_media_type,
    };

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

    #[derive(Clone, Copy)]
    enum TransportMode {
        Success,
        CancelThenSuccess,
    }

    struct FakeTransport {
        calls: AtomicUsize,
        batches: Mutex<Vec<Vec<(u64, VisionMediaType, usize)>>>,
        mode: TransportMode,
    }

    impl FakeTransport {
        fn new(mode: TransportMode) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                batches: Mutex::new(Vec::new()),
                mode,
            }
        }
    }

    impl VisionTransport for FakeTransport {
        fn analyze(
            &self,
            request: VisionBatchRequest,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<VisionBatchResponse, VisionTransportError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let batch = request
                .images()
                .iter()
                .map(|image| (image.image_id(), image.media_type(), image.bytes().len()))
                .collect::<Vec<_>>();
            self.batches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(batch);
            let results = request
                .images()
                .iter()
                .rev()
                .map(|image| {
                    VisionImageResult::new(
                        image.image_id(),
                        VisionImageOutcome::Ok {
                            summary: format!("summary-{}", image.image_id()),
                            visible_text: Vec::new(),
                            details: vec![request.focus().to_owned()],
                        },
                    )
                    .expect("construct fake vision result")
                })
                .collect();
            let response = VisionBatchResponse::new(results);
            let cancel = matches!(self.mode, TransportMode::CancelThenSuccess);
            Box::pin(async move {
                if cancel {
                    assert!(cancellation.cancel());
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

    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("vision-session").expect("valid session ID"),
            session_incarnation_id: SessionIncarnationId::new("vision-incarnation")
                .expect("valid incarnation ID"),
            turn_id: TurnId::new("vision-turn").expect("valid turn ID"),
            call_id: ToolCallId::new("vision-call").expect("valid test call ID"),
        }
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
            json!({"focus": "Inspect this", "image_ids": [1, 1]}),
            json!({"focus": "Inspect this", "image_ids": [0]}),
            json!({"focus": "Inspect this", "paths": ["../one.png"]}),
            json!({"focus": "Inspect this", "paths": ["./one.png"]}),
            json!({"focus": "Inspect this", "paths": ["images//one.png"]}),
            json!({"focus": "Inspect this", "paths": ["one.png"], "extra": true}),
        ] {
            assert!(canonical_request(invalid).is_err());
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
        let maximum_components = vec!["a"; MAX_VISION_PATH_COMPONENTS].join("/");
        let excess_components = vec!["a"; MAX_VISION_PATH_COMPONENTS + 1].join("/");
        assert!(canonical_request(json!({"focus": "x", "paths": [maximum_components]})).is_ok());
        assert!(canonical_request(json!({"focus": "x", "paths": [excess_components]})).is_err());
        assert!(canonical_request(json!({"focus": "x", "paths": ["a".repeat(256)]})).is_err());
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
            "Supply an available PNG, JPEG, GIF, or WebP image within the documented limits."
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

        let oversized = super::RenderedImage {
            image_id: 1,
            state: super::RenderedImageState::Ok {
                summary: "x".repeat(super::MAX_VISION_SERIALIZED_RESULT_BYTES),
                visible_text: Vec::new(),
                details: Vec::new(),
            },
        };
        let error = render_ordered(&[oversized]).expect_err("reject oversized tool output");
        assert_eq!(error.code, "vision_result_too_large");
    }

    #[test]
    fn stable_attachment_failure_renderer_is_total_failure() {
        let output = render_attachment_failures(vec![7, 2]).expect("render attachment failures");
        assert!(output.is_error);
        assert_eq!(output.content["images"][0]["image_id"], 7);
        assert_eq!(output.content["images"][1]["image_id"], 2);
    }
}
