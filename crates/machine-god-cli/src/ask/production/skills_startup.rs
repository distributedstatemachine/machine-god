//! Blocking CLI startup joins native selection and optional initial discovery.

use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeEnvironment, NativeOwnedWorkerScope, NativeReferenceHostTerminalOptions,
    NativeSkillSnapshot, NativeSkillsCommand, NativeSkillsService, NativeSkillsServiceResult,
    PreparedNativeRoots, TokioWebSearchRuntime, prepare_native_skills,
};
use std::{ffi::OsString, sync::Arc};

pub(super) struct Prepared {
    pub(super) roots: PreparedNativeRoots,
    pub(super) service: Arc<NativeSkillsService>,
    pub(super) snapshot: Option<Arc<NativeSkillSnapshot>>,
}

/// Derive native root selection from the same snapshot supplied to terminal
/// and Git selection. This helper never rereads the process environment.
pub(super) fn environment(values: &[(OsString, OsString)]) -> NativeEnvironment {
    let selected = |key: &str| {
        values
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    NativeEnvironment::new(
        selected("XDG_CONFIG_HOME"),
        selected("XDG_STATE_HOME"),
        selected("HOME"),
    )
}

/// The caller latches setup signals before entering. All effects finish before
/// this returns, including error/panic paths, and before a full host is acquired.
pub(super) fn prepare(
    runtime: &TokioWebSearchRuntime,
    roots: PreparedNativeRoots,
    environment: NativeEnvironment,
    terminal: NativeReferenceHostTerminalOptions,
    discover: bool,
) -> Result<Prepared, ()> {
    let workers = NativeOwnedWorkerScope::new();
    let completion = workers.completion();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(async {
            let (roots, service) = prepare_native_skills(
                roots,
                environment,
                terminal,
                workers.clone(),
                CancellationToken::new(),
            )
            .await
            .map_err(|_| ())?;
            let snapshot = if discover {
                let selected = Arc::clone(&service);
                let cwd = roots.workspace_root().to_owned();
                let result = workers
                    .run(move || {
                        selected.execute(NativeSkillsCommand::List, &cwd, &CancellationToken::new())
                    })
                    .await
                    .map_err(|_| ())?
                    .map_err(|_| ())?;
                let NativeSkillsServiceResult::Catalog(view) = result else {
                    return Err(());
                };
                Some(Arc::new(view.snapshot))
            } else {
                None
            };
            Ok(Prepared {
                roots,
                service,
                snapshot,
            })
        })
    }));
    workers.close();
    completion.wait_on_worker().map_err(|_| ())?;
    result.map_err(std::mem::forget)?
}
