//! Explicit owned preparation and one-shot execution binding for file approvals.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, FilesystemAccess, PermissionError,
    PermissionExecutionAdmission, PermissionInvocation, PermissionRequest, PermissionRequestId,
    SessionId, SessionIncarnationId, ToolContext, ToolError, ToolErrorKind, ToolName, TurnId,
};
use rustix::fd::{AsFd, BorrowedFd};
use serde_json::Value;

mod snapshot;
use snapshot::Snapshot;
mod endpoints;
pub(crate) use endpoints::NativeFileEndpoint;
pub(crate) use endpoints::project as project_endpoint_arguments;

enum PreparationRoots {
    Single(NativeFileApprovalAuthority),
    Endpoints(Option<NativeFileEndpoint>, NativeFileEndpoint),
}

/// Complete selected-file approval observations, including the copy source limit.
pub const MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES: usize = 16 * 1024 * 1024;
/// Bound on simultaneously retained preparations/admissions/executions.
pub const MAX_NATIVE_FILE_APPROVALS: usize = 4;
/// Each slot reserves a full preimage plus bounded logical/private arguments,
/// endpoint metadata, core identities, and the write/edit result.
pub const MAX_NATIVE_FILE_APPROVAL_RETAINED_BYTES: usize =
    MAX_NATIVE_FILE_APPROVALS * (MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES + 256 * 1024);

/// The actual concrete operation; never inferred from a displayed string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFileApprovalKind {
    Write,
    Edit,
    Delete,
    Rename,
    Copy,
}

/// Complete selected destination state; absence is distinct from an empty file.
#[derive(Clone, Copy)]
pub enum NativeFileApprovalPreimage<'a> {
    Missing,
    File(&'a [u8]),
    EmptyDirectory,
}
impl fmt::Debug for NativeFileApprovalPreimage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "Missing",
            Self::File(_) => "File { .. }",
            Self::EmptyDirectory => "EmptyDirectory",
        })
    }
}

/// Fixed failures never retain paths, contents, raw OS errors, or policy text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFileApprovalError {
    Invalid,
    Unavailable,
    Changed,
    Denied,
    Cancelled,
    Busy,
    Limit,
    GenerationExhausted,
}
impl fmt::Display for NativeFileApprovalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("file approval failed")
    }
}
impl std::error::Error for NativeFileApprovalError {}
type Error = NativeFileApprovalError;
impl Error {
    pub(crate) fn tool(self) -> ToolError {
        ToolError::new(
            if self == Self::Cancelled {
                ToolErrorKind::Cancelled
            } else {
                ToolErrorKind::PermissionDenied
            },
            "file_approval_failed",
            "file approval failed",
            false,
        )
    }
}

fn permission_error(_: Error) -> PermissionError {
    PermissionError::new("file_approval_failed", "file approval failed")
}

/// Explicit retained directory authority for selected-file preapproval reads.
#[derive(Clone)]
pub struct NativeFileApprovalAuthority {
    root: Arc<File>,
}
impl NativeFileApprovalAuthority {
    pub(crate) fn directory(&self) -> &File {
        &self.root
    }

    /// # Errors
    /// Rejects a descriptor that is not a currently linked directory.
    pub fn from_directory(directory: File) -> Result<Self, Error> {
        let stat = rustix::fs::fstat(&directory).map_err(|_| Error::Unavailable)?;
        if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_dir() || stat.st_nlink == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            root: Arc::new(directory),
        })
    }
}
impl fmt::Debug for NativeFileApprovalAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFileApprovalAuthority")
            .finish_non_exhaustive()
    }
}

/// Bounded synchronous live-policy check; implementations must not perform I/O,
/// wait, acquire an undo lock, or retain a registry lock across this callback.
pub trait NativeFileApprovalPolicy: Send + Sync + 'static {
    /// # Errors
    /// Rejects an expired, revoked, closed, or otherwise obsolete approval.
    fn revalidate(&self) -> Result<(), PermissionError>;
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Identity {
    session: SessionId,
    incarnation: SessionIncarnationId,
    turn: TurnId,
    request: PermissionRequestId,
    call: machine_god_core::ToolCallId,
    name: ToolName,
}
impl Identity {
    fn context_matches(&self, context: &ToolContext, name: &str) -> bool {
        self.session == context.session_id
            && self.incarnation == context.session_incarnation_id
            && self.turn == context.turn_id
            && self.call == context.call_id
            && self.name.as_str() == name
    }
    fn same_execution(&self, other: &Self) -> bool {
        self.session == other.session
            && self.incarnation == other.incarnation
            && self.turn == other.turn
            && self.call == other.call
            && self.name == other.name
    }
}
enum Slot {
    Reserved(u64),
    Ready(Box<Proof>),
}

