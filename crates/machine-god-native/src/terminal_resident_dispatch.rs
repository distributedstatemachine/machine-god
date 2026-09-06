//! Short owner-thread dispatch for already-authorized resident actions.
//! Startup, catalog discovery and probe effects belong to the enclosing host.

use machine_god_core::{
    BackgroundOutputOwner, MAX_TERMINAL_ACTION_OUTPUT_BYTES, TerminalActionRequest,
    TerminalActionResult, TerminalActorRole, TerminalAllowedControls, TerminalClosePolicy,
    TerminalCursor, TerminalDimensions, TerminalLifecycle, TerminalMonitorId, TerminalRawRange,
    TerminalReturnCondition, TerminalReturnOutcome, TerminalSessionFacts, TerminalSessionId,
    TerminalSignal, TerminalWriteLeaseIntent,
};

use crate::terminal_catalog::owner_name;
use crate::terminal_history::TerminalHistoryError;
use crate::terminal_input::TerminalWriterId;
use crate::terminal_journal::TerminalJournalPage;
use crate::terminal_monitor::{TerminalMonitorActivation, TerminalWaitOutcome};
use crate::terminal_owner::TerminalOwnerContext;
use crate::terminal_profile::{TerminalJournalPersistence, TerminalProfileMutationContext};
use crate::terminal_registry::{TerminalRegistry, TerminalRegistryError};
use crate::terminal_session::{TerminalSessionBackend, TerminalSessionError};
use crate::terminal_wait::{
    TerminalWaitError, TerminalWaitFuture, TerminalWaitHistory, TerminalWaitIdentity,
    TerminalWaitReceipt,
};
use crate::terminal_write_completion::{
    TerminalWriteError, TerminalWriteFuture, TerminalWriteIdentity, TerminalWriteReceipt,
};

/// Host-supplied identity and descriptive controls, never decoded from a request.
/// The enclosing host has already authorized the particular action. Revocation
/// additionally requires separately established close authority.
#[derive(Clone)]
pub(crate) struct TerminalResidentAuthority {
    pub(crate) owner: BackgroundOutputOwner,
    pub(crate) actor: TerminalActorRole,
    pub(crate) writer: TerminalWriterId,
    pub(crate) controls: TerminalAllowedControls,
    pub(crate) revoke_authorized: bool,
}

/// A successfully committed effect whose later facts projection was unavailable.
/// Keeping this distinct from admission failure prevents automatic resubmission.
#[derive(Debug)]
pub(crate) enum TerminalResidentEffect {
    Resize(TerminalDimensions),
    Signal(TerminalSignal),
    Close(TerminalClosePolicy),
    Monitor(TerminalMonitorId),
}

#[derive(Debug)]
pub(crate) enum TerminalResidentError {
    Invalid,
    Unsupported,
    Cancelled,
    UnauthorizedRevoke,
    /// Session errors may follow a native effect (for example failed final
    /// checkpoint publication). This is never an automatic-retry guarantee.
    Registry(TerminalRegistryError),
    Wait(TerminalWaitError),
    Write(TerminalWriteError),
    Committed {
        effect: TerminalResidentEffect,
        error: TerminalRegistryError,
    },
}
impl From<TerminalRegistryError> for TerminalResidentError {
    fn from(error: TerminalRegistryError) -> Self {
        Self::Registry(error)
    }
}
type Result<T> = std::result::Result<T, TerminalResidentError>;

/// Aggregate journal-sized pages up to the public one-MiB bound, but never
/// across a segment: the public raw-range contract has one segment identity.
/// Shared by resident and cold-history hosts. The reader has no mutation effects.
pub(crate) fn read_terminal_action_page(
    cursor: &TerminalCursor,
    mut read: impl FnMut(
        &TerminalCursor,
        usize,
    ) -> std::result::Result<TerminalJournalPage, TerminalRegistryError>,
) -> std::result::Result<TerminalJournalPage, TerminalRegistryError> {
    const JOURNAL_PAGE_BYTES: usize = 64 * 1024;
    let mut page = read(cursor, JOURNAL_PAGE_BYTES)?;
    check_read_page(&page, cursor, JOURNAL_PAGE_BYTES)?;
    while !page.bytes.is_empty()
        && page.bytes.len() < MAX_TERMINAL_ACTION_OUTPUT_BYTES
        && page.next < page.latest
    {
        let remaining = MAX_TERMINAL_ACTION_OUTPUT_BYTES - page.bytes.len();
        let maximum = remaining.min(JOURNAL_PAGE_BYTES);
        let next = read(&page.next, maximum)?;
        check_read_page(&next, &page.next, maximum)?;
        if next.next.segment() != page.next.segment() {
            break;
        }
        if next.bytes.is_empty()
            || next.gap.is_some()
            || next.latest != page.latest
            || next.earliest != page.earliest
            || next.next.offset().checked_sub(next.bytes.len() as u64) != Some(page.next.offset())
        {
            return Err(TerminalRegistryError::Invalid);
        }
        page.bytes.extend_from_slice(&next.bytes);
        page.next = next.next;
    }
    Ok(page)
}

