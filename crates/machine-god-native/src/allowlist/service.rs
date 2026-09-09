//! Config I/O and policy reload share the accepted runtime's lifecycle permit.

use super::{
    NativeAllowlistCommand as Command, NativeAllowlistError as Error,
    NativeAllowlistReceipt as Receipt, NativeAllowlistReloadError as ReloadError,
    NativeAllowlistRequest, NativeAllowlistSources as Sources,
};
use crate::{
    NativeConversationRuntime, NativeOwnedWorkerScope, NativePermissionSession,
    NativeUserConfigStore, conversation_lifecycle::LifecyclePermit,
};
use machine_god_core::BoxFuture;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) fn execute(
    runtime: Arc<NativeConversationRuntime>,
    store: Arc<NativeUserConfigStore>,
    workspace: PathBuf,
    workers: NativeOwnedWorkerScope,
    request: NativeAllowlistRequest,
) -> BoxFuture<'static, Result<Receipt, Error>> {
    execute_inner(
        runtime,
        store,
        workspace,
        workers,
        request,
        #[cfg(test)]
        None,
    )
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Stage {
    BeforeLoad,
    AfterCommit,
    BeforeCleanup,
}
#[cfg(test)]
pub(crate) type Hook = Arc<dyn Fn(Stage) + Send + Sync>;

pub(crate) fn execute_inner(
    runtime: Arc<NativeConversationRuntime>,
    store: Arc<NativeUserConfigStore>,
    workspace: PathBuf,
    workers: NativeOwnedWorkerScope,
    request: NativeAllowlistRequest,
    #[cfg(test)] hook: Option<Hook>,
) -> BoxFuture<'static, Result<Receipt, Error>> {
    Box::pin(async move {
        let permit = runtime
            .acquire_file_control()
            .map_err(|_| Error::Unavailable)?;
        let permissions = runtime.permissions().cloned().ok_or(Error::Unavailable)?;
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_start = started.clone();
        let mutation = matches!(request.command, Command::Mutate { .. });
        workers
            .run(move || {
                started.store(true, std::sync::atomic::Ordering::Release);
                // Neither abandoning the response nor a transition can release
                // admission while publication/reload is still executing. Actual
                // thread/TLS join remains the separate host completion obligation.
                let result = run(
                    &store,
                    &workspace,
                    &permissions,
                    &permit,
                    request,
                    #[cfg(test)]
                    hook.as_ref(),
                );
                let cleanup = contain(|| {
                    #[cfg(test)]
                    if let Some(hook) = hook {
                        hook(Stage::BeforeCleanup);
                    }
                    drop(permit);
                });
                match (result, cleanup) {
                    (Ok(mut receipt), Err(())) => {
                        match &mut receipt {
                            Receipt::View { reload, .. } => {
                                *reload = Err(ReloadError::Unavailable);
                            }
                            Receipt::Mutation { reload, .. } => {
                                *reload = Some(Err(ReloadError::Unavailable));
                            }
                        }
                        Ok(receipt)
                    }
                    (result, _) => result,
                }
            })
            .await
            .map_err(|_| {
                if mutation && observed_start.load(std::sync::atomic::Ordering::Acquire) {
                    Error::Ambiguous
                } else {
                    Error::Unavailable
                }
            })?
    })
}

fn run(
    store: &NativeUserConfigStore,
    workspace: &Path,
    permissions: &NativePermissionSession,
    permit: &LifecyclePermit,
    request: NativeAllowlistRequest,
    #[cfg(test)] hook: Option<&Hook>,
) -> Result<Receipt, Error> {
    let snapshot = contain(|| {
        #[cfg(test)]
        if let Some(hook) = hook {
            hook(Stage::BeforeLoad);
        }
        store.load()
    })
    .map_err(|()| Error::Unavailable)?
    .map_err(Error::Config)?;
    match request.command {
        Command::View(view) => {
            let sources = sources(snapshot.loaded(), workspace).map_err(Error::Config)?;
            let reload = install(permissions, permit, &sources);
            Ok(Receipt::View {
                view,
                sources,
                reload,
            })
        }
        Command::Mutate { scope, mutation } => {
            let commit = contain(|| {
                futures_executor::block_on(
                    store.apply_permission_mutation(&snapshot, workspace, scope, &mutation),
                )
            })
            .map_err(|()| Error::Ambiguous)?
            .map_err(Error::Config)?;
            if matches!(
                mutation,
                crate::NativeConfiguredPermissionMutation::Remove { .. }
            ) && commit.outcome == crate::NativeConfiguredPermissionMutationOutcome::Unchanged
            {
                return Ok(Receipt::Mutation {
                    scope,
                    mutation,
                    outcome: commit.outcome,
                    sources: None,
                    reload: None,
                });
            }
            // The commit's projected config is not a fresh effective-source
            // observation. Another writer may publish after our successful CAS.
            let refreshed = contain(|| {
                #[cfg(test)]
                if let Some(hook) = hook {
                    hook(Stage::AfterCommit);
                }
                store.load()
            })
            .map_err(|()| ReloadError::Unavailable)
            .and_then(|result| result.map_err(ReloadError::Config))
            .and_then(|snapshot| {
                sources(snapshot.loaded(), workspace).map_err(ReloadError::Config)
            });
            let (sources, reload) = match refreshed {
                Ok(sources) => {
                    let reload = install(permissions, permit, &sources);
                    (Some(sources), reload)
                }
                Err(error) => (None, Err(error)),
            };
            Ok(Receipt::Mutation {
                scope,
                mutation,
                outcome: commit.outcome,
                sources,
                reload: Some(reload),
            })
        }
    }
}

fn sources(
    loaded: &crate::LoadedNativeConfig,
    workspace: &Path,
) -> Result<Sources, crate::NativeUserConfigError> {
    let sources = loaded
        .config()
        .permission_sources(workspace)
        .map_err(crate::NativeUserConfigError::InvalidConfig)?;
    Ok(Sources {
        user: sources.user().clone(),
        local: sources.local().cloned(),
    })
}

fn install(
    permissions: &NativePermissionSession,
    permit: &LifecyclePermit,
    sources: &Sources,
) -> Result<(), ReloadError> {
    contain(|| {
        permissions.set_configured_rules_admitted(permit, Arc::new(sources.effective().clone()))
    })
    .map_err(|()| ReloadError::Unavailable)?
    .map_err(|_| ReloadError::Permission)
}

fn contain<T>(operation: impl FnOnce() -> T) -> Result<T, ()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        std::mem::forget(payload);
    })
}