pub(crate) struct NativeFileApprovalClaim {
    identity: Identity,
    generation: u64,
}
impl Slot {
    fn generation(&self) -> u64 {
        match self {
            Self::Reserved(generation) => *generation,
            Self::Ready(proof) => proof.ticket.generation,
        }
    }
}
struct State {
    next_generation: u64,
    close_generation: Option<u64>,
    slots: BTreeMap<Identity, Slot>,
}

/// Explicit host-owned route from permission confirmation to concrete execution.
pub struct NativeFileApprovalRegistry {
    state: Mutex<State>,
    retained: AtomicUsize,
}
impl Default for NativeFileApprovalRegistry {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for NativeFileApprovalRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFileApprovalRegistry")
            .finish_non_exhaustive()
    }
}
impl NativeFileApprovalRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                next_generation: 1,
                close_generation: Some(0),
                slots: BTreeMap::new(),
            }),
            retained: AtomicUsize::new(0),
        }
    }

    /// Strict argument copying is bounded; reservation and all descriptor work
    /// occur only when this owned future is first polled. No prompt is invoked.
    /// # Errors
    /// Rejects unsupported/mismatched calls, changed evidence, limits, cancellation,
    /// overlapping identical call routes, and exhausted generation allocation.
    #[must_use]
    pub fn prepare(
        self: &Arc<Self>,
        authority: &NativeFileApprovalAuthority,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedFileApproval, Error>> {
        self.prepare_roots(
            PreparationRoots::Single(authority.clone()),
            request,
            invocation,
            cancellation,
        )
    }

    pub(crate) fn prepare_endpoints(
        self: &Arc<Self>,
        source: Option<NativeFileEndpoint>,
        target: NativeFileEndpoint,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedFileApproval, Error>> {
        self.prepare_roots(
            PreparationRoots::Endpoints(source, target),
            request,
            invocation,
            cancellation,
        )
    }

    fn prepare_roots(
        self: &Arc<Self>,
        roots: PreparationRoots,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedFileApproval, Error>> {
        let validated = match &roots {
            PreparationRoots::Single(_) => validate_invocation(request, invocation)
                .map(|kind| (kind, invocation.arguments.clone())),
            PreparationRoots::Endpoints(source, target) => {
                validate_endpoint_invocation(request, invocation, source.as_ref(), target)
            }
        };
        let (kind, private_arguments) = match validated {
            Ok(validated) => validated,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let close_generation = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.close_generation);
        let arguments = invocation.arguments.clone(); // validated flat <=64KiB JSON
        let identity = Identity {
            session: request.session_id.clone(),
            incarnation: request.session_incarnation_id.clone(),
            turn: request.turn_id.clone(),
            request: request.id.clone(),
            call: invocation.call_id.clone(),
            name: invocation.tool_name.clone(),
        };
        let registry = Arc::clone(self);
        let execution_arguments =
            matches!(&roots, PreparationRoots::Endpoints(..)).then(|| private_arguments.clone());
        Box::pin(async move {
            snapshot::check(&cancellation)?;
            let ticket = registry.reserve(identity, close_generation)?;
            let snapshot = match &roots {
                PreparationRoots::Single(authority) => Snapshot::prepare(
                    authority.root.as_fd(),
                    kind,
                    &private_arguments,
                    &cancellation,
                )?,
                PreparationRoots::Endpoints(source, target) => Snapshot::prepare_endpoints(
                    source.as_ref(),
                    target,
                    kind,
                    &private_arguments,
                    &cancellation,
                )?,
            };
            ticket.ensure_reserved()?;
            Ok(PreparedFileApproval {
                proof: Proof {
                    ticket,
                    snapshot,
                    kind,
                    arguments,
                    execution_arguments,
                    policy: None,
                },
            })
        })
    }

    fn reserve(
        self: &Arc<Self>,
        identity: Identity,
        close_generation: Option<u64>,
    ) -> Result<Ticket, Error> {
        self.retained
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_NATIVE_FILE_APPROVALS).then_some(count + 1)
            })
            .map_err(|_| Error::Busy)?;
        let budget = Budget(Arc::downgrade(self));
        let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if close_generation.is_none() || state.close_generation != close_generation {
            return Err(Error::Denied);
        }
        if state
            .slots
            .keys()
            .any(|current| current.same_execution(&identity))
        {
            return Err(Error::Busy);
        }
        let generation = state.next_generation;
        state.next_generation = generation
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        state
            .slots
            .insert(identity.clone(), Slot::Reserved(generation));
        Ok(Ticket {
            registry: Arc::downgrade(self),
            identity,
            generation,
            _budget: budget,
        })
    }

    /// Cancels all unclaimed routes for exactly this historical turn. The host
    /// must call this on turn drop/end, including admitted-but-unpolled calls.
    /// It must separately invalidate its live policy for already claimed calls.
    pub fn close_turn(
        &self,
        session: &SessionId,
        incarnation: &SessionIncarnationId,
        turn: &TurnId,
    ) {
        let removed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.close_generation = state
                .close_generation
                .and_then(|generation| generation.checked_add(1));
            let keys: Vec<_> = state
                .slots
                .keys()
                .filter(|id| {
                    &id.session == session && &id.incarnation == incarnation && &id.turn == turn
                })
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| state.slots.remove(&key))
                .collect::<Vec<_>>()
        };
        drop(removed); // Owned policy/descriptors never drop under registry lock.
    }

    /// Read-only construction stamp, not a claim or an authority transfer. It
    /// prevents an old unpolled execution from consuming a newer request that
    /// happens to reuse the same call ID and arguments.
    pub(crate) fn execution_ticket(
        &self,
        context: &ToolContext,
        name: &str,
    ) -> Result<NativeFileApprovalClaim, Error> {
        let state = self.state.lock().map_err(|_| Error::Unavailable)?;
        let (identity, slot) = state
            .slots
            .iter()
            .find(|(identity, _)| identity.context_matches(context, name))
            .ok_or(Error::Denied)?;
        if !matches!(slot, Slot::Ready(_)) {
            return Err(Error::Denied);
        }
        Ok(NativeFileApprovalClaim {
            identity: identity.clone(),
            generation: slot.generation(),
        })
    }

    pub(crate) fn claim(
        &self,
        ticket: NativeFileApprovalClaim,
        context: &ToolContext,
        name: &str,
        arguments: &Value,
        root: BorrowedFd<'_>,
        cancellation: &CancellationToken,
    ) -> Result<NativeFileApprovalExecution, Error> {
        self.claim_checked(ticket, context, name, cancellation, |proof| {
            if proof
                .execution_arguments
                .as_ref()
                .unwrap_or(&proof.arguments)
                != arguments
            {
                return Err(Error::Denied);
            }
            proof.snapshot.root_matches(root)
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "binds the exact core invocation and both retained native endpoints"
    )]
    pub(crate) fn claim_endpoints(
        &self,
        ticket: NativeFileApprovalClaim,
        context: &ToolContext,
        name: &str,
        arguments: &Value,
        source: Option<&NativeFileEndpoint>,
        target: &NativeFileEndpoint,
        cancellation: &CancellationToken,
    ) -> Result<NativeFileApprovalExecution, Error> {
        self.claim_checked(ticket, context, name, cancellation, |proof| {
            if proof.arguments != *arguments {
                return Err(Error::Denied);
            }
            proof.snapshot.endpoints_match(source, target)
        })
    }

    fn claim_checked(
        &self,
        ticket: NativeFileApprovalClaim,
        context: &ToolContext,
        name: &str,
        cancellation: &CancellationToken,
        check_roots: impl FnOnce(&Proof) -> Result<(), Error>,
    ) -> Result<NativeFileApprovalExecution, Error> {
        snapshot::check(cancellation)?;
        let removed = {
            let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if !ticket.identity.context_matches(context, name)
                || !matches!(state.slots.get(&ticket.identity), Some(Slot::Ready(proof)) if proof.ticket.generation == ticket.generation)
            {
                return Err(Error::Denied);
            }
            state.slots.remove(&ticket.identity).ok_or(Error::Denied)?
        };
        drop(ticket); // The construction stamp is consumed together with the route.
        let Slot::Ready(proof) = removed else {
            return Err(Error::Denied);
        };
        check_roots(&proof)?;
        proof.snapshot.revalidate(cancellation)?;
        proof
            .policy
            .as_ref()
            .ok_or(Error::Denied)?
            .revalidate()
            .map_err(|_| Error::Denied)?;
        snapshot::check(cancellation)?;
        Ok(NativeFileApprovalExecution { proof })
    }
}

