//! Leased, explicitly registered inventory service; no ambient/global pool.

use super::{ProcessInventoryHelper, failure};
use crate::NativeOwnedWorkerScopeIdentity;
use crate::background_process::InventoryChild;
use crate::process_inventory_protocol as wire;
use crate::terminal_helper::{
    TerminalHelperError, TerminalHelperErrorKind, check_deadline, encode_helper_deadline,
    read_gate, write_gate,
};
use machine_god_core::{CancellationToken, MAX_TERMINAL_EXEC_DURATION};
use std::io::Read;
use std::process::{ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, TerminalHelperError>;

// Keep the result and its evaluation order intact. Test diagnostics include
// only fixed stages and typed, data-free errors; release builds read no clocks
// or emit output through this wrapper.
macro_rules! query_stage {
    ($started:ident, $deadline:ident, $stage:literal, $result:expr) => {{
        let result = $result;
        #[cfg(test)]
        if let Err(error) = &result {
            eprintln!(
                "inventory query: stage={} error={error:?} elapsed={:?} remaining={:?}",
                $stage,
                $started.elapsed(),
                $deadline.saturating_duration_since(Instant::now())
            );
        }
        result
    }};
}

#[derive(Default)]
pub(crate) struct ServiceRegistration {
    slot: Mutex<ServiceSlot>,
    #[cfg(test)]
    pub(super) starts: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct ServiceSlot {
    lease: Weak<Service>,
    // Only a Boolean tombstone, not child or scoped cleanup ownership. This
    // survives the last lease and is set by exact reap, including quarantine.
    reaped: Option<Arc<AtomicBool>>,
}

impl std::fmt::Debug for ServiceRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceRegistration")
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct InventoryLease(Arc<Service>);

impl std::fmt::Debug for InventoryLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InventoryLease").finish_non_exhaustive()
    }
}

struct Service {
    helper: ProcessInventoryHelper,
    registration: Arc<ServiceRegistration>,
    scope: NativeOwnedWorkerScopeIdentity,
    state: Mutex<ServiceState>,
}

#[derive(Default)]
struct ServiceState {
    ready: Option<Ready>,
    next_sequence: u64,
}

struct Ready {
    // Close both pipes before the child guard kills/reaps or transfers it.
    input: ChildStdin,
    output: ChildStdout,
    child: InventoryChild,
}

impl ServiceRegistration {
    #[cfg(test)]
    pub(super) fn hold_retirement_for_test(&self) -> RetirementHold {
        let mut slot = self.slot.lock().unwrap();
        assert!(slot.lease.upgrade().is_none());
        assert!(
            slot.reaped
                .as_ref()
                .is_none_or(|ticket| ticket.load(Ordering::Acquire))
        );
        let ticket = Arc::new(AtomicBool::new(false));
        slot.reaped = Some(Arc::clone(&ticket));
        RetirementHold(ticket)
    }

    pub(crate) fn prepare(
        self: &Arc<Self>,
        helper: &ProcessInventoryHelper,
        deadline: Instant,
        cancellation: &CancellationToken,
        stop: &[&CancellationToken],
    ) -> Result<InventoryLease> {
        check_startup(deadline, cancellation, stop)?;
        let scope = NativeOwnedWorkerScopeIdentity::current();
        let mut slot = lock_until(&self.slot, deadline, cancellation, stop)?;
        let service = if let Some(service) = slot.lease.upgrade() {
            if !service.scope.matches(&scope) {
                return Err(failure(TerminalHelperErrorKind::InvalidRequest));
            }
            service
        } else {
            if slot
                .reaped
                .as_ref()
                .is_some_and(|reaped| !reaped.load(Ordering::Acquire))
            {
                return Err(failure(TerminalHelperErrorKind::Process));
            }
            let service = Arc::new(Service {
                helper: helper.clone(),
                registration: Arc::clone(self),
                scope: scope.clone(),
                state: Mutex::new(ServiceState::default()),
            });
            slot.lease = Arc::downgrade(&service);
            service
        };
        drop(slot);
        let lease = InventoryLease(service);
        {
            let mut state = lock_until(&lease.0.state, deadline, cancellation, stop)?;
            lease
                .0
                .ensure_ready(&mut state, deadline, cancellation, stop)?;
        }
        Ok(lease)
    }
}

