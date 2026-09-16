//! Exact parent MCP preparation, transferable only into its original factory.
//! The outer selection must retain its residency reservation and this owner
//! through startup, adoption, rejection and actual settlement.
use super::{
    Arc, BoxFuture, CancellationToken, Duration, JournalOwner, ManagedRestorationAuthority,
    ManagedRuntimeError, NativeConversation, NoticePrincipal, PreparedManagedRuntime,
    RuntimeSelection, SharedManagedRuntimeFactory, SharedManagedRuntimeFactoryOptions, preparation,
    resources::{self, CloseAuthority, McpLifetime},
};
use crate::mcp::ephemeral::NativeMcpEphemeralConfiguration;
use std::{fmt, sync::Weak};
#[cfg(test)]
mod tests;

/// Successful allocation retains a stage even when peer startup failed.
/// Call `ready` before adoption; rejection still owns `settle` obligations.
pub(in crate::reference_host) struct StagedParentMcp {
    factory: Weak<SharedManagedRuntimeFactoryOptions>,
    mcp: Option<Arc<McpLifetime>>,
    authority: CloseAuthority,
    startup: Result<(), ManagedRuntimeError>,
    closing: Option<BoxFuture<'static, Result<(), ManagedRuntimeError>>>,
    settled: Option<Result<(), ManagedRuntimeError>>,
}
pub(in crate::reference_host) struct StagedParentFailure {
    pub error: ManagedRuntimeError,
    pub stage: StagedParentMcp,
}
impl fmt::Debug for StagedParentMcp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StagedParentMcp { .. }")
    }
}
impl fmt::Debug for StagedParentFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagedParentFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl StagedParentMcp {
    pub(in crate::reference_host) fn ready(&self) -> Result<(), ManagedRuntimeError> {
        self.startup?;
        self.mcp
            .as_ref()
            .ok_or(ManagedRuntimeError::Invalid)?
            .instance
            .ephemeral
            .as_ref()
            .ok_or(ManagedRuntimeError::Invalid)?
            .ready()
            .map_err(|_| ManagedRuntimeError::Unavailable)
    }

    /// Inert until polled. Abandoning the caller's wrapper preserves the original
    /// operation. Terminal cleanup failure remains a failure and requires the
    /// outer owner to fence selection, never retry toward a success claim.
    pub(in crate::reference_host) fn settle(
        &mut self,
    ) -> BoxFuture<'_, Result<(), ManagedRuntimeError>> {
        Box::pin(async move {
            if let Some(result) = self.settled {
                return result;
            }
            if self.closing.is_none() {
                self.closing = Some(resources::close_mcp(
                    self.mcp.clone().ok_or(ManagedRuntimeError::Invalid)?,
                    self.authority.clone(),
                    Box::pin(async {}),
                ));
            }
            let result = futures_util::future::poll_fn(|cx| {
                self.closing
                    .as_mut()
                    .expect("retained staged cleanup")
                    .as_mut()
                    .poll(cx)
            })
            .await;
            self.closing = None;
            self.settled = Some(result);
            result
        })
    }
}
impl Drop for StagedParentMcp {
    fn drop(&mut self) {
        if let Some(mcp) = &self.mcp {
            mcp.close();
        }
    }
}

