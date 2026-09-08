//! Production composition of retained native evidence and exact live review context.

mod action;
mod identity;
mod input;

use crate::{
    NativeFileApprovalAuthority, NativeFileApprovalRegistry, NativeOwnedWorkerScope,
    NativePermissionActionPreparer, NativePermissionContexts, NativePermissionController,
    NativePermissionReviewContext, NativePermissionReviewer, NativePermissionTargetAuthority,
    NativePreparedPermissionAction, NativePreparedPermissionTargets, PreparedFileApproval,
};
use futures_util::future::{Either, select};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionInvocation, PermissionRequest,
    SessionId, SessionIncarnationId, ToolCall, TurnId,
};
use std::sync::{
    Arc, OnceLock, Weak,
    atomic::{AtomicUsize, Ordering},
};

/// Explicit native authorities; construction starts no worker, review or I/O.
/// The host must attach the same contexts/controller to each conversation and
/// inject the same file registry into all five mutation tools.
pub struct NativeToolPermissionPreparer {
    targets: Arc<NativePermissionTargetAuthority>,
    files: Arc<NativeFileApprovalAuthority>,
    registry: Arc<NativeFileApprovalRegistry>,
    contexts: Arc<NativePermissionContexts>,
    reviewer: Arc<dyn NativePermissionReviewer>,
    workers: NativeOwnedWorkerScope,
    active: Arc<AtomicUsize>,
    controller: OnceLock<Weak<NativePermissionController>>,
}
impl NativeToolPermissionPreparer {
    #[must_use]
    pub fn new(
        targets: Arc<NativePermissionTargetAuthority>,
        files: Arc<NativeFileApprovalAuthority>,
        registry: Arc<NativeFileApprovalRegistry>,
        contexts: Arc<NativePermissionContexts>,
        reviewer: Arc<dyn NativePermissionReviewer>,
        workers: NativeOwnedWorkerScope,
    ) -> Self {
        Self {
            targets,
            files,
            registry,
            contexts,
            reviewer,
            workers,
            active: Arc::new(AtomicUsize::new(0)),
            controller: OnceLock::new(),
        }
    }

    /// Binds the actual controller once without creating an ownership cycle.
    /// This permits configured read grants to satisfy write/edit disclosure.
    /// # Errors
    /// Rejects replacement of an already bound controller.
    pub fn bind_controller(
        &self,
        controller: &Arc<NativePermissionController>,
    ) -> Result<(), PermissionError> {
        self.controller
            .set(Arc::downgrade(controller))
            .map_err(|_| invalid())
    }
}
impl std::fmt::Debug for NativeToolPermissionPreparer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeToolPermissionPreparer { .. }")
    }
}

impl NativePermissionActionPreparer for NativeToolPermissionPreparer {
    fn close_turn(&self, session: &SessionId, incarnation: &SessionIncarnationId, turn: &TurnId) {
        self.registry.close_turn(session, incarnation, turn);
    }

    fn prepare<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        Box::pin(async move {
            check(&cancellation)?;
            let context = self.contexts.snapshot(request).map_err(|_| invalid())?;
            if context.permission_policy().is_none()
                || context.target_call_id() != invocation.call_id
            {
                return Err(invalid());
            }
            let permit = Permit::acquire(&self.active)?;
            let call = input::copy_invocation(request, invocation, &cancellation)?;
            let request = request.clone(); // bounded before copying its capability
            let session = request.session_id.clone();
            let incarnation = request.session_incarnation_id.clone();
            let turn = request.turn_id.clone();
            let effective = CancellationToken::new();
            let cancel_on_drop = CancelOnDrop(effective.clone());
            let targets = Arc::clone(&self.targets);
            let files = Arc::clone(&self.files);
            let registry = Arc::clone(&self.registry);
            let operation = self.workers.run(move || {
                let evidence =
                    prepare_on_worker(&targets, &files, &registry, &request, &call, &effective)?;
                check_context(&context, &effective)?;
                Ok::<_, PermissionError>((evidence, context, permit))
            });
            let ((targets, file), context, permit) =
                match select(operation, cancellation.cancelled()).await {
                    Either::Left((result, _)) => result.map_err(|_| invalid())??,
                    Either::Right(_) => return Err(invalid()),
                };
            drop(cancel_on_drop);
            check_context(&context, &cancellation)?;
            Ok(Box::new(action::Action::new(
                action::Route {
                    session,
                    incarnation,
                    turn,
                },
                targets,
                file,
                context,
                Arc::clone(&self.reviewer),
                permit,
                self.controller.get().cloned().unwrap_or_default(),
            )?) as Box<dyn NativePreparedPermissionAction>)
        })
    }
}

fn prepare_on_worker(
    targets: &NativePermissionTargetAuthority,
    files: &NativeFileApprovalAuthority,
    registry: &Arc<NativeFileApprovalRegistry>,
    request: &PermissionRequest,
    call: &ToolCall,
    cancellation: &CancellationToken,
) -> Result<
    (
        NativePreparedPermissionTargets,
        Option<PreparedFileApproval>,
    ),
    PermissionError,
> {
    check(cancellation)?;
    let invocation = PermissionInvocation {
        tool_name: &call.name,
        call_id: &call.id,
        arguments: &call.arguments,
    };
    if matches!(
        call.name.as_str(),
        "write_file" | "edit_file" | "delete_file" | "rename_file" | "copy_file"
    ) {
        targets.validate_file_authority(files)?;
        let file = futures_executor::block_on(registry.prepare(
            files,
            request,
            invocation,
            cancellation.clone(),
        ))
        .map_err(|_| invalid())?;
        let projection = targets.from_file(&file, invocation, cancellation)?;
        Ok((projection, Some(file)))
    } else {
        let prepared =
            futures_executor::block_on(targets.prepare(request, invocation, cancellation.clone()))?;
        prepared.revalidate()?;
        Ok((prepared, None))
    }
}

struct Permit(Arc<AtomicUsize>);
impl Permit {
    fn acquire(active: &Arc<AtomicUsize>) -> Result<Self, PermissionError> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| invalid())?;
        Ok(Self(Arc::clone(active)))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
fn check(cancellation: &CancellationToken) -> Result<(), PermissionError> {
    if cancellation.is_cancelled() {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn check_context(
    context: &NativePermissionReviewContext,
    cancellation: &CancellationToken,
) -> Result<(), PermissionError> {
    check(cancellation)?;
    context.is_live().then_some(()).ok_or_else(invalid)
}
fn invalid() -> PermissionError {
    PermissionError::new(
        "permission_preparation_failed",
        "native permission preparation failed",
    )
}