impl Service {
    fn ensure_ready(
        &self,
        state: &mut ServiceState,
        deadline: Instant,
        cancellation: &CancellationToken,
        stop: &[&CancellationToken],
    ) -> Result<()> {
        check_startup(deadline, cancellation, stop)?;
        if let Some(ready) = state.ready.as_mut() {
            if ready
                .child
                .exited()
                .map_err(|_| failure(TerminalHelperErrorKind::Process))?
            {
                state.ready.take();
                return Err(failure(TerminalHelperErrorKind::Process));
            }
            return Ok(());
        }
        let mut slot = lock_until(&self.registration.slot, deadline, cancellation, stop)?;
        if slot
            .reaped
            .as_ref()
            .is_some_and(|reaped| !reaped.load(Ordering::Acquire))
        {
            return Err(failure(TerminalHelperErrorKind::Process));
        }
        let reaped = Arc::new(AtomicBool::new(false));
        slot.reaped = Some(Arc::clone(&reaped));
        drop(slot);
        let stamp = match encode_helper_deadline(deadline, MAX_TERMINAL_EXEC_DURATION) {
            Ok(stamp) => stamp,
            Err(error) => {
                reaped.store(true, Ordering::Release);
                return Err(error);
            }
        };
        let mut command = std::process::Command::new(&self.helper.program);
        command
            .args(&self.helper.arguments)
            .env_clear()
            .env(wire::STARTUP_ENV, stamp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        check_startup(deadline, cancellation, stop)
            .inspect_err(|_| reaped.store(true, Ordering::Release))?;
        #[cfg(test)]
        let spawn_started = Instant::now();
        let mut child = InventoryChild::spawn(&mut command, &reaped)
            .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
        #[cfg(test)]
        {
            self.registration.starts.fetch_add(1, Ordering::AcqRel);
            eprintln!(
                "inventory startup: stage=spawn elapsed={:?} remaining={:?}",
                spawn_started.elapsed(),
                deadline.saturating_duration_since(Instant::now())
            );
        }
        drop(command);
        let (input, output) = child
            .pipes()
            .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
        for fd in [
            &input as &dyn std::os::fd::AsFd,
            &output as &dyn std::os::fd::AsFd,
        ] {
            let flags = rustix::fs::fcntl_getfl(fd)
                .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
            rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)
                .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
        }
        let mut ready = Ready {
            input,
            output,
            child,
        };
        read_startup_ready(&mut ready.output, deadline, cancellation, stop)?;
        check_startup(deadline, cancellation, stop)?;
        if ready
            .child
            .exited()
            .map_err(|_| failure(TerminalHelperErrorKind::Process))?
        {
            return Err(failure(TerminalHelperErrorKind::Process));
        }
        state.ready = Some(ready);
        state.next_sequence = 1;
        Ok(())
    }
}

