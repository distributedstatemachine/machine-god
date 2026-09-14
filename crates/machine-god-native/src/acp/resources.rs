//! Descriptor-confined, bounded instruction snapshots for accepted ACP resources.
//! Resource target files are validated, not injected as user text or executed.

use super::session::{NativeAcpPrompt, NativeAcpResourceOmission, NativeAcpResourceOmissionReason};
use crate::conversation_resource_context::NativeResourcePromptContext;
use crate::{NativeOwnedWorkerScope, NativeWorkspaceScopeSnapshot};
use machine_god_core::{BoxFuture, CancellationToken, Prompt};
use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{FileType, Mode, OFlags},
};
use std::fmt::Write as _;
use std::{
    collections::BTreeSet,
    fmt,
    path::{Component, Path, PathBuf},
};

pub const MAX_ACP_CONTEXT_FILE_BYTES: usize = 16 * 1024;
pub const MAX_ACP_CONTEXT_BYTES: usize = 60 * 1024;
pub const MAX_ACP_CONTEXT_DIRECTORIES: usize = 128;
pub const MAX_ACP_RESOURCE_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpResourceContextError {
    Cancelled,
    WorkerUnavailable,
}
impl fmt::Display for NativeAcpResourceContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ACP resource context is unavailable")
    }
}
impl std::error::Error for NativeAcpResourceContextError {}

/// The text remains canonical user input; context remains provider-only advisory data.
pub struct NativeAcpMaterializedPrompt {
    pub prompt: Prompt,
    pub context: Option<NativeResourcePromptContext>,
    pub omissions: Vec<NativeAcpResourceOmission>,
    pub omitted_records: usize,
}
impl fmt::Debug for NativeAcpMaterializedPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAcpMaterializedPrompt")
            .field("omissions", &self.omissions.len())
            .field("omitted_records", &self.omitted_records)
            .finish_non_exhaustive()
    }
}

pub type NativeAcpResourceContextResult =
    Result<NativeAcpMaterializedPrompt, NativeAcpResourceContextError>;
pub type NativeAcpOwnedResourceContextResult<L> =
    Result<(L, NativeAcpResourceContextResult), NativeAcpResourceContextError>;

/// Explicit retained authority. Construction performs no filesystem I/O.
#[derive(Clone)]
pub struct NativeAcpResourceContextReader {
    scope: NativeWorkspaceScopeSnapshot,
    workers: NativeOwnedWorkerScope,
    #[cfg(test)]
    before_read: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}
impl fmt::Debug for NativeAcpResourceContextReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpResourceContextReader(..)")
    }
}

impl NativeAcpResourceContextReader {
    #[cfg(test)]
    pub(crate) fn with_test_before_read(
        mut self,
        hook: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        self.before_read = Some(hook);
        self
    }
    #[must_use]
    pub const fn new(scope: NativeWorkspaceScopeSnapshot, workers: NativeOwnedWorkerScope) -> Self {
        Self {
            scope,
            workers,
            #[cfg(test)]
            before_read: None,
        }
    }