struct Budget(Weak<NativeFileApprovalRegistry>);
impl Drop for Budget {
    fn drop(&mut self) {
        if let Some(registry) = self.0.upgrade() {
            registry.retained.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
struct Ticket {
    registry: Weak<NativeFileApprovalRegistry>,
    identity: Identity,
    generation: u64,
    _budget: Budget,
}
impl Ticket {
    fn ensure_reserved(&self) -> Result<(), Error> {
        let registry = self.registry.upgrade().ok_or(Error::Denied)?;
        let state = registry.state.lock().map_err(|_| Error::Unavailable)?;
        if matches!(state.slots.get(&self.identity), Some(Slot::Reserved(generation)) if *generation == self.generation)
        {
            Ok(())
        } else {
            Err(Error::Denied)
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        let removed = {
            let mut state = registry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state
                .slots
                .get(&self.identity)
                .is_some_and(|slot| slot.generation() == self.generation)
            {
                state.slots.remove(&self.identity)
            } else {
                None
            }
        };
        drop(removed);
    }
}

struct Proof {
    ticket: Ticket,
    snapshot: Snapshot,
    kind: NativeFileApprovalKind,
    arguments: Value,
    // Only endpoint preparations carry this exact, validated execution alias.
    execution_arguments: Option<Value>,
    policy: Option<Arc<dyn NativeFileApprovalPolicy>>,
}

/// Owned evidence across a prompt; no mutex guard or implicit approval is held.
pub struct PreparedFileApproval {
    proof: Proof,
}
impl fmt::Debug for PreparedFileApproval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedFileApproval")
            .finish_non_exhaustive()
    }
}
impl PreparedFileApproval {
    #[must_use]
    pub const fn kind(&self) -> NativeFileApprovalKind {
        self.proof.kind
    }
    #[must_use]
    pub fn tool_name(&self) -> &str {
        self.proof.ticket.identity.name.as_str()
    }
    #[must_use]
    pub fn target_path(&self) -> &str {
        self.proof.snapshot.target_path()
    }
    #[must_use]
    pub fn source_path(&self) -> Option<&str> {
        self.proof.snapshot.source_path()
    }
    #[must_use]
    pub fn preimage(&self) -> NativeFileApprovalPreimage<'_> {
        self.proof.snapshot.preimage()
    }
    #[must_use]
    pub fn source_preimage(&self) -> Option<&[u8]> {
        self.proof.snapshot.source_preimage()
    }
    #[must_use]
    pub fn postimage(&self) -> Option<&[u8]> {
        self.proof.snapshot.postimage()
    }
    /// Content-rule identity deliberately excludes transient runtime inode IDs.
    /// # Errors
    /// An overlong canonical identity is unavailable for saving; the owned
    /// one-shot approval remains valid and is not truncated or weakened.
    pub fn saved_rule_key(&self) -> Result<crate::NativePermissionRuleKey, Error> {
        self.proof
            .snapshot
            .content_identity(self.tool_name(), &self.proof.arguments)
    }
    /// Constructs the owned core admission without running policy or publishing
    /// a route. Dropping it before core admission releases this exact generation.
    #[must_use]
    pub fn admit(
        mut self,
        policy: Arc<dyn NativeFileApprovalPolicy>,
    ) -> NativeFileApprovalAdmission {
        self.proof.policy = Some(policy);
        NativeFileApprovalAdmission { proof: self.proof }
    }
}

/// One-shot core admission; the host retains turn cleanup responsibility.
pub struct NativeFileApprovalAdmission {
    proof: Proof,
}
impl fmt::Debug for NativeFileApprovalAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFileApprovalAdmission")
            .finish_non_exhaustive()
    }
}
impl PermissionExecutionAdmission for NativeFileApprovalAdmission {
    fn admit(self: Box<Self>) -> Result<(), PermissionError> {
        let proof = self.proof;
        proof
            .policy
            .as_ref()
            .ok_or(Error::Denied)
            .map_err(permission_error)?
            .revalidate()
            .map_err(|_| permission_error(Error::Denied))?;
        let registry = proof
            .ticket
            .registry
            .upgrade()
            .ok_or_else(|| permission_error(Error::Denied))?;
        let mut state = registry
            .state
            .lock()
            .map_err(|_| permission_error(Error::Unavailable))?;
        if !matches!(state.slots.get(&proof.ticket.identity), Some(Slot::Reserved(generation)) if *generation == proof.ticket.generation)
        {
            return Err(permission_error(Error::Denied));
        }
        let key = proof.ticket.identity.clone();
        let previous = state.slots.insert(key, Slot::Ready(Box::new(proof)));
        drop(state);
        drop(previous);
        Ok(())
    }
}

