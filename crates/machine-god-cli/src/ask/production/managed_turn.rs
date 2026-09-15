//! One-shot presentation adapter; the native owner retains all product state.

use super::{
    AskCommandOutcome, AskSignal, AskSignalControlSender, AskSignals, OutputBridge,
    SessionSelection, SignalSource, TurnDriveResult, drive_turn_stream, managed_startup,
    wall_clock_ms,
};
use futures_core::Stream;
use machine_god_core::TurnEvent;
use machine_god_native::{
    NativeInteractiveInitialSession, NativeInteractiveOutcome, NativeInteractiveSession,
    NativeInteractiveSessionOptions, NativeManagedAgents, NativeManagedInteractiveStartup,
    NativeModelCatalog, NativeReferenceHost, NativeResumeTarget,
};
use std::{
    cell::RefCell,
    future::poll_fn,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

#[cfg(test)]
mod tests;

pub(super) struct Setup {
    pub selection: SessionSelection,
    pub prompt: String,
    pub workspace: PathBuf,
    pub catalog: Option<Arc<NativeModelCatalog>>,
}

pub(super) async fn execute(
    host: Arc<NativeReferenceHost>,
    mut agents: NativeManagedAgents,
    setup: Setup,
    output: OutputBridge,
    signals: &mut AskSignals,
    control: &AskSignalControlSender,
) -> Result<TurnDriveResult, ()> {
    let options = NativeInteractiveSessionOptions::new(
        setup.workspace,
        host.loaded_config().config().model_preferences(),
    );
    let Ok(mut options) = options else {
        settle_agents(&mut agents, signals).await?;
        return Err(());
    };
    options = options.with_required_mcp_startup();
    if let Some(catalog) = setup.catalog {
        options = options.with_catalog(catalog);
    }
    let startup = match NativeManagedInteractiveStartup::new(host, options, agents) {
        Ok(startup) => startup,
        Err((_, mut agents)) => {
            settle_agents(&mut agents, signals).await?;
            return Err(());
        }
    };
    let initial = match setup.selection {
        SessionSelection::CreateGenerated => NativeInteractiveInitialSession::Fresh,
        SessionSelection::Resume(id) => {
            NativeInteractiveInitialSession::Resume(NativeResumeTarget::Exact(id))
        }
    };
    let Some(mut owner) = managed_startup::open(startup, initial, signals).await? else {
        return Ok(TurnDriveResult {
            outcome: signals
                .first_observed
                .map_or(AskCommandOutcome::OperationalFailure, AskSignal::outcome),
            stalled_output_after_signal: false,
        });
    };
    // Even enqueue/control failure retains the actual native session through
    // shutdown. No provider, child or cleanup future is detached on this path.
    let admitted = owner.enqueue(setup.prompt.into()).map_err(|_| ());
    if admitted.and_then(|_| control.activate_turn()).is_ok() {
        drive(owner, signals, output).await
    } else {
        settle(&mut owner, signals).await?;
        Err(())
    }
}

async fn settle_agents(
    agents: &mut NativeManagedAgents,
    signals: &mut AskSignals,
) -> Result<(), ()> {
    agents.request_shutdown();
    poll_fn(|cx| {
        let _ = signals.poll_signal(cx);
        agents.poll_shutdown(cx, wall_clock_ms().unwrap_or(0))
    })
    .await
    .map_err(|_| ())
}

async fn settle(owner: &mut NativeInteractiveSession, signals: &mut AskSignals) -> Result<(), ()> {
    owner.request_shutdown();
    poll_fn(|cx| {
        let _ = signals.poll_signal(cx);
        let _ = owner.poll_progress(cx, wall_clock_ms().unwrap_or(0));
        let _ = owner.take_presentation();
        let _ = owner.take_outcome();
        if owner.shutdown_error().is_some() {
            Poll::Ready(Err(()))
        } else if owner.is_closed() {
            // A retained operational error is not a cleanup receipt. Preserve
            // the failure, but first finish the owner's actual shutdown.
            Poll::Ready(if owner.managed_error().is_some() {
                Err(())
            } else {
                Ok(())
            })
        } else {
            Poll::Pending
        }
    })
    .await
}

async fn drive(
    owner: NativeInteractiveSession,
    signals: &mut AskSignals,
    output: OutputBridge,
) -> Result<TurnDriveResult, ()> {
    let owner = RefCell::new(owner);
    let mut events = Events {
        owner: &owner,
        ended: false,
    };
    let mut progress = Progress {
        owner: &owner,
        signals,
        signalled: false,
    };
    let result = drive_turn_stream(
        &mut events,
        || owner.borrow_mut().request_shutdown(),
        &mut progress,
        output,
    )
    .await;
    let mut owner = owner.into_inner();
    settle(&mut owner, signals).await?;
    Ok(result)
}

struct Events<'a> {
    owner: &'a RefCell<NativeInteractiveSession>,
    ended: bool,
}

impl Stream for Events<'_> {
    type Item = Result<TurnEvent, ()>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.ended {
            return Poll::Ready(None);
        }
        let selected = self.owner;
        let mut owner = selected.borrow_mut();
        let _ = owner.poll_progress(cx, wall_clock_ms().unwrap_or(0));
        if owner.managed_error().is_some() || owner.shutdown_error().is_some() {
            self.ended = true;
            return Poll::Ready(Some(Err(())));
        }
        if let Some(event) = owner.take_presentation() {
            return Poll::Ready(Some(Ok(event.payload)));
        }
        if let Some(outcome) = owner.take_outcome() {
            self.ended = true;
            return match outcome {
                NativeInteractiveOutcome::Turn(event) => {
                    Poll::Ready(Some(event.map(|event| event.payload).map_err(|_| ())))
                }
                NativeInteractiveOutcome::Shutdown => Poll::Ready(None),
                _ => Poll::Ready(Some(Err(()))),
            };
        }
        if owner.is_closed() {
            self.ended = true;
            return Poll::Ready(None);
        }
        Poll::Pending
    }
}

/// The output driver polls this lane while awaiting write/flush ACKs. Only the
/// native bounded presentation slot can fill; hidden child and cancellation
/// progress does not depend on draining another foreground event. `RefCell`
/// borrows end within each poll, never across an await or another callback.
struct Progress<'a> {
    owner: &'a RefCell<NativeInteractiveSession>,
    signals: &'a mut AskSignals,
    signalled: bool,
}

impl SignalSource for Progress<'_> {
    fn poll_signal(&mut self, cx: &mut Context<'_>) -> Poll<AskSignal> {
        if !self.signalled {
            let signal = self
                .signals
                .first_observed
                .map_or_else(|| self.signals.poll_signal(cx), Poll::Ready);
            if signal.is_ready() {
                self.signalled = true;
                return signal;
            }
        }
        let mut owner = self.owner.borrow_mut();
        let _ = owner.poll_progress(cx, wall_clock_ms().unwrap_or(0));
        if !self.signalled && (owner.managed_error().is_some() || owner.shutdown_error().is_some())
        {
            self.signalled = true;
            return Poll::Ready(AskSignal::ControlFailed);
        }
        Poll::Pending
    }
}
