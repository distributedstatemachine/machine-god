mod mutation;
mod validation;

use super::records::{
    JournalCatalogCursor, JournalCatalogEntry, JournalCatalogPage, JournalControl, JournalCreate,
    JournalHead, JournalHistoryCursor, JournalHistoryPage, JournalMutation, JournalPageRef,
    JournalPublication, JournalReceipt, JournalRecord, JournalSnapshot, JournalWork, StoredPage,
    identity_matches,
};
use super::{JournalError as Error, Shared, filesystem as fs};
use serde::Serialize;
use std::io::Write;
use std::sync::Arc;

pub(super) struct PendingPublication {
    pub receipt: JournalReceipt,
    id: String,
    expected: Option<[u8; 32]>,
    source: Option<Arc<fs::Source>>,
    candidate: Vec<u8>,
}

struct Reservation {
    shared: Arc<Shared>,
    retained: bool,
    operation: u64,
}
impl Reservation {
    fn acquire(shared: &Arc<Shared>) -> Result<Self, Error> {
        let amount = shared
            .limits
            .page_bytes
            .checked_mul(8)
            .and_then(|n| n.checked_add(shared.limits.head_bytes * 4))
            .ok_or(Error::Limit)?;
        let mut state = shared.state.lock().map_err(|_| Error::Invalid)?;
        if state.pending.is_some() {
            return Err(Error::Ambiguous);
        }
        if state
            .used
            .checked_add(state.reserved)
            .and_then(|n| n.checked_add(amount))
            .is_none_or(|n| n > shared.limits.aggregate_bytes)
        {
            return Err(Error::Limit);
        }
        let next = state
            .next_operation
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        let operation = state.next_operation;
        state.next_operation = next;
        state.reserved = amount;
        Ok(Self {
            shared: shared.clone(),
            retained: false,
            operation,
        })
    }
    fn refresh(&self) -> Result<(), Error> {
        let used = fs::scan_usage(&self.shared.root, self.shared.limits)?;
        let mut state = self.shared.state.lock().map_err(|_| Error::Invalid)?;
        state.used = used.bytes;
        state.entries = used.entries;
        Ok(())
    }
    fn reserve_entries(&self, new_page: bool) -> Result<(), Error> {
        // The exclusive operation slot reserves this peak until publication or
        // reconciliation finishes. Keep one spare entry after publication for
        // owner-epoch replacement, even if a failed head replacement leaves its
        // staging file behind. A new head uses that same staging slot.
        let additional = usize::from(new_page) + 2;
        let state = self.shared.state.lock().map_err(|_| Error::Invalid)?;
        if state
            .entries
            .checked_add(additional)
            .is_none_or(|n| n > self.shared.limits.directory_entries)
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.retained
            && let Ok(mut state) = self.shared.state.lock()
        {
            state.reserved = 0;
        }
    }
}

