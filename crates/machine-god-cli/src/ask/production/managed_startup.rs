//! Thin startup selection; native retains journals, preparation and cleanup.

use super::{AskSignals, wall_clock_ms};
use machine_god_native::{
    NativeInteractiveInitialSession, NativeInteractivePromptInbox, NativeInteractiveSession,
    NativeManagedAgents, NativeManagedInteractiveStartup, NativeReferenceHost,
    NativeReferenceHostManagedOptions, NativeSessionOrigin, TokioWebSearchRuntime,
};
use std::{future::poll_fn, path::Path, sync::Arc};

pub(super) fn options(inbox: &NativeInteractivePromptInbox) -> NativeReferenceHostManagedOptions {
    base_options().with_prompt_inbox(inbox)
}

pub(super) fn base_options() -> NativeReferenceHostManagedOptions {
    NativeReferenceHostManagedOptions::new(Arc::new(machine_god_native::mcp::clock::TokioMcpClock))
}

/// Runs on the existing constructor worker. Always returns the original host so
/// even failed acquisition enters the outer host/input/worker settlement path.
pub(super) fn prepare(
    mut host: NativeReferenceHost,
    state: &Path,
    runtime: &TokioWebSearchRuntime,
) -> (NativeReferenceHost, Result<Option<NativeManagedAgents>, ()>) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !host.managed_agents_selected() {
            return Ok(None);
        }
        let directory = rustix::fs::open(
            state,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| ())?;
        let preferences = host.loaded_config().config().model_preferences();
        runtime
            .block_on(host.open_workspace_managed_agents(
                directory,
                preferences,
                NativeSessionOrigin::Cli,
            ))
            .map(Some)
            .map_err(|_| ())
    }));
    let result = match result {
        Ok(result) => result,
        Err(payload) => {
            std::mem::forget(payload);
            Err(())
        }
    };
    (host, result)
}

pub(super) async fn open(
    mut startup: NativeManagedInteractiveStartup,
    initial: NativeInteractiveInitialSession,
    signals: &mut AskSignals,
) -> Result<Option<NativeInteractiveSession>, ()> {
    let selection =
        wall_clock_ms().and_then(|now| startup.request_open(initial, now).map_err(|_| ()));
    if selection.is_err() {
        startup.request_shutdown();
    }
    let result = poll_fn(|cx| {
        if signals.first_observed.is_some() || signals.poll_signal(cx).is_ready() {
            startup.request_shutdown();
        }
        startup.poll_open(cx, wall_clock_ms().unwrap_or(0))
    })
    .await;
    match result {
        Ok(owner) if selection.is_ok() => Ok(owner),
        _ => {
            startup.request_shutdown();
            let _ = poll_fn(|cx| startup.poll_open(cx, wall_clock_ms().unwrap_or(0))).await;
            Err(())
        }
    }
}