    /// Materializes a fresh bounded instruction snapshot on the owned worker.
    /// Inert before poll; dropping the future cancels its worker operation.
    #[must_use]
    pub fn materialize(
        &self,
        input: NativeAcpPrompt,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, NativeAcpResourceContextResult> {
        let materialization = self.materialize_with_lease(input, (), cancellation);
        Box::pin(async move { materialization.await?.1 })
    }

    /// Moves the exact FIFO admission lease onto the worker until reads settle.
    /// Dropped response custody cancels work without releasing an active worker's lease.
    #[must_use]
    pub fn materialize_with_lease<L: Send + 'static>(
        &self,
        input: NativeAcpPrompt,
        lease: L,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, NativeAcpOwnedResourceContextResult<L>> {
        let reader = self.clone();
        let guard = CancelOnDrop(Some(cancellation.clone()));
        Box::pin(async move {
            let mut guard = guard;
            check_cancel(&cancellation)?;
            let workers = reader.workers.clone();
            let result = workers
                .run(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        reader.read(input, &cancellation)
                    }))
                    .unwrap_or_else(|payload| {
                        std::mem::forget(payload);
                        Err(NativeAcpResourceContextError::WorkerUnavailable)
                    });
                    (lease, result)
                })
                .await
                .map_err(|_| NativeAcpResourceContextError::WorkerUnavailable)?;
            guard.0.take();
            Ok(result)
        })
    }

    fn read(
        &self,
        mut input: NativeAcpPrompt,
        cancellation: &CancellationToken,
    ) -> Result<NativeAcpMaterializedPrompt, NativeAcpResourceContextError> {
        #[cfg(test)]
        if let Some(hook) = &self.before_read {
            hook();
        }
        check_cancel(cancellation)?;
        let mut snapshot = Snapshot {
            text: String::new(),
            directories: BTreeSet::new(),
        };
        // The explicitly admitted primary workspace contributes default rules.
        let root_instruction = self.scope.primary_identity().join("AGENTS.md");
        if let Ok(route) = self.scope.route(&root_instruction) {
            snapshot.read_directory(
                route.root_descriptor(),
                route.root_identity(),
                &mut input,
                cancellation,
            )?;
        } else {
            omit_path(
                &mut input,
                &root_instruction,
                NativeAcpResourceOmissionReason::UnsafeTarget,
            );
        }
        let targets = std::mem::take(&mut input.resource_targets);
        for target in targets {
            check_cancel(cancellation)?;
            let Ok(route) = self.scope.route(&target) else {
                omit_path(
                    &mut input,
                    &target,
                    NativeAcpResourceOmissionReason::UnsafeTarget,
                );
                continue;
            };
            // Retain each opened ancestor until target validation and context
            // reading finish. Rename/symlink replacement cannot redirect reads.
            let directories = match target_directories(
                route.root_descriptor(),
                route.relative_path(),
                cancellation,
            ) {
                Ok(directories) => directories,
                Err(ReadError::Cancelled) => return Err(NativeAcpResourceContextError::Cancelled),
                Err(error) => {
                    omit_path(&mut input, &target, error.reason());
                    continue;
                }
            };
            snapshot.read_directory(
                route.root_descriptor(),
                route.root_identity(),
                &mut input,
                cancellation,
            )?;
            let mut path = route.root_identity().to_path_buf();
            for (component, directory) in directories {
                path.push(component);
                snapshot.read_directory(&directory, &path, &mut input, cancellation)?;
            }
        }
        check_cancel(cancellation)?;
        if !input.omissions.is_empty() || input.omitted_records != 0 {
            let mut reasons = [0_usize; 4];
            for omission in &input.omissions {
                reasons[match omission.reason {
                    NativeAcpResourceOmissionReason::UnsafeTarget => 0,
                    NativeAcpResourceOmissionReason::TargetLimit => 1,
                    NativeAcpResourceOmissionReason::Unavailable => 2,
                    NativeAcpResourceOmissionReason::ContextLimit => 3,
                }] += 1;
            }
            // Fixed fields reserve <256 bytes beyond the 60-KiB content budget.
            let _ = write!(
                snapshot.text,
                "\nACP resource context omissions: unsafe={}, target_limit={}, unavailable={}, context_limit={}, additional_records={}.\n",
                reasons[0], reasons[1], reasons[2], reasons[3], input.omitted_records
            );
        }
        let context = if snapshot.text.is_empty() {
            None
        } else {
            Some(
                NativeResourcePromptContext::new(snapshot.text)
                    .map_err(|_| NativeAcpResourceContextError::WorkerUnavailable)?,
            )
        };
        check_cancel(cancellation)?;
        Ok(NativeAcpMaterializedPrompt {
            prompt: input.prompt,
            context,
            omissions: input.omissions,
            omitted_records: input.omitted_records,
        })
    }
}