pub(super) fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, Error> {
    struct Buffer {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit - self.bytes.len() {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut buffer, value).map_err(|_| Error::Limit)?;
    Ok(buffer.bytes.into_boxed_slice().into_vec())
}

fn inspect_unreserved(shared: &Arc<Shared>, id: &str) -> Result<JournalSnapshot, Error> {
    validation::id(id)?;
    let (bytes, source) = fs::observe(&shared.root, &fs::head_name(id), shared.limits.head_bytes)?
        .ok_or(Error::Missing)?;
    let head: JournalHead = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    validation::head(&head, shared.limits)?;
    if head.id != id {
        return Err(Error::Invalid);
    }
    validate_references(shared, &head, false)?;
    fs::validate_source(&shared.root, &fs::head_name(id), &source)?;
    Ok(JournalSnapshot {
        head,
        digest: fs::digest(&bytes),
        identity: Arc::downgrade(shared),
        source_revision: source.revision(),
    })
}
pub(super) fn inspect(shared: &Arc<Shared>, id: &str) -> Result<JournalSnapshot, Error> {
    let _reservation = Reservation::acquire(shared)?;
    inspect_unreserved(shared, id)
}
fn check_snapshot(
    shared: &Arc<Shared>,
    snapshot: &JournalSnapshot,
) -> Result<Arc<fs::Source>, Error> {
    identity_matches(&snapshot.identity, shared)?;
    let encoded = encode(&snapshot.head, shared.limits.head_bytes)?;
    if fs::digest(&encoded) != snapshot.digest {
        return Err(Error::Conflict);
    }
    let (bytes, source) = fs::observe(
        &shared.root,
        &fs::head_name(&snapshot.head.id),
        shared.limits.head_bytes,
    )?
    .ok_or(Error::Conflict)?;
    if fs::digest(&bytes) != snapshot.digest || source.revision() != snapshot.source_revision {
        return Err(Error::Conflict);
    }
    Ok(source)
}

pub(super) fn create(
    shared: &Arc<Shared>,
    create: JournalCreate,
) -> Result<JournalPublication, Error> {
    let reservation = Reservation::acquire(shared)?;
    validation::id(&create.id)?;
    validation::configuration(&create.configuration)?;
    if let Some(parent) = &create.parent_id {
        validation::id(parent)?;
    }
    if fs::read(
        &shared.root,
        &fs::head_name(&create.id),
        shared.limits.head_bytes,
    )?
    .is_some()
    {
        return Err(Error::Conflict);
    }
    if create.mode == machine_god_core::ManagedAgentMode::OneOff && create.initial_work.is_none() {
        return Err(Error::Invalid);
    }
    let mut head = JournalHead {
        version: 1,
        owner_epoch: shared.epoch,
        id: create.id,
        generation: 1,
        revision: 1,
        mode: create.mode,
        configuration: create.configuration,
        transcript: create.transcript,
        controller: create.controller,
        parent_id: create.parent_id,
        parent_owner: create.parent_owner,
        parent_generation: create.parent_generation,
        status: machine_god_core::ManagedAgentState::Idle,
        queue: Vec::new(),
        failure: None,
        intent: None,
        notice_cursor: 0,
        history_tail: None,
        next_sequence: 1,
        last_event_sequence: 0,
    };
    let mut records = if let Some(work) = create.initial_work {
        mutation::enqueue(&mut head, work, shared.limits)?
    } else {
        Vec::new()
    };
    records.insert(
        0,
        mutation::event(&head, machine_god_core::ManagedEventKind::Created)?,
    );
    publish(shared, reservation, None, None, head, records)
}
pub(super) fn mutate(
    shared: &Arc<Shared>,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> Result<JournalPublication, Error> {
    let reservation = Reservation::acquire(shared)?;
    let source = check_snapshot(shared, &snapshot)?;
    if snapshot.head.owner_epoch != shared.epoch {
        if !matches!(mutation, JournalMutation::Recover) {
            return Err(Error::RecoveryRequired);
        }
    } else if matches!(mutation, JournalMutation::Recover) {
        return Err(Error::Conflict);
    }
    let expected = snapshot.digest;
    let mut head = snapshot.head;
    head.owner_epoch = shared.epoch;
    head.revision = head.revision.checked_add(1).ok_or(Error::Exhausted)?;
    let records = mutation::apply(&mut head, mutation, shared.limits)?;
    publish(
        shared,
        reservation,
        Some(expected),
        Some(&source),
        head,
        records,
    )
}

fn bind_event_sequence(head: &mut JournalHead, records: &[JournalRecord]) -> Result<(), Error> {
    let mut event_seen = false;
    for record in records {
        if let JournalRecord::Event(event) = record {
            if event_seen || event.sequence != head.next_sequence || event.revision != head.revision
            {
                return Err(Error::Invalid);
            }
            event_seen = true;
            head.last_event_sequence = event.sequence;
        }
    }
    Ok(())
}

fn publish(
    shared: &Arc<Shared>,
    mut reservation: Reservation,
    expected: Option<[u8; 32]>,
    source: Option<&Arc<fs::Source>>,
    mut head: JournalHead,
    mut records: Vec<JournalRecord>,
) -> Result<JournalPublication, Error> {
    let previous = head.history_tail.clone();
    bind_event_sequence(&mut head, &records)?;
    records.push(JournalRecord::Control(JournalControl {
        revision: head.revision,
        status: head.status,
        intent: head.intent,
        failure: head.failure.clone(),
        controller: head.controller.clone(),
        parent_id: head.parent_id.clone(),
        parent_owner: head.parent_owner.clone(),
        parent_generation: head.parent_generation,
        notice_cursor: head.notice_cursor,
    }));
    let page = if records.is_empty() {
        None
    } else {
        validation::records(&records)?;
        let sequence = head.next_sequence;
        head.next_sequence = sequence.checked_add(1).ok_or(Error::Exhausted)?;
        let page = StoredPage {
            version: 1,
            child_id: head.id.clone(),
            owner: head.transcript.clone(),
            generation: head.generation,
            sequence,
            previous: head.history_tail.clone(),
            records,
        };
        let bytes = encode(&page, shared.limits.page_bytes)?;
        let reference = JournalPageRef {
            child_id: head.id.clone(),
            owner: head.transcript.clone(),
            generation: head.generation,
            sequence,
            length: bytes.len(),
            digest: fs::digest(&bytes),
        };
        for item in &mut head.queue {
            if item.page.sequence == 0 {
                item.page = reference.clone();
            }
        }
        head.history_tail = Some(reference.clone());
        Some((reference, bytes))
    };
    validation::head(&head, shared.limits)?;
    let candidate = encode(&head, shared.limits.head_bytes)?;
    reservation.reserve_entries(page.is_some())?;
    let receipt = JournalReceipt {
        identity: Arc::downgrade(shared),
        operation: reservation.operation,
    };
    // Install reconciliation custody before the first publication effect.
    shared.state.lock().map_err(|_| Error::Invalid)?.pending = Some(PendingPublication {
        receipt: receipt.clone(),
        id: head.id.clone(),
        expected,
        source: source.cloned(),
        candidate: candidate.clone(),
    });
    reservation.retained = true;
    let result = (|| {
        // Repair the immediate predecessor before publishing a new extension.
        if let Some(previous) = previous {
            read_page(shared, &previous, true)?;
        }
        if let Some((reference, bytes)) = &page {
            fs::publish(shared, &fs::page_name(reference), bytes, true, None)?;
        }
        validate_references(shared, &head, true)?;
        let actual = fs::read(
            &shared.root,
            &fs::head_name(&head.id),
            shared.limits.head_bytes,
        )?;
        if actual.as_ref().map(|bytes| fs::digest(bytes)) != expected {
            return Err(Error::Conflict);
        }
        fs::publish(
            shared,
            &fs::head_name(&head.id),
            &candidate,
            false,
            source.map(Arc::as_ref),
        )?;
        reservation.refresh()
    })();
    if result.is_err() {
        return Ok(JournalPublication::Ambiguous(receipt));
    }
    let Ok(snapshot) = inspect_unreserved(shared, &head.id) else {
        return Ok(JournalPublication::Ambiguous(receipt));
    };
    let old = {
        let mut state = shared.state.lock().map_err(|_| Error::Invalid)?;
        state.reserved = 0;
        state.pending.take()
    };
    drop(old);
    Ok(JournalPublication::Confirmed(Box::new(snapshot)))
}

fn read_page(
    shared: &Shared,
    reference: &JournalPageRef,
    durable: bool,
) -> Result<StoredPage, Error> {
    validation::reference(reference, shared.limits)?;
    let name = fs::page_name(reference);
    let bytes = fs::read(&shared.root, &name, shared.limits.page_bytes)?.ok_or(Error::Invalid)?;
    if bytes.len() != reference.length || fs::digest(&bytes) != reference.digest {
        return Err(Error::Invalid);
    }
    let page: StoredPage = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    if page.version != 1
        || page.child_id != reference.child_id
        || page.owner != reference.owner
        || page.generation != reference.generation
        || page.sequence != reference.sequence
    {
        return Err(Error::Invalid);
    }
    validation::records(&page.records)?;
    if let Some(previous) = &page.previous {
        validation::reference(previous, shared.limits)?;
        if previous.child_id != page.child_id
            || previous.generation > page.generation
            || (previous.generation == page.generation && previous.owner != page.owner)
            || previous.sequence >= page.sequence
        {
            return Err(Error::Invalid);
        }
    }
    if durable {
        fs::durable(shared, &name, &bytes, shared.limits.page_bytes)?;
    }
    Ok(page)
}
fn validate_references(shared: &Shared, head: &JournalHead, durable: bool) -> Result<(), Error> {
    if let Some(tail) = &head.history_tail {
        read_page(shared, tail, durable)?;
    }
    for work in &head.queue {
        let page = read_page(shared, &work.page, durable)?;
        if !page.records.iter().any(
            |record| matches!(record, JournalRecord::WorkAccepted(value) if value.id == work.id),
        ) {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
pub(super) fn read_work(
    shared: &Arc<Shared>,
    reference: &JournalPageRef,
) -> Result<JournalWork, Error> {
    let _reservation = Reservation::acquire(shared)?;
    let page = read_page(shared, reference, false)?;
    let mut works = page.records.into_iter().filter_map(|record| match record {
        JournalRecord::WorkAccepted(work) => Some(work),
        _ => None,
    });
    let work = works.next().ok_or(Error::Invalid)?;
    if works.next().is_some() {
        return Err(Error::Invalid);
    }
    Ok(work)
}

pub(super) fn reconcile(
    shared: &Arc<Shared>,
    receipt: &JournalReceipt,
) -> Result<JournalPublication, Error> {
    identity_matches(&receipt.identity, shared)?;
    let (id, expected, source, candidate) = {
        let state = shared.state.lock().map_err(|_| Error::Invalid)?;
        let pending = state.pending.as_ref().ok_or(Error::Conflict)?;
        if pending.receipt.operation != receipt.operation {
            return Err(Error::Conflict);
        }
        (
            pending.id.clone(),
            pending.expected,
            pending.source.clone(),
            pending.candidate.clone(),
        )
    };
    #[cfg(test)]
    super::tests::checkpoint(shared, super::tests::FailurePoint::ReconcileSync)?;
    rustix::fs::fsync(&shared.root).map_err(|_| Error::Persistence)?;
    let current = fs::read(&shared.root, &fs::head_name(&id), shared.limits.head_bytes)?;
    let outcome = if current.as_deref() == Some(&candidate) {
        let snapshot = inspect_unreserved(shared, &id)?;
        validate_references(shared, &snapshot.head, true)?;
        fs::durable(
            shared,
            &fs::head_name(&id),
            &candidate,
            shared.limits.head_bytes,
        )?;
        JournalPublication::Confirmed(Box::new(snapshot))
    } else if current.as_ref().map(|bytes| fs::digest(bytes)) == expected {
        if let Some(source) = source {
            fs::validate_source(&shared.root, &fs::head_name(&id), &source)?;
        }
        // A failed page publication cannot authorize an unreferenced candidate.
        JournalPublication::NotApplied
    } else {
        return Err(Error::Conflict);
    };
    let used = fs::scan_usage(&shared.root, shared.limits)?;
    fs::validate_owner(shared)?;
    let old = {
        let mut state = shared.state.lock().map_err(|_| Error::Invalid)?;
        state.used = used.bytes;
        state.entries = used.entries;
        state.reserved = 0;
        state.pending.take()
    };
    drop(old);
    Ok(outcome)
}

pub(super) fn history(
    shared: &Arc<Shared>,
    snapshot: &JournalSnapshot,
    after: Option<&JournalHistoryCursor>,
    limit: usize,
) -> Result<JournalHistoryPage, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::Limit);
    }
    let _reservation = Reservation::acquire(shared)?;
    check_snapshot(shared, snapshot)?;
    let (mut next, mut offset) = if let Some(cursor) = after {
        identity_matches(&cursor.identity, shared)?;
        if cursor.snapshot != snapshot.digest {
            return Err(Error::Conflict);
        }
        (Some(cursor.next.clone()), cursor.offset)
    } else {
        (snapshot.head.history_tail.clone(), 0)
    };
    let mut records = Vec::new();
    let mut bytes = 0;
    // At most limit pages are touched: every persisted page is nonempty.
    while let Some(reference) = next.take() {
        let page = read_page(shared, &reference, false)?;
        if offset >= page.records.len() {
            return Err(Error::Invalid);
        }
        let count = page.records.len();
        for record in page.records.into_iter().skip(offset) {
            let charge = encode(&record, shared.limits.page_bytes)?.len();
            if records.len() == limit || bytes + charge > 512 * 1024 {
                if records.is_empty() {
                    return Err(Error::Limit);
                }
                break;
            }
            bytes += charge;
            records.push(record);
            offset += 1;
        }
        if offset < count {
            next = Some(reference);
            break;
        }
        next = page.previous;
        offset = 0;
        if records.len() == limit {
            break;
        }
    }
    Ok(JournalHistoryPage {
        records,
        next: next.map(|next| JournalHistoryCursor {
            identity: Arc::downgrade(shared),
            snapshot: snapshot.digest,
            next,
            offset,
        }),
    })
}

pub(super) fn catalog(
    shared: &Arc<Shared>,
    after: Option<&JournalCatalogCursor>,
    limit: usize,
) -> Result<JournalCatalogPage, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::Limit);
    }
    let _reservation = Reservation::acquire(shared)?;
    if let Some(cursor) = after {
        identity_matches(&cursor.identity, shared)?;
    }
    let names = fs::head_candidates(
        shared,
        after.map_or("", |cursor| cursor.after.as_str()),
        limit + 1,
    )?;
    let more = names.len() > limit;
    let mut entries = Vec::new();
    let mut scanned = 0_usize;
    let mut last = None;
    for name in names.into_iter().take(limit) {
        let bytes =
            fs::read(&shared.root, &name, shared.limits.head_bytes)?.ok_or(Error::Conflict)?;
        scanned = scanned.checked_add(bytes.len()).ok_or(Error::Limit)?;
        if scanned > shared.limits.aggregate_bytes {
            return Err(Error::Limit);
        }
        let head: JournalHead = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        validation::head(&head, shared.limits)?;
        if fs::head_name(&head.id) != name {
            return Err(Error::Invalid);
        }
        entries.push(JournalCatalogEntry {
            id: head.id,
            generation: head.generation,
            revision: head.revision,
            name: head.configuration.name,
            parent_id: head.parent_id,
            status: head.status,
            recovery_required: head.owner_epoch != shared.epoch,
        });
        last = Some(name);
    }
    Ok(JournalCatalogPage {
        entries,
        next: if more {
            last.map(|after| JournalCatalogCursor {
                identity: Arc::downgrade(shared),
                after,
            })
        } else {
            None
        },
    })
}