fn check_read_page(
    page: &TerminalJournalPage,
    requested: &TerminalCursor,
    maximum: usize,
) -> std::result::Result<(), TerminalRegistryError> {
    if page.bytes.len() > maximum
        || &page.next < requested
        || page.next > page.latest
        || page.earliest > page.latest
        || page.next.offset() < page.bytes.len() as u64
    {
        return Err(TerminalRegistryError::Invalid);
    }
    Ok(())
}

pub(crate) fn resident_read_result(
    mut session: TerminalSessionFacts,
    page: TerminalJournalPage,
) -> std::result::Result<TerminalActionResult, TerminalRegistryError> {
    session.raw_gap = page.gap;
    let raw_range = if page.bytes.is_empty() {
        None
    } else {
        Some(TerminalRawRange {
            start: TerminalCursor::new(
                page.next.segment(),
                page.next
                    .offset()
                    .checked_sub(page.bytes.len() as u64)
                    .ok_or(TerminalRegistryError::Invalid)?,
            )
            .map_err(|_| TerminalRegistryError::Invalid)?,
            end: page.next,
        })
    };
    let result = TerminalActionResult::Read {
        session,
        output: page.bytes,
        raw_range,
    };
    result
        .validate()
        .map_err(|_| TerminalRegistryError::Invalid)?;
    Ok(result)
}

pub(crate) enum TerminalResidentDispatch {
    Ready(TerminalActionResult),
    /// Facts are a bounded admission snapshot, not a final lifecycle assertion.
    /// Retain them with the receipt if shutdown refuses a refresh request.
    Wait {
        admitted: TerminalSessionFacts,
        future: TerminalWaitFuture,
    },
    Write {
        admitted: TerminalSessionFacts,
        future: TerminalWriteFuture,
    },
}

/// Acquires only this resident's exact profile namespace. The guard cannot
/// escape to a reply, observer or asynchronous receipt future.
fn with_persistence<B: TerminalSessionBackend, S, T>(
    context: &mut TerminalOwnerContext<'_, B, S>,
    authority: &TerminalResidentAuthority,
    id: &TerminalSessionId,
    operation: impl FnOnce(
        &mut TerminalRegistry<B>,
        &mut dyn TerminalJournalPersistence,
    ) -> std::result::Result<T, TerminalRegistryError>,
) -> std::result::Result<T, TerminalRegistryError> {
    context.registry.authorize_resident(&authority.owner, id)?;
    let namespace = owner_name(context.registry.workspace(), &authority.owner);
    let mut transaction = context.store.transaction().map_err(|error| {
        TerminalRegistryError::Session(TerminalSessionError::History(
            TerminalHistoryError::Profile(error.into()),
        ))
    })?;
    let mut persistence =
        TerminalProfileMutationContext::new(&mut transaction, *context.budget, &namespace);
    operation(context.registry, &mut persistence)
}

/// Also usable after an owned receipt completes: intentionally does not reject
/// caller cancellation, which must never erase an already-committed effect.
pub(crate) fn resident_facts<B: TerminalSessionBackend, S>(
    context: &mut TerminalOwnerContext<'_, B, S>,
    authority: &TerminalResidentAuthority,
    id: &TerminalSessionId,
) -> std::result::Result<TerminalSessionFacts, TerminalRegistryError> {
    with_persistence(context, authority, id, |registry, persistence| {
        registry.project_facts_with(
            persistence,
            &authority.owner,
            id,
            authority.actor,
            &authority.controls,
        )
    })
}

