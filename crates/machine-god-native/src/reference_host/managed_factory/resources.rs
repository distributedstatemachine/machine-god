use super::{Arc, BoxFuture, CancellationToken, Duration, ManagedMcpInstance, ManagedRuntimeError};
use crate::managed::{
    conversation::ManagedConversationBinding, manager::factory::ManagedRuntimeResources,
    mcp::NativePrincipalMcpOwner, scheduler::RunRef,
};
use crate::{NativeOwnedWorkerCompletion, mcp::runtime::NativeMcpPeerCompletion};
use std::{
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};

pub(super) struct Resources {
    binding: ManagedConversationBinding,
    mcp: Arc<McpLifetime>,
    preparation: NativeOwnedWorkerCompletion,
    turn: Option<(RunRef, Option<BoxFuture<'static, ()>>)>,
    admission: Option<BoxFuture<'static, ()>>,
    close_authority: CloseAuthority,
    closing: Close,
    prompt: Option<Prompt>,
}
pub(super) enum Prompt {
    Active(crate::NativeInteractivePromptPrincipal),
    Reserved(crate::interactive_prompts::NativeInteractivePromptReservation),
}
#[derive(Clone)]
pub(super) struct CloseAuthority {
    pub workers: crate::NativeOwnedWorkerScope,
    pub journal_owner: crate::managed::store::JournalOwner,
    pub timeout: Duration,
}
pub(super) struct McpLifetime {
    pub(super) instance: ManagedMcpInstance,
    owner: OnceLock<Arc<NativePrincipalMcpOwner>>,
    closed: AtomicBool,
    startup: Option<NativeOwnedWorkerCompletion>,
}
impl McpLifetime {
    pub(super) fn new(
        instance: ManagedMcpInstance,
        startup: Option<NativeOwnedWorkerCompletion>,
    ) -> Self {
        Self {
            instance,
            owner: OnceLock::new(),
            closed: AtomicBool::new(false),
            startup,
        }
    }
    /// A staged instance binds exactly once, without copying its publication.
    /// Closing before or concurrently with binding cannot revive admission.
    pub(super) fn bind(
        &self,
        owner: Arc<NativePrincipalMcpOwner>,
    ) -> Result<(), ManagedRuntimeError> {
        if self.closed.load(Ordering::Acquire) {
            owner.retire();
            return Err(ManagedRuntimeError::Unavailable);
        }
        if let Err(owner) = self.owner.set(owner) {
            owner.retire();
            return Err(ManagedRuntimeError::Invalid);
        }
        if self.closed.load(Ordering::Acquire) {
            self.owner.get().expect("bound MCP owner").retire();
            return Err(ManagedRuntimeError::Unavailable);
        }
        Ok(())
    }
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(owner) = self.owner.get() {
            owner.retire();
        }
        if let Some(controller) = &self.instance.controller {
            controller.close();
        }
        if let Some(ephemeral) = &self.instance.ephemeral {
            ephemeral.close();
        }
        self.instance.runtime.close();
    }
}
impl Drop for McpLifetime {
    fn drop(&mut self) {
        self.close();
    }
}
enum Close {
    Open,
    Running(BoxFuture<'static, Result<(), ManagedRuntimeError>>),
    Finished(Result<(), ManagedRuntimeError>),
}
impl Resources {
    pub(super) fn new(
        binding: ManagedConversationBinding,
        mcp: Arc<McpLifetime>,
        preparation: NativeOwnedWorkerCompletion,
        prompt: Option<Prompt>,
        close_authority: CloseAuthority,
    ) -> Self {
        Self {
            binding,
            mcp,
            preparation,
            turn: None,
            admission: None,
            close_authority,
            closing: Close::Open,
            prompt,
        }
    }
}
impl ManagedRuntimeResources for Resources {
    fn activate_foreground(&mut self) -> Result<(), ManagedRuntimeError> {
        if !matches!(self.closing, Close::Open) {
            return Err(ManagedRuntimeError::Unavailable);
        }
        self.prompt = match self.prompt.take() {
            Some(Prompt::Reserved(reservation)) => match reservation.activate() {
                Ok(principal) => Some(Prompt::Active(principal)),
                Err((error, reservation)) => {
                    self.prompt = Some(Prompt::Reserved(reservation));
                    return Err(super::prompt_error(error));
                }
            },
            current => current,
        };
        Ok(())
    }
    fn mcp_controls(&self) -> Option<crate::managed::manager::factory::ManagedMcpControls> {
        if !matches!(self.closing, Close::Open) {
            return None;
        }
        Some(crate::managed::manager::factory::ManagedMcpControls {
            runtime: Some(self.mcp.instance.runtime.clone()),
            controller: self.mcp.instance.controller.clone(),
            ephemeral: self.mcp.instance.ephemeral.clone(),
        })
    }
    fn poll_turn_settled(
        &mut self,
        cx: &mut Context<'_>,
        run: &RunRef,
    ) -> Poll<Result<(), ManagedRuntimeError>> {
        if self
            .turn
            .as_ref()
            .is_some_and(|(original, _)| !original.same_run(run))
        {
            self.turn = None;
        }
        if self.turn.is_none() {
            let Ok(cleanup) = self.binding.cleanup_for(run) else {
                return Poll::Ready(Err(ManagedRuntimeError::Invalid));
            };
            let completion = cleanup.completion();
            let preparation = self.preparation.clone();
            self.turn = Some((
                run.clone(),
                Some(Box::pin(async move {
                    preparation.wait().await;
                    completion.wait().await;
                })),
            ));
        }
        let future = &mut self.turn.as_mut().expect("exact run waiter").1;
        let Some(waiting) = future else {
            return Poll::Ready(Ok(()));
        };
        let result = waiting.as_mut().poll(cx);
        if result.is_ready() {
            *future = None;
        }
        result.map(Ok)
    }
    fn poll_admission_settled(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), ManagedRuntimeError>> {
        if self.admission.is_none() {
            let completion = self.binding.admission_completion();
            let preparation = self.preparation.clone();
            self.admission = Some(Box::pin(async move {
                preparation.wait().await;
                if let Some(completion) = completion {
                    completion.wait().await;
                }
            }));
        }
        let result = self
            .admission
            .as_mut()
            .expect("admission waiter")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            self.admission = None;
        }
        result.map(Ok)
    }
    fn begin_close(&mut self) {
        if !matches!(self.closing, Close::Open) {
            return;
        }
        // Cut off exactly this principal's pending prompts and unconsumed
        // answers before waiting for any permission/elicitation cleanup.
        if let Some(Prompt::Active(mut principal)) = self.prompt.take() {
            principal.retire();
        }
        let preparation = self.preparation.clone();
        let admission = self.binding.admission_completion();
        self.closing = Close::Running(close_mcp(
            self.mcp.clone(),
            self.close_authority.clone(),
            Box::pin(async move {
                preparation.wait().await;
                if let Some(admission) = admission {
                    admission.wait().await;
                }
            }),
        ));
    }
    fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), ManagedRuntimeError>> {
        self.begin_close();
        let result = match &mut self.closing {
            Close::Open => unreachable!("close started"),
            Close::Finished(result) => return Poll::Ready(*result),
            Close::Running(future) => future.as_mut().poll(cx),
        };
        if let Poll::Ready(result) = result {
            self.closing = Close::Finished(result);
        }
        result
    }
}