struct Snapshot {
    text: String,
    directories: BTreeSet<(i128, i128)>,
}
impl Snapshot {
    fn read_directory(
        &mut self,
        directory: &OwnedFd,
        path: &Path,
        input: &mut NativeAcpPrompt,
        cancellation: &CancellationToken,
    ) -> Result<(), NativeAcpResourceContextError> {
        check_cancel(cancellation)?;
        let instruction = path.join("AGENTS.md");
        let Ok(stat) = rustix::fs::fstat(directory) else {
            omit_path(
                input,
                &instruction,
                NativeAcpResourceOmissionReason::Unavailable,
            );
            return Ok(());
        };
        let identity = (i128::from(stat.st_dev), i128::from(stat.st_ino));
        if self.directories.contains(&identity) {
            return Ok(());
        }
        if self.directories.len() == MAX_ACP_CONTEXT_DIRECTORIES {
            omit_path(
                input,
                &instruction,
                NativeAcpResourceOmissionReason::ContextLimit,
            );
            return Ok(());
        }
        self.directories.insert(identity);
        match read_instruction(directory, cancellation) {
            Ok(Some(text)) => {
                // JSON escaping keeps path labels from injecting new source lines.
                let label = serde_json::to_string(&instruction.to_string_lossy())
                    .expect("string serialization");
                let prefix = format!("External workspace advisory instructions\nSource: {label}\n");
                let suffix = "\nEnd external workspace advisory instructions\n\n";
                if self.text.len() + prefix.len() + text.len() + suffix.len()
                    > MAX_ACP_CONTEXT_BYTES
                {
                    omit_path(
                        input,
                        &instruction,
                        NativeAcpResourceOmissionReason::ContextLimit,
                    );
                } else {
                    self.text.push_str(&prefix);
                    self.text.push_str(&text);
                    self.text.push_str(suffix);
                }
            }
            Ok(None) => {}
            Err(ReadError::Cancelled) => return Err(NativeAcpResourceContextError::Cancelled),
            Err(error) => omit_path(input, &instruction, error.reason()),
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ReadError {
    Unavailable,
    Limit,
    Cancelled,
}
impl ReadError {
    const fn reason(self) -> NativeAcpResourceOmissionReason {
        match self {
            Self::Limit => NativeAcpResourceOmissionReason::ContextLimit,
            Self::Unavailable | Self::Cancelled => NativeAcpResourceOmissionReason::Unavailable,
        }
    }
}
fn check_read_cancel(cancellation: &CancellationToken) -> Result<(), ReadError> {
    if cancellation.is_cancelled() {
        Err(ReadError::Cancelled)
    } else {
        Ok(())
    }
}
fn target_directories(
    root: &OwnedFd,
    relative: &Path,
    cancellation: &CancellationToken,
) -> Result<Vec<(PathBuf, OwnedFd)>, ReadError> {
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty() || components.len() > MAX_ACP_RESOURCE_DEPTH {
        return Err(ReadError::Limit);
    }
    let mut directories: Vec<(PathBuf, OwnedFd)> = Vec::new();
    for (index, component) in components.iter().enumerate() {
        check_read_cancel(cancellation)?;
        let Component::Normal(name) = component else {
            return Err(ReadError::Unavailable);
        };
        let parent = directories
            .last()
            .map_or(root.as_fd(), |(_, fd)| fd.as_fd());
        let final_component = index + 1 == components.len();
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if !final_component {
            flags |= OFlags::DIRECTORY;
        }
        let descriptor = rustix::fs::openat(parent, *name, flags, Mode::empty())
            .map_err(|_| ReadError::Unavailable)?;
        check_read_cancel(cancellation)?;
        if final_component {
            let stat = rustix::fs::fstat(&descriptor).map_err(|_| ReadError::Unavailable)?;
            if !FileType::from_raw_mode(stat.st_mode).is_file() {
                return Err(ReadError::Unavailable);
            }
        } else {
            directories.push((PathBuf::from(name), descriptor));
        }
    }
    Ok(directories)
}
fn read_instruction(
    directory: &OwnedFd,
    cancellation: &CancellationToken,
) -> Result<Option<String>, ReadError> {
    check_read_cancel(cancellation)?;
    let file = match rustix::fs::openat(
        directory,
        "AGENTS.md",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(ReadError::Unavailable),
    };
    check_read_cancel(cancellation)?;
    let metadata = rustix::fs::fstat(&file).map_err(|_| ReadError::Unavailable)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(ReadError::Unavailable);
    }
    if u64::try_from(metadata.st_size).is_err()
        || u64::try_from(metadata.st_size)
            .is_ok_and(|size| size > MAX_ACP_CONTEXT_FILE_BYTES as u64)
    {
        return Err(ReadError::Limit);
    }
    let mut bytes = vec![0_u8; MAX_ACP_CONTEXT_FILE_BYTES + 1];
    let mut length = 0;
    loop {
        check_read_cancel(cancellation)?;
        match rustix::io::read(&file, &mut bytes[length..]) {
            Ok(0) => break,
            Ok(count) => {
                length += count;
                if length > MAX_ACP_CONTEXT_FILE_BYTES {
                    return Err(ReadError::Limit);
                }
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Err(ReadError::Unavailable),
        }
    }
    check_read_cancel(cancellation)?;
    bytes.truncate(length);
    if bytes.contains(&0) {
        return Err(ReadError::Unavailable);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| ReadError::Unavailable)
}
fn omit_path(input: &mut NativeAcpPrompt, path: &Path, reason: NativeAcpResourceOmissionReason) {
    let source = path.to_string_lossy();
    let mut end = source.len().min(super::session::MAX_ACP_RESOURCE_URI_BYTES);
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    input.omit(&source[..end], reason);
}
fn check_cancel(cancellation: &CancellationToken) -> Result<(), NativeAcpResourceContextError> {
    if cancellation.is_cancelled() {
        Err(NativeAcpResourceContextError::Cancelled)
    } else {
        Ok(())
    }
}
struct CancelOnDrop(Option<CancellationToken>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancellation) = self.0.take() {
            cancellation.cancel();
        }
    }
}

#[cfg(test)]
mod tests;