/// No action acquires native authority here. In particular allowed-controls
/// booleans and persisted identifiers are not authorization capabilities.
/// Monitor activation evidence must come from separately authorized host work.
/// Probe descriptions remain queued in the session until the ordinary owner
/// pump returns them to its observer; this function never consumes that queue.
#[allow(
    clippy::too_many_lines,
    reason = "closed nine-action routing keeps effect and receipt handling adjacent"
)]
pub(crate) fn dispatch_resident<B: TerminalSessionBackend, S>(
    mut context: TerminalOwnerContext<'_, B, S>,
    authority: &TerminalResidentAuthority,
    request: &TerminalActionRequest,
    monitor_activation: TerminalMonitorActivation,
) -> Result<TerminalResidentDispatch> {
    request
        .validate()
        .map_err(|_| TerminalResidentError::Invalid)?;
    if context.cancellation.is_cancelled() {
        return Err(TerminalResidentError::Cancelled);
    }
    let cancellation = context.cancellation;
    let id = request
        .session_id()
        .ok_or(TerminalResidentError::Unsupported)?;
    context.registry.authorize_resident(&authority.owner, id)?;
    if let TerminalActionRequest::Write { request, .. } = &request
        && request.lease == TerminalWriteLeaseIntent::Revoke
        && !authority.revoke_authorized
    {
        return Err(TerminalResidentError::UnauthorizedRevoke);
    }

    let result = match request {
        TerminalActionRequest::Read { cursor, .. } => {
            let page = read_terminal_action_page(cursor, |cursor, maximum| {
                context.registry.read(&authority.owner, id, cursor, maximum)
            })?;
            resident_read_result(resident_facts(&mut context, authority, id)?, page)?
        }
        TerminalActionRequest::Screen { .. } => {
            let snapshot = context.registry.screen(&authority.owner, id)?;
            TerminalActionResult::Screen {
                session: resident_facts(&mut context, authority, id)?,
                snapshot,
            }
        }
        TerminalActionRequest::Inspect { events, .. } => {
            with_persistence(&mut context, authority, id, |registry, persistence| {
                before_effect(cancellation)?;
                registry.inspect_result_with(
                    persistence,
                    &authority.owner,
                    id,
                    authority.actor,
                    events,
                    &authority.controls,
                )
            })?
        }
        TerminalActionRequest::Write { request, .. } => {
            let admitted = resident_facts(&mut context, authority, id)?;
            let future = context
                .writes
                .submit(
                    TerminalWriteIdentity {
                        owner: authority.owner.clone(),
                        session: id.clone(),
                        actor: authority.actor,
                        writer: authority.writer,
                    },
                    context.cancellation,
                    || {
                        context
                            .registry
                            .mutate_with_profile(
                                context.store,
                                context.budget,
                                &authority.owner,
                                id,
                                |session, persistence| {
                                    session.write_completion_with(
                                        persistence,
                                        &authority.owner,
                                        authority.actor,
                                        authority.writer,
                                        request,
                                        context.cancellation.is_cancelled(),
                                    )
                                },
                            )
                            .map(|(input, error)| TerminalWriteReceipt {
                                input,
                                publication_error: error.map(TerminalRegistryError::Session),
                            })
                    },
                )
                .map_err(TerminalResidentError::Write)?;
            return Ok(TerminalResidentDispatch::Write { admitted, future });
        }
        TerminalActionRequest::Wait { request, .. } => {
            let admitted = resident_facts(&mut context, authority, id)?;
            let (mut observation, last_output, _) =
                context.registry.wait_observation(&authority.owner, id)?;
            observation.now_ms = context.now_ms;
            if matches!(request.condition, TerminalReturnCondition::Match { .. }) {
                // Admission reads no large history; the owner catches up from the
                // retained beginning one bounded page per scheduler turn.
                observation.cursor = context
                    .registry
                    .read(
                        &authority.owner,
                        id,
                        &TerminalCursor::new(1, 0).expect("valid origin"),
                        1,
                    )?
                    .earliest;
            }
            let (reservation, future) = context
                .waits
                .register(
                    TerminalWaitIdentity {
                        owner: authority.owner.clone(),
                        session: id.clone(),
                        actor: authority.actor,
                        writer: authority.writer,
                    },
                    request.clone(),
                    &observation,
                    last_output,
                    TerminalWaitHistory::default(),
                    context.cancellation.clone(),
                )
                .map_err(TerminalResidentError::Wait)?;
            // Reserve before the persistent effect. Rejected admission must not
            // later cancel another wait or a pre-existing input lease.
            if matches!(
                observation.lifecycle,
                TerminalLifecycle::Starting | TerminalLifecycle::Running
            ) {
                let admission = context.registry.mutate_with_profile(
                    context.store,
                    context.budget,
                    &authority.owner,
                    id,
                    |session, persistence| {
                        if context.cancellation.is_cancelled() {
                            return Err(TerminalSessionError::Input(
                                crate::terminal_input::TerminalInputError::Cancelled,
                            ));
                        }
                        session.begin_attention_with(
                            persistence,
                            &authority.owner,
                            authority.actor,
                            authority.writer,
                            context.now_ms,
                        )
                    },
                );
                if let Err(error) = admission {
                    context.waits.withdraw_unadmitted(reservation);
                    return Err(error.into());
                }
            }
            return Ok(TerminalResidentDispatch::Wait { admitted, future });
        }
        TerminalActionRequest::Resize { dimensions, .. } => {
            context.registry.mutate_with_profile(
                context.store,
                context.budget,
                &authority.owner,
                id,
                |session, persistence| {
                    before_effect(cancellation)?;
                    session.resize_with(persistence, &authority.owner, dimensions, context.now_ms)
                },
            )?;
            let session = resident_facts(&mut context, authority, id).map_err(|error| {
                TerminalResidentError::Committed {
                    effect: TerminalResidentEffect::Resize(dimensions.clone()),
                    error,
                }
            })?;
            TerminalActionResult::Resize {
                session,
                dimensions: dimensions.clone(),
            }
        }
        TerminalActionRequest::Signal { signal, .. } => {
            context.registry.mutate_with_profile(
                context.store,
                context.budget,
                &authority.owner,
                id,
                |session, persistence| {
                    before_effect(cancellation)?;
                    session.signal_with(persistence, &authority.owner, *signal, context.now_ms)
                },
            )?;
            let session = resident_facts(&mut context, authority, id).map_err(|error| {
                TerminalResidentError::Committed {
                    effect: TerminalResidentEffect::Signal(*signal),
                    error,
                }
            })?;
            TerminalActionResult::Signal {
                session,
                signal: *signal,
            }
        }
        TerminalActionRequest::Close { policy, .. } => {
            let now_ms = context.now_ms;
            with_persistence(&mut context, authority, id, |registry, persistence| {
                before_effect(cancellation)?;
                registry.close_with(persistence, &authority.owner, id, *policy, now_ms)
            })?;
            let mut session = resident_facts(&mut context, authority, id).map_err(|error| {
                TerminalResidentError::Committed {
                    effect: TerminalResidentEffect::Close(*policy),
                    error,
                }
            })?;
            session.next_actions = machine_god_core::TerminalAllowedControls::default();
            TerminalActionResult::Close {
                session,
                policy: *policy,
            }
        }
        TerminalActionRequest::Monitor { operation, .. } => {
            let mutation = context.registry.mutate_with_profile(
                context.store,
                context.budget,
                &authority.owner,
                id,
                |session, persistence| {
                    before_effect(cancellation)?;
                    session.monitor_with(
                        persistence,
                        &authority.owner,
                        operation.clone(),
                        monitor_activation,
                        context.now_ms,
                    )
                },
            )?;
            let session = resident_facts(&mut context, authority, id).map_err(|error| {
                TerminalResidentError::Committed {
                    effect: TerminalResidentEffect::Monitor(mutation.monitor_id.clone()),
                    error,
                }
            })?;
            TerminalActionResult::Monitor {
                session,
                monitor_id: Some(mutation.monitor_id),
            }
        }
        TerminalActionRequest::Exec { .. }
        | TerminalActionRequest::Start { .. }
        | TerminalActionRequest::List { .. } => return Err(TerminalResidentError::Unsupported),
    };
    result
        .validate_for(request)
        .map_err(|_| TerminalResidentError::Invalid)?;
    Ok(TerminalResidentDispatch::Ready(result))
}