pub(crate) struct NativeFileApprovalExecution {
    proof: Box<Proof>,
}
impl NativeFileApprovalExecution {
    pub(crate) fn revalidate(&self, cancellation: &CancellationToken) -> Result<(), Error> {
        self.proof.snapshot.revalidate(cancellation)?;
        self.policy(cancellation)
    }
    pub(crate) fn revalidate_stage(
        &self,
        parent: BorrowedFd<'_>,
        name: &str,
        descriptor: BorrowedFd<'_>,
        cancellation: &CancellationToken,
    ) -> Result<(), Error> {
        self.proof.snapshot.revalidate(cancellation)?;
        self.proof
            .snapshot
            .verify_stage(parent, name, descriptor, cancellation)?;
        self.policy(cancellation)
    }
    fn policy(&self, cancellation: &CancellationToken) -> Result<(), Error> {
        snapshot::check(cancellation)?;
        self.proof
            .policy
            .as_ref()
            .ok_or(Error::Denied)?
            .revalidate()
            .map_err(|_| Error::Denied)?;
        snapshot::check(cancellation)
    }
}

fn validate_invocation(
    request: &PermissionRequest,
    invocation: PermissionInvocation<'_>,
) -> Result<NativeFileApprovalKind, Error> {
    use NativeFileApprovalKind as Kind;
    let args = invocation.arguments;
    let kind = match invocation.tool_name.as_str() {
        crate::WRITE_FILE_TOOL_NAME => {
            crate::write_file::validate_approval_arguments(args)?;
            Kind::Write
        }
        crate::EDIT_FILE_TOOL_NAME => {
            crate::edit_file::validate_approval_arguments(args)?;
            Kind::Edit
        }
        crate::DELETE_FILE_TOOL_NAME => {
            crate::delete_file::validate_approval_arguments(args)?;
            Kind::Delete
        }
        crate::RENAME_FILE_TOOL_NAME => {
            crate::rename_file::validate_approval_arguments(args)?;
            Kind::Rename
        }
        crate::COPY_FILE_TOOL_NAME => {
            crate::copy_file::validate_approval_arguments(args)?;
            Kind::Copy
        }
        _ => return Err(Error::Invalid),
    };
    validate_capability(request, kind, args)?;
    Ok(kind)
}