impl SharedManagedRuntimeFactory {
    pub(in crate::reference_host) fn stage_parent_mcp(
        &self,
        journal_owner: JournalOwner,
        seed: Arc<crate::reference_host::mcp::ManagedParentMcpSeed>,
        configuration: NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<StagedParentMcp, ManagedRuntimeError>> {
        let factory = Arc::downgrade(&self.0);
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(ManagedRuntimeError::Unavailable);
            }
            let selected = factory.upgrade().ok_or(ManagedRuntimeError::Unavailable)?;
            let cohort = preparation::begin(&selected, &journal_owner)?;
            let completion = cohort.completion();
            preparation::Attributed::new(
                cohort,
                Box::pin(async move {
                    let workers = selected
                        .services
                        .control_workers
                        .as_ref()
                        .ok_or(ManagedRuntimeError::Invalid)?;
                    let instance = seed
                        .compose(
                            workers,
                            &selected.reserved_tool_names,
                            selected
                                .services
                                .permission_preparation
                                .as_ref()
                                .ok_or(ManagedRuntimeError::Invalid)?,
                        )
                        .map_err(|_| ManagedRuntimeError::Invalid)?;
                    if instance.ephemeral.is_none() || instance.controller.is_some() {
                        return Err(ManagedRuntimeError::Invalid);
                    }
                    let mcp = Arc::new(McpLifetime::new(instance, Some(completion)));
                    let mut stage = StagedParentMcp {
                        factory,
                        mcp: Some(mcp.clone()),
                        authority: CloseAuthority {
                            workers: workers.clone(),
                            journal_owner,
                            timeout: selected.cleanup_timeout,
                        },
                        startup: Err(ManagedRuntimeError::Unavailable),
                        closing: None,
                        settled: None,
                    };
                    stage.startup = start(&mcp, configuration, cancellation).await;
                    // Startup workers may own live peers. Do not wait for their
                    // completion until retirement co-polls those peers' settlement.
                    Ok(stage)
                }),
            )
            .await
        })
    }

    pub(in crate::reference_host) fn prepare_staged_parent(
        &self,
        mut stage: StagedParentMcp,
        conversation: NativeConversation,
        authority: ManagedRestorationAuthority,
        principal: NoticePrincipal,
    ) -> BoxFuture<'static, Result<PreparedManagedRuntime, StagedParentFailure>> {
        let factory = Arc::downgrade(&self.0);
        Box::pin(async move {
            let preparation = (|| {
                if !Weak::ptr_eq(&factory, &stage.factory) {
                    return Err(ManagedRuntimeError::Invalid);
                }
                stage.ready()?;
                let factory = factory.upgrade().ok_or(ManagedRuntimeError::Unavailable)?;
                let journal_owner = stage.authority.journal_owner.clone();
                let cohort = preparation::begin(&factory, &journal_owner)?;
                let completion = cohort.completion();
                let prepared_completion = completion.clone();
                let mcp = stage.mcp.clone().ok_or(ManagedRuntimeError::Invalid)?;
                let operation = preparation::Attributed::new(
                    cohort,
                    Box::pin(async move {
                        factory.compose_selected(
                            conversation,
                            &journal_owner,
                            RuntimeSelection {
                                principal,
                                authority,
                                parent_mcp: Some(mcp),
                            },
                            prepared_completion,
                        )
                    }),
                );
                Ok((completion, operation))
            })();
            let result = match preparation {
                Ok((completion, operation)) => {
                    let result = operation.await;
                    completion.wait().await;
                    result
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(prepared) => {
                    // Resources now owns the exact instance and original startup
                    // completion. Disarm only this wrapper's final cutoff.
                    stage.mcp.take();
                    Ok(prepared)
                }
                Err(error) => Err(StagedParentFailure { error, stage }),
            }
        })
    }
}

async fn start(
    mcp: &McpLifetime,
    configuration: NativeMcpEphemeralConfiguration,
    cancellation: CancellationToken,
) -> Result<(), ManagedRuntimeError> {
    let deadline = mcp
        .instance
        .clock
        .now()
        .checked_add(Duration::from_secs(30))
        .ok_or(ManagedRuntimeError::Invalid)?;
    let owner = mcp
        .instance
        .ephemeral
        .as_ref()
        .ok_or(ManagedRuntimeError::Invalid)?;
    let receipt = owner
        .replace(configuration, cancellation.clone(), deadline)
        .await
        .map_err(|_| ManagedRuntimeError::Unavailable)?;
    if receipt.closed_after_publication() || cancellation.is_cancelled() {
        return Err(ManagedRuntimeError::Unavailable);
    }
    owner.ready().map_err(|_| ManagedRuntimeError::Unavailable)
}