fn before_effect(
    cancellation: &machine_god_core::CancellationToken,
) -> std::result::Result<(), TerminalSessionError> {
    if cancellation.is_cancelled() {
        Err(TerminalSessionError::Input(
            crate::terminal_input::TerminalInputError::Cancelled,
        ))
    } else {
        Ok(())
    }
}

/// Preserve the native receipt separately even when a public projection cannot
/// represent loss or failed attention publication. Never replace those facts
/// with a successful public wait result.
pub(crate) fn resident_wait_result(
    session: TerminalSessionFacts,
    receipt: TerminalWaitReceipt,
) -> std::result::Result<TerminalActionResult, TerminalWaitReceipt> {
    if receipt.attention_error.is_some() {
        return Err(receipt);
    }
    let outcome = match receipt.outcome {
        TerminalWaitOutcome::Started => TerminalReturnOutcome::Started {},
        TerminalWaitOutcome::Exited(exit_code) => TerminalReturnOutcome::Exited { exit_code },
        TerminalWaitOutcome::Signaled(signal) => TerminalReturnOutcome::Signal {
            signal: u32::try_from(signal).map_err(|_| receipt)?,
        },
        TerminalWaitOutcome::ConditionMet => TerminalReturnOutcome::ConditionMet {},
        TerminalWaitOutcome::SafetyCeiling => TerminalReturnOutcome::SafetyCeiling {},
        TerminalWaitOutcome::Cancelled => TerminalReturnOutcome::Cancelled {},
        TerminalWaitOutcome::Lost => return Err(receipt),
    };
    let result = TerminalActionResult::Wait { session, outcome };
    result.validate().map_err(|_| receipt)?;
    Ok(result)
}