fn read_startup_ready(
    output: &mut ChildStdout,
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<()> {
    #[cfg(test)]
    let read_started = Instant::now();
    let mut handshake = [0; 8];
    let mut startup_output = StartupOutput {
        output,
        stop,
        #[cfg(test)]
        observations: (0, 0, 0),
    };
    let gate = read_gate(&mut startup_output, &mut handshake, deadline, cancellation)
        .or_else(|error| check_startup(deadline, cancellation, stop).and(Err(error)));
    #[cfg(test)]
    if let Err(error) = &gate {
        eprintln!(
            "inventory startup: stage=ready error={error:?} bytes={} would_block={} interrupted={} read_elapsed={:?}",
            startup_output.observations.0,
            startup_output.observations.1,
            startup_output.observations.2,
            read_started.elapsed()
        );
    }
    gate?;
    if handshake != wire::READY {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    Ok(())
}

impl InventoryLease {
    #[cfg(test)]
    pub(crate) fn set_next_sequence_for_test(&self, sequence: u64) {
        self.0.state.lock().unwrap().next_sequence = sequence;
    }
    pub(crate) fn query(&self, deadline: Instant) -> Result<Vec<u8>> {
        #[cfg(test)]
        let query_started = Instant::now();
        let cancellation = CancellationToken::new();
        query_stage!(
            query_started,
            deadline,
            "admission_deadline",
            check_deadline(deadline, &cancellation)
        )?;
        if deadline.saturating_duration_since(Instant::now())
            > crate::background_process::GROUP_SNAPSHOT_TIMEOUT
        {
            return query_stage!(
                query_started,
                deadline,
                "admission_budget",
                Err(failure(TerminalHelperErrorKind::InvalidRequest))
            );
        }
        if !self
            .0
            .scope
            .matches(&NativeOwnedWorkerScopeIdentity::current())
        {
            return query_stage!(
                query_started,
                deadline,
                "admission_scope",
                Err(failure(TerminalHelperErrorKind::InvalidRequest))
            );
        }
        let mut state = query_stage!(
            query_started,
            deadline,
            "lock",
            lock_until(&self.0.state, deadline, &cancellation, &[])
        )?;
        // A previous failed request may restart here, under this request's own
        // original deadline, only after the previous exact child has reaped.
        // There is never a retry inside the request that detected the failure.
        let result = query_stage!(
            query_started,
            deadline,
            "ensure_ready",
            self.0
                .ensure_ready(&mut state, deadline, &cancellation, &[])
        )
        .and_then(|()| query_ready(&mut state, deadline, &cancellation));
        if result.is_err() {
            state.ready.take();
        }
        result
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct RetirementHold(Arc<AtomicBool>);

#[cfg(test)]
impl Drop for RetirementHold {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn query_ready(
    state: &mut ServiceState,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    #[cfg(test)]
    let query_started = Instant::now();
    let sequence = state.next_sequence;
    state.next_sequence = query_stage!(
        query_started,
        deadline,
        "sequence",
        sequence
            .checked_add(1)
            .ok_or_else(|| failure(TerminalHelperErrorKind::Protocol))
    )?;
    let request = query_stage!(
        query_started,
        deadline,
        "encode_request",
        wire::encode_request(sequence, deadline)
    )?;
    let ready = query_stage!(
        query_started,
        deadline,
        "ready_state",
        state
            .ready
            .as_mut()
            .ok_or_else(|| failure(TerminalHelperErrorKind::Process))
    )?;
    // No unsolicited/stale bytes from an earlier request may become a reply.
    query_stage!(
        query_started,
        deadline,
        "unsolicited_before",
        require_no_extra_output(ready, deadline, cancellation)
    )?;
    query_stage!(
        query_started,
        deadline,
        "write_request",
        write_gate(&mut ready.input, &request, deadline, cancellation)
    )?;
    let mut header = [0; 17];
    query_stage!(
        query_started,
        deadline,
        "read_header",
        read_gate(&mut ready.output, &mut header, deadline, cancellation)
    )?;
    let length = query_stage!(
        query_started,
        deadline,
        "decode_header",
        wire::decode_response_header(&header, sequence)
    )?;
    let mut bytes = vec![0; length];
    query_stage!(
        query_started,
        deadline,
        "read_payload",
        read_gate(&mut ready.output, &mut bytes, deadline, cancellation)
    )?;
    let mut completion = [0; 12];
    query_stage!(
        query_started,
        deadline,
        "read_completion",
        read_gate(&mut ready.output, &mut completion, deadline, cancellation)
    )?;
    query_stage!(
        query_started,
        deadline,
        "validate_completion",
        wire::validate_completion(&completion, sequence)
    )?;
    query_stage!(
        query_started,
        deadline,
        "decode_inventory",
        super::decode_inventory(&bytes)
    )?;
    query_stage!(
        query_started,
        deadline,
        "unsolicited_after",
        require_no_extra_output(ready, deadline, cancellation)
    )?;
    if query_stage!(
        query_started,
        deadline,
        "child_status",
        ready
            .child
            .exited()
            .map_err(|_| failure(TerminalHelperErrorKind::Process))
    )? {
        return query_stage!(
            query_started,
            deadline,
            "child_exited",
            Err(failure(TerminalHelperErrorKind::Process))
        );
    }
    query_stage!(
        query_started,
        deadline,
        "final_deadline",
        check_deadline(deadline, cancellation)
    )?;
    Ok(bytes)
}

fn require_no_extra_output(
    ready: &mut Ready,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<()> {
    loop {
        check_deadline(deadline, cancellation)?;
        let mut extra = [0; 1];
        match ready.output.read(&mut extra) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            _ => return Err(failure(TerminalHelperErrorKind::Protocol)),
        }
    }
}

fn lock_until<'a, T>(
    mutex: &'a Mutex<T>,
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<MutexGuard<'a, T>> {
    loop {
        check_startup(deadline, cancellation, stop)?;
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(failure(TerminalHelperErrorKind::Process));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(
                    Duration::from_millis(1)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    }
}

fn check_startup(
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<()> {
    if stop.iter().any(|token| token.is_cancelled()) {
        return Err(failure(TerminalHelperErrorKind::Cancelled));
    }
    check_deadline(deadline, cancellation)
}

struct StartupOutput<'a> {
    output: &'a mut ChildStdout,
    stop: &'a [&'a CancellationToken],
    #[cfg(test)]
    observations: (usize, usize, usize),
}

impl Read for StartupOutput<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.stop.iter().any(|token| token.is_cancelled()) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        let result = self.output.read(bytes);
        #[cfg(test)]
        match &result {
            Ok(count) => self.observations.0 += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                self.observations.1 += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                self.observations.2 += 1;
            }
            _ => {}
        }
        result
    }
}