fn validate_endpoint_invocation(
    request: &PermissionRequest,
    invocation: PermissionInvocation<'_>,
    source: Option<&NativeFileEndpoint>,
    target: &NativeFileEndpoint,
) -> Result<(NativeFileApprovalKind, Value), Error> {
    use NativeFileApprovalKind as Kind;
    let kind = match invocation.tool_name.as_str() {
        crate::WRITE_FILE_TOOL_NAME => Kind::Write,
        crate::EDIT_FILE_TOOL_NAME => Kind::Edit,
        crate::DELETE_FILE_TOOL_NAME => Kind::Delete,
        crate::COPY_FILE_TOOL_NAME => Kind::Copy,
        crate::RENAME_FILE_TOOL_NAME => Kind::Rename,
        _ => return Err(Error::Invalid),
    };
    let private = project_endpoint_arguments(kind, invocation.arguments, source, target)?;
    validate_capability(request, kind, invocation.arguments)?;
    Ok((kind, private))
}

fn validate_capability(
    request: &PermissionRequest,
    kind: NativeFileApprovalKind,
    args: &Value,
) -> Result<(), Error> {
    use NativeFileApprovalKind as Kind;
    let text = |key: &str| {
        args[key]
            .as_str()
            .expect("validated flat arguments")
            .to_owned()
    };
    let expected = match kind {
        Kind::Write | Kind::Edit | Kind::Delete => Capability::Filesystem {
            path: text("path"),
            access: match kind {
                Kind::Write => FilesystemAccess::Write,
                Kind::Edit => FilesystemAccess::Edit,
                _ => FilesystemAccess::Delete,
            },
        },
        Kind::Copy => Capability::FilesystemCopy {
            source: text("source"),
            destination: text("destination"),
        },
        Kind::Rename => Capability::FilesystemRename {
            old_path: text("old_path"),
            new_path: text("new_path"),
        },
    };
    if request.capability != expected {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod endpoint_tests;
#[cfg(test)]
pub(crate) mod tests;