/// Shared retirement for an unbound staged parent and an enrolled runtime.
/// The original completion is observed alongside peer settlement, never instead
/// of it. An active startup cohort is not a per-turn settlement prerequisite.
pub(super) fn close_mcp(
    mcp: Arc<McpLifetime>,
    authority: CloseAuthority,
    original: BoxFuture<'static, ()>,
) -> BoxFuture<'static, Result<(), ManagedRuntimeError>> {
    let Some(deadline) = mcp.instance.clock.now().checked_add(authority.timeout) else {
        mcp.close();
        return Box::pin(async { Err(ManagedRuntimeError::Invalid) });
    };
    let cohort = authority
        .workers
        .begin_cleanup_run_with_keepalive(Arc::new(authority.journal_owner.clone()))
        .ok()
        .map(Arc::new);
    if let Some(cohort) = &cohort {
        cohort.with_poll(|| mcp.close());
    } else {
        // Cutoff is immediate; the original peers still own their reap tickets.
        mcp.close();
    }
    Box::pin(async move {
        let owned = async {
            let cohort = match cohort {
                Some(cohort) => cohort,
                None => reserve_close(&authority).await?,
            };
            let completion = cohort.completion();
            let selected = mcp.clone();
            let result = super::preparation::Attributed::new(
                cohort,
                Box::pin(async move {
                    selected.close();
                    let original = async {
                        original.await;
                        if let Some(startup) = &selected.startup {
                            startup.wait().await;
                        }
                    };
                    let (result, ()) =
                        futures_util::future::join(settle_mcp(&selected, deadline), original).await;
                    result
                }),
            )
            .await;
            completion.wait().await;
            result
        };
        match futures_util::future::select(
            Box::pin(owned),
            mcp.instance.clock.sleep_until(deadline),
        )
        .await
        {
            futures_util::future::Either::Left((result, _)) => result,
            futures_util::future::Either::Right(_) => Err(ManagedRuntimeError::Unavailable),
        }
    })
}

async fn reserve_close(
    authority: &CloseAuthority,
) -> Result<Arc<crate::owned_worker::NativeOwnedWorkerRun>, ManagedRuntimeError> {
    loop {
        if let Ok(cohort) = authority
            .workers
            .begin_cleanup_run_with_keepalive(Arc::new(authority.journal_owner.clone()))
        {
            return Ok(Arc::new(cohort));
        }
        authority
            .workers
            .wait_for_cleanup_capacity()
            .await
            .map_err(|_| ManagedRuntimeError::Unavailable)?;
    }
}
impl Drop for Resources {
    fn drop(&mut self) {
        self.mcp.close();
    }
}
async fn settle_mcp(mcp: &McpLifetime, deadline: Instant) -> Result<(), ManagedRuntimeError> {
    let cancellation = CancellationToken::new();
    if let Some(ephemeral) = &mcp.instance.ephemeral {
        ephemeral
            .settle(cancellation, deadline)
            .await
            .map_err(|_| ManagedRuntimeError::Unavailable)?;
    } else if let Some(controller) = &mcp.instance.controller {
        let receipt = controller
            .settle(deadline, cancellation)
            .await
            .map_err(|_| ManagedRuntimeError::Unavailable)?;
        if !receipt.complete {
            return Err(ManagedRuntimeError::Unavailable);
        }
    } else {
        let peers = mcp
            .instance
            .runtime
            .drain_retired(deadline, cancellation)
            .await
            .map_err(|_| ManagedRuntimeError::Unavailable)?;
        for peer in peers {
            match peer {
                NativeMcpPeerCompletion::Stdio(completion) => completion.wait().await,
                #[cfg(feature = "mcp-http")]
                NativeMcpPeerCompletion::Http(completion) => completion.completed().await,
                #[cfg(test)]
                NativeMcpPeerCompletion::Script(completion) => {
                    if !completion.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(ManagedRuntimeError::Unavailable);
                    }
                }
            }
        }
    }
    Ok(())
}