/// Accepted bytes are retained even when the independent publication failed.
/// The host must expose that failure rather than claim successful durability.
pub(crate) fn resident_write_result(
    session: TerminalSessionFacts,
    receipt: TerminalWriteReceipt,
) -> std::result::Result<TerminalActionResult, TerminalWriteReceipt> {
    if receipt.publication_error.is_some()
        || matches!(
            receipt.input.progress,
            crate::terminal_input::TerminalInputProgress::Pending
                | crate::terminal_input::TerminalInputProgress::Failed
        )
    {
        return Err(receipt);
    }
    let result = TerminalActionResult::Write {
        session,
        accepted_bytes: u32::try_from(receipt.input.accepted_bytes).map_err(|_| receipt)?,
    };
    result.validate().map_err(|_| receipt)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
    use crate::terminal_history::TerminalHistory;
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_profile::{TerminalProfileBudget, TerminalProfileLimits};
    use crate::terminal_profile_store::TerminalProfileStore;
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_session::TerminalSession;
    use crate::terminal_session_record::test_metadata;
    use crate::terminal_wait::TerminalWaitCoordinator;
    use crate::terminal_write_completion::TerminalWriteCoordinator;
    use machine_god_core::{
        CancellationToken, SessionId, SessionIncarnationId, TerminalEventQuery,
        TerminalMonitorCondition, TerminalMonitorDefinition, TerminalMonitorLifetime,
        TerminalMonitorOperation, TerminalNotifySchedule, TerminalWaitRequest,
        TerminalWritePayload, TerminalWriteRequest,
    };
    use std::collections::VecDeque;
    use std::future::Future;
    use std::num::NonZeroU64;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    #[derive(Default)]
    struct NativeState {
        output: VecDeque<Vec<u8>>,
        written: Vec<u8>,
        write_limit: Option<usize>,
        signals: usize,
        resizes: usize,
        closes: usize,
        cancel_on_signal: Option<CancellationToken>,
    }
    struct Backend(Arc<Mutex<NativeState>>);
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            let bytes = self
                .0
                .lock()
                .unwrap()
                .output
                .pop_front()
                .unwrap_or_default();
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(TerminalPtyRead {
                bytes_read: bytes.len(),
                closed: false,
            })
        }
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            let mut state = self.0.lock().unwrap();
            let count = bytes.len().min(state.write_limit.unwrap_or(usize::MAX));
            state.written.extend_from_slice(&bytes[..count]);
            Ok(BackgroundInputReceipt::new(
                count,
                false,
                if count == bytes.len() {
                    BackgroundInputStatus::Written
                } else {
                    BackgroundInputStatus::Backpressure
                },
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(TerminalPtyStatus::Running)
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            self.0.lock().unwrap().resizes += 1;
            Ok(())
        }
        fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
            let mut state = self.0.lock().unwrap();
            state.signals += 1;
            if let Some(cancellation) = &state.cancel_on_signal {
                cancellation.cancel();
            }
            Ok(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            output: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            let mut state = self.0.lock().unwrap();
            state.closes += 1;
            for bytes in state.output.drain(..) {
                output(&bytes);
            }
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }
    struct Fixture {
        registry: TerminalRegistry<Backend>,
        store: TerminalProfileStore,
        budget: TerminalProfileBudget,
        waits: TerminalWaitCoordinator,
        writes: TerminalWriteCoordinator,
        state: Arc<Mutex<NativeState>>,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-resident-dispatch-{:032x}",
                u128::from_le_bytes(random)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            let fd = rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )
            .unwrap();
            let store = TerminalProfileStore::prepare(fd).unwrap();
            let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            let authority = authority();
            let id = id();
            let state = Arc::new(Mutex::new(NativeState::default()));
            let mut transaction = store.transaction().unwrap();
            let mut catalog = transaction
                .prepare_catalog("/workspace".into(), authority.owner.clone())
                .unwrap();
            drop(transaction.create_session(&mut catalog, &id).unwrap());
            let completion = budget
                .create_journal(
                    &mut transaction,
                    catalog.namespace_key(),
                    &id,
                    TerminalJournalLimits::default(),
                )
                .unwrap();
            completion.accounting.unwrap();
            let mut persistence = TerminalProfileMutationContext::new(
                &mut transaction,
                budget,
                catalog.namespace_key(),
            );
            let history = TerminalHistory::create_with(
                &mut persistence,
                completion.operation.unwrap(),
                &TerminalDimensions::new(3, 20).unwrap(),
            )
            .unwrap();
            let mut session = TerminalSession::new_with(
                &mut persistence,
                Backend(Arc::clone(&state)),
                history,
                authority.owner.clone(),
                id.clone(),
                test_metadata(),
                0,
            )
            .unwrap();
            session.shell_ready_with(&mut persistence, 0).unwrap();
            let mut registry = TerminalRegistry::new("/workspace".into()).unwrap();
            registry.start(authority.owner, id, || Ok(session)).unwrap();
            drop(transaction);
            Self {
                registry,
                store,
                budget,
                waits: TerminalWaitCoordinator::new(),
                writes: TerminalWriteCoordinator::new(),
                state,
                path,
            }
        }
        #[allow(
            clippy::needless_pass_by_value,
            reason = "test helper consumes temporary request fixtures"
        )]
        fn dispatch(
            &mut self,
            who: &TerminalResidentAuthority,
            request: TerminalActionRequest,
            cancellation: &CancellationToken,
        ) -> Result<TerminalResidentDispatch> {
            dispatch_resident(
                TerminalOwnerContext {
                    registry: &mut self.registry,
                    store: &self.store,
                    budget: &self.budget,
                    waits: &mut self.waits,
                    writes: &mut self.writes,
                    state: &mut (),
                    now_ms: 0,
                    cancellation,
                },
                who,
                &request,
                TerminalMonitorActivation::default(),
            )
        }
        fn action(&mut self, request: TerminalActionRequest) -> TerminalActionResult {
            let expected = request.clone();
            let TerminalResidentDispatch::Ready(result) = self
                .dispatch(&authority(), request, &CancellationToken::new())
                .unwrap()
            else {
                panic!("immediate reply expected")
            };
            result.validate_for(&expected).unwrap();
            assert!(self.store.transaction().is_ok());
            result
        }
        fn pump(&mut self) {
            self.registry
                .pump_with_profile(&self.store, &self.budget, 0, 16)
                .unwrap();
            self.writes.observe(&self.registry);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.registry
                .shutdown_with_profile(
                    &self.store,
                    &self.budget,
                    self.registry.minimum_time_ms(),
                    TerminalClosePolicy::Force,
                )
                .unwrap();
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn authority() -> TerminalResidentAuthority {
        TerminalResidentAuthority {
            owner: BackgroundOutputOwner::new(
                SessionId::new("owner").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            ),
            actor: TerminalActorRole::Agent,
            writer: TerminalWriterId::new(NonZeroU64::new(1).unwrap()),
            controls: TerminalAllowedControls::default(),
            revoke_authorized: false,
        }
    }
    fn id() -> TerminalSessionId {
        TerminalSessionId::new("terminal").unwrap()
    }
    fn query() -> TerminalEventQuery {
        TerminalEventQuery {
            after_event_id: 0,
            acknowledge_event_id: None,
            max_events: 10,
        }
    }
    fn poll<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
        Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
    }
    fn write(lease: TerminalWriteLeaseIntent, payload: Option<&str>) -> TerminalActionRequest {
        TerminalActionRequest::Write {
            session_id: id(),
            request: TerminalWriteRequest {
                lease,
                payload: payload.map(|text| TerminalWritePayload::Text { text: text.into() }),
            },
        }
    }

    #[test]
    fn routes_read_screen_inspect_resize_signal_and_close() {
        let mut fixture = Fixture::new();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"hello".to_vec());
        fixture.pump();
        let read = fixture.action(TerminalActionRequest::Read {
            session_id: id(),
            cursor: TerminalCursor::new(1, 0).unwrap(),
        });
        let TerminalActionResult::Read {
            output, raw_range, ..
        } = read
        else {
            panic!()
        };
        assert_eq!(output, b"hello");
        assert_eq!(raw_range.unwrap().end.offset(), 5);
        fixture.action(TerminalActionRequest::Screen { session_id: id() });
        fixture.action(TerminalActionRequest::Inspect {
            session_id: id(),
            events: query(),
        });
        fixture.action(TerminalActionRequest::Resize {
            session_id: id(),
            dimensions: TerminalDimensions::new(4, 30).unwrap(),
        });
        fixture.action(TerminalActionRequest::Signal {
            session_id: id(),
            signal: TerminalSignal::Interrupt,
        });
        let closed = fixture.action(TerminalActionRequest::Close {
            session_id: id(),
            policy: TerminalClosePolicy::Force,
        });
        assert_eq!(
            closed.session().unwrap().lifecycle,
            TerminalLifecycle::Closed
        );
        let state = fixture.state.lock().unwrap();
        assert_eq!((state.resizes, state.signals, state.closes), (1, 1, 1));
    }

    #[test]
    fn wrong_owner_and_pre_cancelled_effects_are_rejected() {
        let mut fixture = Fixture::new();
        let mut foreign = authority();
        foreign.owner = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("foreign").unwrap(),
        );
        let request = TerminalActionRequest::Signal {
            session_id: id(),
            signal: TerminalSignal::Interrupt,
        };
        assert!(matches!(
            fixture.dispatch(&foreign, request.clone(), &CancellationToken::new()),
            Err(TerminalResidentError::Registry(
                TerminalRegistryError::NotFound
            ))
        ));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            fixture.dispatch(&authority(), request, &cancelled),
            Err(TerminalResidentError::Cancelled)
        ));
        assert_eq!(fixture.state.lock().unwrap().signals, 0);
        assert!(matches!(
            fixture.dispatch(
                &authority(),
                write(TerminalWriteLeaseIntent::Revoke, None),
                &CancellationToken::new()
            ),
            Err(TerminalResidentError::UnauthorizedRevoke)
        ));
    }

    #[test]
    fn cancellation_after_signal_does_not_erase_commit() {
        let mut fixture = Fixture::new();
        let cancellation = CancellationToken::new();
        fixture.state.lock().unwrap().cancel_on_signal = Some(cancellation.clone());
        assert!(matches!(
            fixture.dispatch(
                &authority(),
                TerminalActionRequest::Signal {
                    session_id: id(),
                    signal: TerminalSignal::Interrupt
                },
                &cancellation
            ),
            Ok(TerminalResidentDispatch::Ready(
                TerminalActionResult::Signal { .. }
            ))
        ));
        assert!(cancellation.is_cancelled());
        assert_eq!(fixture.state.lock().unwrap().signals, 1);
    }

    #[test]
    fn pending_write_keeps_exact_bytes_after_cancel_and_close() {
        for close in [false, true] {
            let mut fixture = Fixture::new();
            let TerminalResidentDispatch::Write {
                mut future,
                admitted,
            } = fixture
                .dispatch(
                    &authority(),
                    write(TerminalWriteLeaseIntent::Acquire, None),
                    &CancellationToken::new(),
                )
                .unwrap()
            else {
                panic!()
            };
            let Poll::Ready(Ok(receipt)) = poll(&mut future) else {
                panic!()
            };
            resident_write_result(admitted, receipt).unwrap();
            fixture.state.lock().unwrap().write_limit = Some(2);
            let cancellation = CancellationToken::new();
            let TerminalResidentDispatch::Write {
                mut future,
                admitted,
            } = fixture
                .dispatch(
                    &authority(),
                    write(TerminalWriteLeaseIntent::Use, Some("abcdefgh")),
                    &cancellation,
                )
                .unwrap()
            else {
                panic!()
            };
            cancellation.cancel();
            if close {
                fixture.action(TerminalActionRequest::Close {
                    session_id: id(),
                    policy: TerminalClosePolicy::Force,
                });
                fixture.writes.observe(&fixture.registry);
            } else {
                fixture.state.lock().unwrap().write_limit = None;
                fixture.pump();
            }
            let Poll::Ready(Ok(receipt)) = poll(&mut future) else {
                panic!("settled receipt expected")
            };
            assert_eq!(
                receipt.input.accepted_bytes,
                fixture.state.lock().unwrap().written.len()
            );
            assert!(receipt.input.accepted_bytes > 0);
            let result = resident_write_result(admitted, receipt).unwrap();
            result.validate().unwrap();
        }
    }

    #[test]
    fn wait_registers_retained_match_cursor_and_completes_with_receipt() {
        let mut fixture = Fixture::new();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"ready".to_vec());
        fixture.pump();
        let TerminalResidentDispatch::Wait {
            admitted,
            mut future,
        } = fixture
            .dispatch(
                &authority(),
                TerminalActionRequest::Wait {
                    session_id: id(),
                    request: TerminalWaitRequest {
                        condition: TerminalReturnCondition::Match {
                            pattern: "ready".into(),
                        },
                        safety_ceiling_ms: 100,
                    },
                },
                &CancellationToken::new(),
            )
            .unwrap()
        else {
            panic!()
        };
        let observations = fixture.waits.observations();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].cursor.offset(), 0);
        let (context, last_output, process) = fixture
            .registry
            .wait_observation(&authority().owner, &id())
            .unwrap();
        fixture
            .waits
            .advance_one(
                &observations[0],
                b"ready",
                &context,
                last_output,
                process,
                true,
            )
            .unwrap();
        for completion in fixture.waits.take_ready() {
            fixture
                .registry
                .finish_attention_with(
                    &fixture.store,
                    &fixture.budget,
                    &authority().owner,
                    &id(),
                    authority().actor,
                    authority().writer,
                    0,
                    false,
                )
                .unwrap();
            completion.publish();
        }
        let Poll::Ready(receipt) = poll(&mut future) else {
            panic!()
        };
        assert_eq!(receipt.outcome, TerminalWaitOutcome::ConditionMet);
        assert!(matches!(
            resident_wait_result(admitted, receipt).unwrap(),
            TerminalActionResult::Wait {
                outcome: TerminalReturnOutcome::ConditionMet {},
                ..
            }
        ));
    }

    #[test]
    fn rejected_wait_admission_does_not_cancel_existing_writer() {
        let mut fixture = Fixture::new();
        let TerminalResidentDispatch::Write { .. } = fixture
            .dispatch(
                &authority(),
                write(TerminalWriteLeaseIntent::Acquire, None),
                &CancellationToken::new(),
            )
            .unwrap()
        else {
            panic!()
        };
        let mut other = authority();
        other.writer = TerminalWriterId::new(NonZeroU64::new(2).unwrap());
        assert!(
            fixture
                .dispatch(
                    &other,
                    TerminalActionRequest::Wait {
                        session_id: id(),
                        request: TerminalWaitRequest {
                            condition: TerminalReturnCondition::Exit {},
                            safety_ceiling_ms: 100
                        }
                    },
                    &CancellationToken::new()
                )
                .is_err()
        );
        assert!(fixture.waits.observations().is_empty());
        assert!(fixture.waits.take_ready().is_empty());
        assert!(
            fixture
                .dispatch(
                    &authority(),
                    write(TerminalWriteLeaseIntent::Use, Some("ok")),
                    &CancellationToken::new()
                )
                .is_ok()
        );
    }

    #[test]
    fn monitor_probe_descriptions_remain_available_to_owner_observer() {
        let mut fixture = Fixture::new();
        fixture.action(TerminalActionRequest::Monitor {
            session_id: id(),
            operation: TerminalMonitorOperation::Add {
                definition: TerminalMonitorDefinition {
                    condition: TerminalMonitorCondition::TcpReady {
                        host: "localhost".into(),
                        port: 80,
                    },
                    notify: TerminalNotifySchedule::OnMatch,
                    lifetime: TerminalMonitorLifetime::UntilMatch,
                    check_schedule: Some(machine_god_core::TerminalSchedule { interval_ms: 10 }),
                },
            },
        });
        let steps = fixture
            .registry
            .pump_with_profile(&fixture.store, &fixture.budget, 10, 16)
            .unwrap();
        assert_eq!(
            steps
                .iter()
                .filter_map(|step| step.result.as_ref().ok())
                .map(|step| step.probes.len())
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn receipt_projection_preserves_shutdown_and_publication_failures() {
        let mut fixture = Fixture::new();
        let facts = fixture
            .action(TerminalActionRequest::Inspect {
                session_id: id(),
                events: query(),
            })
            .session()
            .unwrap()
            .clone();
        let wait = TerminalWaitReceipt {
            outcome: TerminalWaitOutcome::Exited(7),
            attention_error: Some(crate::terminal_wait::TerminalWaitAttentionError::Unavailable),
        };
        assert_eq!(resident_wait_result(facts.clone(), wait).unwrap_err(), wait);
        let write = TerminalWriteReceipt {
            input: crate::terminal_input::TerminalInputReceipt {
                operation_id: NonZeroU64::new(1),
                accepted_bytes: 3,
                encoded_bytes: 8,
                progress: crate::terminal_input::TerminalInputProgress::Closed,
            },
            publication_error: Some(TerminalRegistryError::Closed),
        };
        assert_eq!(
            resident_write_result(facts.clone(), write).unwrap_err(),
            write
        );
        let unavailable = TerminalWriteReceipt {
            publication_error: None,
            input: crate::terminal_input::TerminalInputReceipt {
                progress: crate::terminal_input::TerminalInputProgress::Failed,
                ..write.input
            },
        };
        assert_eq!(
            resident_write_result(facts, unavailable).unwrap_err(),
            unavailable
        );
    }

    #[test]
    fn read_projection_uses_actual_gap_start_and_rejects_discontinuous_pages() {
        let mut fixture = Fixture::new();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"hello".to_vec());
        fixture.pump();
        let facts = fixture
            .action(TerminalActionRequest::Inspect {
                session_id: id(),
                events: query(),
            })
            .session()
            .unwrap()
            .clone();
        let start = TerminalCursor::new(1, 0).unwrap();
        let earliest = TerminalCursor::new(1, 3).unwrap();
        let latest = TerminalCursor::new(1, 5).unwrap();
        let page = TerminalJournalPage {
            bytes: b"lo".to_vec(),
            next: latest.clone(),
            earliest: earliest.clone(),
            latest,
            gap: Some(machine_god_core::TerminalGap::new(start.clone(), earliest.clone()).unwrap()),
        };
        let TerminalActionResult::Read {
            session,
            raw_range,
            output,
        } = resident_read_result(facts, page).unwrap()
        else {
            panic!()
        };
        assert_eq!(output, b"lo");
        assert_eq!(raw_range.unwrap().start, earliest);
        assert!(session.raw_gap.is_some());
        let mut calls = 0;
        assert!(matches!(
            read_terminal_action_page(&start, |_, _| {
                calls += 1;
                Ok(TerminalJournalPage {
                    bytes: vec![0; 2],
                    next: TerminalCursor::new(1, if calls == 1 { 2 } else { 5 }).unwrap(),
                    earliest: start.clone(),
                    latest: TerminalCursor::new(1, 10).unwrap(),
                    gap: None,
                })
            }),
            Err(TerminalRegistryError::Invalid)
        ));
    }

    #[test]
    fn action_read_aggregates_to_one_mib_and_stops_at_segment_boundary() {
        let start = TerminalCursor::new(1, 0).unwrap();
        let mut calls = 0;
        let page = read_terminal_action_page(&start, |cursor, maximum| {
            calls += 1;
            Ok(TerminalJournalPage {
                bytes: vec![b'x'; maximum],
                next: TerminalCursor::new(cursor.segment(), cursor.offset() + maximum as u64)
                    .unwrap(),
                earliest: start.clone(),
                latest: TerminalCursor::new(2, 8).unwrap(),
                gap: None,
            })
        })
        .unwrap();
        assert_eq!(page.bytes.len(), MAX_TERMINAL_ACTION_OUTPUT_BYTES);
        assert_eq!(calls, 16);
        let page = read_terminal_action_page(&TerminalCursor::new(1, 100).unwrap(), |cursor, _| {
            let (bytes, next) = if cursor.segment() == 1 && cursor.offset() == 100 {
                (vec![1; 5], TerminalCursor::new(1, 105).unwrap())
            } else {
                (vec![2; 8], TerminalCursor::new(2, 8).unwrap())
            };
            Ok(TerminalJournalPage {
                bytes,
                next,
                earliest: start.clone(),
                latest: TerminalCursor::new(2, 8).unwrap(),
                gap: None,
            })
        })
        .unwrap();
        assert_eq!(page.bytes, vec![1; 5]);
        assert_eq!(page.next, TerminalCursor::new(1, 105).unwrap());
    }
}
