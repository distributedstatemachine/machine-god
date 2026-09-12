use super::{
    COMPLETE, Charge, EARLY_TTL, Inner, MAX_ID_BYTES, MAX_IDS, McpCompletionError,
    McpCompletionLimits, McpCompletionRoute, McpCompletionSource, McpCompletionTicket,
    McpCompletionWaiter, McpCompletionWindow, McpLegacyCompletionNotification, PENDING, Result,
    Waiter, Window, check_waiter, check_window,
};
use machine_god_core::CancellationToken;
use std::{
    collections::VecDeque,
    sync::{
        Arc, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
pub(super) struct State {
    next: u64,
    windows: Vec<WindowEntry>,
    waiters: Vec<WaiterEntry>,
    candidates: Vec<Candidate>,
}
pub(super) struct WindowEntry {
    data: Arc<Window>,
    early: VecDeque<Early>,
}
struct Early {
    id: Box<str>,
    expires: Instant,
    charge: Charge,
}

// Dynamic containers can retain capacity after removal. Reserve their full
// bounded geometric-growth envelope until their owning registry/window drops.
pub(super) fn base_charge(limits: McpCompletionLimits) -> usize {
    1024 + 4
        * (limits.windows * std::mem::size_of::<WindowEntry>()
            + limits.waiters * std::mem::size_of::<WaiterEntry>()
            + limits.candidates * std::mem::size_of::<Candidate>())
}
pub(super) fn window_charge(limits: McpCompletionLimits) -> usize {
    1024 + 2 * limits.early_per_window * std::mem::size_of::<Early>()
}
pub(super) struct WaiterEntry {
    data: Arc<Waiter>,
    completed: u32,
}
struct Candidate {
    source: McpCompletionSource,
    window: u64,
    id: Arc<str>,
    complete: bool,
    handled: bool,
    _charge: Charge,
}
/// Removed callback-bearing owners are always dropped after their state lock.
#[derive(Default)]
pub(super) struct Removed {
    windows: Vec<WindowEntry>,
    waiters: Vec<WaiterEntry>,
    candidates: Vec<Candidate>,
}

fn lock(inner: &Inner) -> Result<std::sync::MutexGuard<'_, State>> {
    inner
        .state
        .lock()
        .map_err(|_| McpCompletionError::Unavailable)
}
fn same_source(left: &McpCompletionSource, right: &McpCompletionSource) -> bool {
    Arc::ptr_eq(&left.0, &right.0)
}
fn check_source(inner: &Inner, source: &McpCompletionSource) -> Result<()> {
    if inner.closed.load(Ordering::Acquire) || source.0.retired.load(Ordering::Acquire) {
        Err(McpCompletionError::Cancelled)
    } else {
        Ok(())
    }
}
fn remove_window_locked(state: &mut State, id: u64, removed: &mut Removed) {
    if let Some(index) = state.windows.iter().position(|entry| entry.data.id == id) {
        removed.windows.push(state.windows.swap_remove(index));
    }
    let mut index = 0;
    while index < state.waiters.len() {
        if state.waiters[index].data.window.id == id {
            removed.waiters.push(state.waiters.swap_remove(index));
        } else {
            index += 1;
        }
    }
    for candidate in &mut state.candidates {
        if candidate.window == id {
            candidate.handled = true;
        }
    }
}
fn clean_cancelled(inner: &Inner) -> Removed {
    let mut removed = Removed::default();
    let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
    let ids: Vec<_> = state
        .windows
        .iter()
        .filter(|entry| check_window(inner, &entry.data).is_err())
        .map(|entry| entry.data.id)
        .collect();
    for id in ids {
        remove_window_locked(&mut state, id, &mut removed);
    }
    let mut index = 0;
    while index < state.candidates.len() {
        if state.candidates[index]
            .source
            .0
            .retired
            .load(Ordering::Acquire)
        {
            removed.candidates.push(state.candidates.swap_remove(index));
        } else {
            index += 1;
        }
    }
    removed
}

pub(super) fn open(
    inner: &Arc<Inner>,
    source: McpCompletionSource,
    cancellation: CancellationToken,
) -> Result<McpCompletionWindow> {
    drop(clean_cancelled(inner));
    check_source(inner, &source)?;
    if cancellation.is_cancelled() {
        return Err(McpCompletionError::Cancelled);
    }
    let mut state = lock(inner)?;
    check_source(inner, &source)?;
    if state.windows.len() >= inner.limits.windows {
        return Err(McpCompletionError::Limit);
    }
    let next = state.next.checked_add(1).ok_or(McpCompletionError::Limit)?;
    let charge = inner.budget.charge(window_charge(inner.limits))?;
    let data = Arc::new(Window {
        id: next,
        source,
        cancellation,
        closed: AtomicBool::new(false),
        changed: CancellationToken::new(),
        _charge: charge,
    });
    state.next = next;
    state.windows.push(WindowEntry {
        data: data.clone(),
        early: VecDeque::new(),
    });
    Ok(McpCompletionWindow {
        inner: Arc::downgrade(inner),
        data,
    })
}

pub(super) fn validate_ids(ids: &[&str]) -> Result<()> {
    if ids.is_empty() || ids.len() > MAX_IDS {
        return Err(McpCompletionError::Limit);
    }
    for (index, id) in ids.iter().enumerate() {
        if id.is_empty() || id.len() > MAX_ID_BYTES {
            return Err(McpCompletionError::Invalid);
        }
        if ids[..index].contains(id) {
            return Err(McpCompletionError::Duplicate);
        }
    }
    Ok(())
}

pub(super) fn register(
    window: &McpCompletionWindow,
    ids: &[&str],
    now: Instant,
) -> Result<McpCompletionTicket> {
    validate_ids(ids)?;
    let inner = window
        .inner
        .upgrade()
        .ok_or(McpCompletionError::Cancelled)?;
    drop(clean_cancelled(&inner));
    let mut state = lock(&inner)?;
    check_window(&inner, &window.data)?;
    if state.waiters.len() >= inner.limits.waiters
        || ids.len()
            > inner
                .limits
                .candidates
                .saturating_sub(state.candidates.len())
    {
        return Err(McpCompletionError::Limit);
    }
    if state.candidates.iter().any(|candidate| {
        same_source(&candidate.source, &window.data.source) && ids.contains(&candidate.id.as_ref())
    }) {
        return Err(McpCompletionError::Duplicate);
    }
    let next = state.next.checked_add(1).ok_or(McpCompletionError::Limit)?;
    let waiter_charge = inner.budget.charge(1024 + ids.len() * 512)?;
    let charges: Vec<_> = ids
        .iter()
        .map(|_| inner.budget.charge(1024))
        .collect::<Result<_>>()?;
    let owned: Box<[Arc<str>]> = ids.iter().map(|id| Arc::from(*id)).collect();
    let completed = promote_early(&mut state, window.data.id, ids, now)?;
    let done = completed.count_ones() as usize == ids.len();
    let data = Arc::new(Waiter {
        id: next,
        window: window.data.clone(),
        ids: owned,
        deadline: OnceLock::new(),
        status: AtomicU8::new(if done { COMPLETE } else { PENDING }),
        waiting: AtomicBool::new(false),
        changed: CancellationToken::new(),
        _charge: waiter_charge,
    });
    for ((index, id), charge) in data.ids.iter().enumerate().zip(charges) {
        state.candidates.push(Candidate {
            source: window.data.source.clone(),
            window: window.data.id,
            id: id.clone(),
            complete: completed & (1 << index) != 0,
            handled: false,
            _charge: charge,
        });
    }
    state.waiters.push(WaiterEntry {
        data: data.clone(),
        completed,
    });
    state.next = next;
    drop(state);
    if done {
        data.changed.cancel();
    }
    Ok(McpCompletionTicket {
        waiter: McpCompletionWaiter {
            inner: Arc::downgrade(&inner),
            data,
        },
    })
}

fn promote_early(state: &mut State, window: u64, ids: &[&str], now: Instant) -> Result<u32> {
    let early = &mut state
        .windows
        .iter_mut()
        .find(|entry| entry.data.id == window)
        .ok_or(McpCompletionError::Cancelled)?
        .early;
    // The pin retains an early notification at exactly its TTL boundary.
    early.retain(|item| item.expires >= now);
    let mut completed = 0_u32;
    for (index, id) in ids.iter().enumerate() {
        if let Some(position) = early.iter().position(|item| item.id.as_ref() == *id) {
            early.remove(position);
            completed |= 1 << index;
        }
    }
    Ok(completed)
}

pub(super) fn observe(
    inner: &Inner,
    source: &McpCompletionSource,
    notification: &McpLegacyCompletionNotification,
    now: Instant,
) -> Result<McpCompletionRoute> {
    drop(clean_cancelled(inner));
    check_source(inner, source)?;
    let mut state = lock(inner)?;
    check_source(inner, source)?;
    if let Some(candidate) = state.candidates.iter_mut().find(|candidate| {
        same_source(&candidate.source, source) && candidate.id.as_ref() == notification.id.as_ref()
    }) {
        if candidate.complete || candidate.handled {
            return Ok(McpCompletionRoute {
                duplicate: true,
                ..McpCompletionRoute::default()
            });
        }
        candidate.complete = true;
        let mut completed = Vec::new();
        for entry in &mut state.waiters {
            if !same_source(&entry.data.window.source, source)
                || check_waiter(inner, &entry.data, now).is_err()
            {
                continue;
            }
            if let Some(index) = entry
                .data
                .ids
                .iter()
                .position(|id| id.as_ref() == notification.id.as_ref())
            {
                entry.completed |= 1 << index;
                if entry.completed.count_ones() as usize == entry.data.ids.len() {
                    entry.data.status.store(COMPLETE, Ordering::Release);
                    completed.push(entry.data.clone());
                }
            }
        }
        drop(state);
        let count = completed.len();
        for data in completed {
            data.changed.cancel();
        }
        return Ok(McpCompletionRoute {
            completed_waiters: count,
            ..McpCompletionRoute::default()
        });
    }
    journal(inner, &mut state, source, notification, now)
}

fn journal(
    inner: &Inner,
    state: &mut State,
    source: &McpCompletionSource,
    notification: &McpLegacyCompletionNotification,
    now: Instant,
) -> Result<McpCompletionRoute> {
    let expires = now
        .checked_add(EARLY_TTL)
        .ok_or(McpCompletionError::Limit)?;
    let mut targets = Vec::new();
    let mut charges = Vec::new();
    let mut duplicate = false;
    for (index, entry) in state.windows.iter_mut().enumerate() {
        entry.early.retain(|item| item.expires >= now);
        if !same_source(&entry.data.source, source) || check_window(inner, &entry.data).is_err() {
            continue;
        }
        if entry.early.iter().any(|item| item.id == notification.id) {
            duplicate = true;
            continue;
        }
        targets.push(index);
        if entry.early.len() < inner.limits.early_per_window {
            charges.push(inner.budget.charge(1024)?);
        }
    }
    let count = targets.len();
    for index in targets {
        let entry = &mut state.windows[index];
        let charge = if entry.early.len() == inner.limits.early_per_window {
            entry
                .early
                .pop_front()
                .expect("bounded nonempty journal")
                .charge
        } else {
            charges.pop().expect("precharged entry")
        };
        entry.early.push_back(Early {
            id: notification.id.clone(),
            expires,
            charge,
        });
    }
    Ok(McpCompletionRoute {
        early_windows: count,
        duplicate,
        ..McpCompletionRoute::default()
    })
}

pub(super) fn remove_source(inner: &Inner, source: &McpCompletionSource) -> Removed {
    let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
    let mut removed = Removed::default();
    let ids: Vec<_> = state
        .windows
        .iter()
        .filter(|entry| same_source(&entry.data.source, source))
        .map(|entry| entry.data.id)
        .collect();
    for id in ids {
        remove_window_locked(&mut state, id, &mut removed);
    }
    let mut index = 0;
    while index < state.candidates.len() {
        if same_source(&state.candidates[index].source, source) {
            removed.candidates.push(state.candidates.swap_remove(index));
        } else {
            index += 1;
        }
    }
    removed
}
pub(super) fn remove_window(inner: &Inner, id: u64) -> Removed {
    let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
    let mut removed = Removed::default();
    remove_window_locked(&mut state, id, &mut removed);
    removed
}
pub(super) fn remove_waiter(inner: &Inner, id: u64) -> Option<WaiterEntry> {
    let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
    let index = state.waiters.iter().position(|entry| entry.data.id == id)?;
    let removed = state.waiters.swap_remove(index);
    for candidate in &mut state.candidates {
        if candidate.window == removed.data.window.id
            && removed
                .data
                .ids
                .iter()
                .any(|id| Arc::ptr_eq(id, &candidate.id))
        {
            candidate.handled = true;
        }
    }
    Some(removed)
}

#[cfg(test)]
pub(super) fn counts(inner: &Inner) -> (usize, usize, usize, usize) {
    let state = inner.state.lock().unwrap();
    (
        state.windows.len(),
        state.waiters.len(),
        state.candidates.len(),
        state.windows.iter().map(|entry| entry.early.len()).sum(),
    )
}

#[cfg(test)]
pub(super) fn exhaust_sequence(inner: &Inner) {
    inner.state.lock().unwrap().next = u64::MAX;
}
