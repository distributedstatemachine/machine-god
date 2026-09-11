use super::{
    MAX_MANAGED_SKILL_ITEMS, NativeManagedSkills, NativeSkillBatchReceipt, NativeSkillInstallPlan,
    NativeSkillItemOutcome, NativeSkillItemReceipt, NativeSkillManagedError,
    NativeSkillManagedErrorKind, NativeSkillReplacementConsent,
    filesystem::{self as fs, Budget, Tree},
};
use machine_god_core::CancellationToken;
use rustix::fs::{FlockOperation, Mode, OFlags, RenameFlags};
use std::time::{Duration, Instant};
use std::{fs::File, sync::Arc};

use super::planning::Operation;
pub(super) use super::planning::{PlannedItem, prepare_create, prepare_install, prepare_remove};
type Kind = NativeSkillManagedErrorKind;
type Error = NativeSkillManagedError;

struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, FlockOperation::Unlock);
    }
}
fn acquire_lock(root: &File, cancellation: &CancellationToken) -> Result<Lock, Kind> {
    if cancellation.is_cancelled() {
        return Err(Kind::Cancelled);
    }
    let fd = rustix::fs::openat(
        root,
        ".machine-god-skills.lock",
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|_| Kind::Unavailable)?;
    let file = File::from(fd);
    let stat = rustix::fs::fstat(&file).map_err(|_| Kind::Unavailable)?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() || stat.st_nlink != 1 {
        return Err(Kind::InvalidEntry);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    for _ in 0..201 {
        if cancellation.is_cancelled() {
            return Err(Kind::Cancelled);
        }
        match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {
                fs::verify_named(root, ".machine-god-skills.lock", &file)?;
                return Ok(Lock(file));
            }
            Err(error)
                if error == rustix::io::Errno::WOULDBLOCK || error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(Kind::Unsupported),
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    Err(Kind::Busy)
}

pub(super) fn commit(
    owner: &NativeManagedSkills,
    plan: NativeSkillInstallPlan,
    consent: &NativeSkillReplacementConsent,
    cancellation: &CancellationToken,
) -> Result<NativeSkillBatchReceipt, Error> {
    if !Arc::ptr_eq(&plan.authority, &owner.root) {
        return Err(Kind::Changed.into());
    }
    verify_consent(&plan.items, consent)?;
    let _lock = acquire_lock(&owner.root, cancellation)?;
    if fs::named_identity(&owner.root, "skills")? != plan.namespace {
        return Err(Kind::Changed.into());
    }
    if let Some((directory, expected)) = &plan.source
        && fs::read_tree(directory, true, &mut Budget::new(cancellation))?.fingerprint()
            != expected.fingerprint()
    {
        return Err(Kind::Changed.into());
    }
    let mut budget = Budget::new(cancellation);
    for item in &plan.items {
        verify_destination(owner, item, &mut budget)?;
    }
    preflight_native_aliases(&owner.root, &plan.items, cancellation)?;
    let parent = if fs::named_identity(&owner.root, "skills")?.is_some() {
        fs::open_directory(&owner.root, "skills")?
    } else {
        let directory = fs::create_directory(&owner.root, "skills")?;
        owner.root.sync_all().map_err(|_| Kind::Unavailable)?;
        directory
    };
    let mut receipts = Vec::new();
    let mut stopped = false;
    for item in plan.items {
        if stopped {
            receipts.push(receipt(
                &item,
                NativeSkillItemOutcome::NotAttempted,
                None,
                None,
            ));
            continue;
        }
        let result = publish_item(owner, &parent, &item, cancellation);
        stopped = result.recovery_id.is_some()
            || matches!(result.outcome, NativeSkillItemOutcome::Indeterminate)
            || result.error == Some(Kind::Cancelled);
        receipts.push(result);
    }
    Ok(NativeSkillBatchReceipt { items: receipts })
}
fn preflight_native_aliases(
    root: &File,
    items: &[PlannedItem],
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    if items.len() < 2 {
        return Ok(());
    }
    let name = fs::random_name()?;
    let directory = fs::create_directory(root, &name)?;
    let mut transaction = Transaction {
        parent: root,
        directory,
        name,
        preserve: false,
    };
    let result = items.iter().try_for_each(|item| {
        if cancellation.is_cancelled() {
            return Err(Kind::Cancelled);
        }
        rustix::fs::mkdirat(
            &transaction.directory,
            &item.destination,
            Mode::from_raw_mode(0o700),
        )
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                Kind::Collision
            } else {
                Kind::Unavailable
            }
        })
    });
    if let Err(kind) = transaction.cleanup() {
        transaction.preserve = true;
        return Err(Error::with_recovery(kind, transaction.name.clone()));
    }
    result.map_err(Into::into)
}
fn verify_consent(
    items: &[PlannedItem],
    consent: &NativeSkillReplacementConsent,
) -> Result<(), Kind> {
    let supplied = match consent {
        NativeSkillReplacementConsent::NoReplace => &[][..],
        NativeSkillReplacementConsent::ExactDestinations(values) => values.as_slice(),
    };
    if supplied.len() > MAX_MANAGED_SKILL_ITEMS {
        return Err(Kind::ResourceLimit);
    }
    for item in items
        .iter()
        .filter(|item| item.operation != Operation::Remove)
    {
        if item
            .expected
            .as_ref()
            .is_some_and(|expected| !supplied.contains(expected))
        {
            return Err(Kind::ReplacementConsentRequired);
        }
    }
    if supplied.iter().any(|revision| {
        !items
            .iter()
            .any(|item| item.expected.as_ref() == Some(revision))
    }) {
        return Err(Kind::Changed);
    }
    Ok(())
}
fn verify_destination(
    owner: &NativeManagedSkills,
    item: &PlannedItem,
    budget: &mut Budget<'_>,
) -> Result<(), Kind> {
    let observed = super::planning::destination_tree(owner, &item.destination, budget)?
        .as_ref()
        .map(|tree| super::planning::revision(&item.destination, tree));
    if observed != item.expected {
        return Err(Kind::Changed);
    }
    Ok(())
}
fn receipt(
    item: &PlannedItem,
    outcome: NativeSkillItemOutcome,
    error: Option<Kind>,
    recovery_id: Option<String>,
) -> NativeSkillItemReceipt {
    NativeSkillItemReceipt {
        destination: item.destination.clone(),
        outcome,
        error,
        recovery_id,
    }
}

struct Transaction<'a> {
    parent: &'a File,
    directory: File,
    name: String,
    preserve: bool,
}
impl Transaction<'_> {
    fn cleanup(&mut self) -> Result<(), Kind> {
        #[cfg(test)]
        if take_fault(InjectedFault::Cleanup) {
            return Err(Kind::Unavailable);
        }
        fs::cleanup(self.parent, &self.name, &self.directory)?;
        self.parent.sync_all().map_err(|_| Kind::Unavailable)?;
        self.preserve = true;
        Ok(())
    }
}
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::cleanup(self.parent, &self.name, &self.directory);
        }
    }
}

fn publish_item(
    owner: &NativeManagedSkills,
    parent: &File,
    item: &PlannedItem,
    cancellation: &CancellationToken,
) -> NativeSkillItemReceipt {
    let make = || -> Result<Transaction<'_>, Kind> {
        if cancellation.is_cancelled() {
            return Err(Kind::Cancelled);
        }
        let name = fs::random_name()?;
        let directory = fs::create_directory(parent, &name)?;
        Ok(Transaction {
            parent,
            directory,
            name,
            preserve: false,
        })
    };
    let mut transaction = match make() {
        Ok(transaction) => transaction,
        Err(kind) => return receipt(item, NativeSkillItemOutcome::Failed, Some(kind), None),
    };
    let mut result = publish_transaction(owner, parent, &mut transaction, item, cancellation);
    if result.outcome == NativeSkillItemOutcome::Indeterminate {
        transaction.preserve = true;
        result.recovery_id = Some(transaction.name.clone());
    } else if let Err(error) = transaction.cleanup() {
        transaction.preserve = true;
        result.recovery_id = Some(transaction.name.clone());
        result.error = Some(error);
        // Publication may have succeeded, but bounded cleanup has not.
        if matches!(
            result.outcome,
            NativeSkillItemOutcome::Installed
                | NativeSkillItemOutcome::Replaced
                | NativeSkillItemOutcome::Removed
        ) {
            result.outcome = NativeSkillItemOutcome::Indeterminate;
        }
    }
    result
}

fn publish_transaction(
    owner: &NativeManagedSkills,
    parent: &File,
    transaction: &mut Transaction<'_>,
    item: &PlannedItem,
    cancellation: &CancellationToken,
) -> NativeSkillItemReceipt {
    let prepared = prepare_transaction(owner, parent, transaction, item, cancellation);
    let staged = match prepared {
        Ok(staged) => staged,
        Err(kind) => return receipt(item, NativeSkillItemOutcome::Failed, Some(kind), None),
    };
    let mut moved_existing = false;
    if item.expected.is_some() {
        if rename(parent, &item.destination, &transaction.directory, "backup").is_err() {
            return receipt(
                item,
                NativeSkillItemOutcome::Indeterminate,
                Some(Kind::Indeterminate),
                None,
            );
        }
        moved_existing = true;
        let verified = fs::open_directory(&transaction.directory, "backup")
            .and_then(|directory| {
                fs::read_tree(
                    &directory,
                    false,
                    &mut Budget::new(&CancellationToken::new()),
                )
            })
            .map(|tree| Some(super::planning::revision(&item.destination, &tree)) == item.expected);
        if verified != Ok(true) {
            return rollback(parent, transaction, item, Kind::Changed);
        }
        if transaction.directory.sync_all().is_err() || parent.sync_all().is_err() {
            return rollback(parent, transaction, item, Kind::Unavailable);
        }
        #[cfg(test)]
        if take_fault(InjectedFault::CancelAfterBackup) {
            cancellation.cancel();
        }
    }
    if item.operation == Operation::Remove {
        let synced = sync_publication(parent, &transaction.directory);
        if synced.is_err()
            || fs::named_identity(parent, &item.destination) != Ok(None)
            || fs::verify_named(&owner.root, "skills", parent).is_err()
        {
            return receipt(
                item,
                NativeSkillItemOutcome::Indeterminate,
                Some(Kind::Indeterminate),
                None,
            );
        }
        return receipt(item, NativeSkillItemOutcome::Removed, None, None);
    }
    if cancellation.is_cancelled() {
        return if moved_existing {
            rollback(parent, transaction, item, Kind::Cancelled)
        } else {
            receipt(
                item,
                NativeSkillItemOutcome::Failed,
                Some(Kind::Cancelled),
                None,
            )
        };
    }
    if rename(&transaction.directory, "staged", parent, &item.destination).is_err() {
        return if moved_existing {
            rollback(parent, transaction, item, Kind::Unavailable)
        } else {
            receipt(
                item,
                NativeSkillItemOutcome::Indeterminate,
                Some(Kind::Indeterminate),
                None,
            )
        };
    }
    finish_publication(
        owner,
        parent,
        transaction,
        item,
        staged.as_ref(),
        moved_existing,
    )
}

fn finish_publication(
    owner: &NativeManagedSkills,
    parent: &File,
    transaction: &Transaction<'_>,
    item: &PlannedItem,
    staged: Option<&Tree>,
    moved_existing: bool,
) -> NativeSkillItemReceipt {
    // Cancellation no longer changes the committed receipt. Always attempt sync
    // even if postpublication validation has already failed.
    let verified = fs::open_directory(parent, &item.destination)
        .and_then(|directory| {
            let observed = fs::read_tree(
                &directory,
                false,
                &mut Budget::new(&CancellationToken::new()),
            )?;
            if staged.is_some_and(|expected| expected.fingerprint() == observed.fingerprint()) {
                Ok(())
            } else {
                Err(Kind::Changed)
            }
        })
        .and_then(|()| fs::verify_named(&owner.root, "skills", parent));
    #[cfg(test)]
    let verified = if take_fault(InjectedFault::PostPublication) {
        Err(Kind::Changed)
    } else {
        verified
    };
    let synced = sync_publication(parent, &transaction.directory);
    if verified.is_err() || synced.is_err() {
        return receipt(
            item,
            NativeSkillItemOutcome::Indeterminate,
            Some(Kind::Indeterminate),
            None,
        );
    }
    receipt(
        item,
        if moved_existing {
            NativeSkillItemOutcome::Replaced
        } else {
            NativeSkillItemOutcome::Installed
        },
        None,
        None,
    )
}

fn sync_publication(parent: &File, transaction: &File) -> Result<(), Kind> {
    let parent_result = parent.sync_all();
    let transaction_result = transaction.sync_all();
    #[cfg(test)]
    if take_fault(InjectedFault::Sync) {
        return Err(Kind::Unavailable);
    }
    parent_result
        .and(transaction_result)
        .map_err(|_| Kind::Unavailable)
}

fn prepare_transaction(
    owner: &NativeManagedSkills,
    parent: &File,
    transaction: &Transaction<'_>,
    item: &PlannedItem,
    cancellation: &CancellationToken,
) -> Result<Option<Tree>, Kind> {
    let mut budget = Budget::new(cancellation);
    let staged = if item.operation == Operation::Remove {
        None
    } else {
        let directory = fs::create_directory(&transaction.directory, "staged")?;
        fs::write_tree(&directory, &item.tree, &mut budget)?;
        let observed = fs::read_tree(&directory, false, &mut Budget::new(cancellation))?;
        if !item.tree.matches_publication(&observed) {
            return Err(Kind::Changed);
        }
        Some(observed)
    };
    transaction
        .directory
        .sync_all()
        .map_err(|_| Kind::Unavailable)?;
    fs::verify_named(parent, &transaction.name, &transaction.directory)?;
    fs::verify_named(&owner.root, "skills", parent)?;
    verify_destination(owner, item, &mut Budget::new(cancellation))?;
    if let Some(expected) = &staged {
        let current = fs::open_directory(&transaction.directory, "staged")?;
        if fs::read_tree(&current, false, &mut Budget::new(cancellation))?.fingerprint()
            != expected.fingerprint()
        {
            return Err(Kind::Changed);
        }
    }
    if cancellation.is_cancelled() {
        return Err(Kind::Cancelled);
    }
    Ok(staged)
}
fn rename(from: &File, source: &str, to: &File, destination: &str) -> Result<(), Kind> {
    #[cfg(test)]
    if (source == "staged" && take_fault(InjectedFault::Publish))
        || (source == "backup" && take_fault(InjectedFault::Rollback))
    {
        return Err(Kind::Unavailable);
    }
    rustix::fs::renameat_with(from, source, to, destination, RenameFlags::NOREPLACE)
        .map_err(|_| Kind::Unavailable)
}
fn rollback(
    parent: &File,
    transaction: &Transaction<'_>,
    item: &PlannedItem,
    kind: Kind,
) -> NativeSkillItemReceipt {
    let restored = rename(&transaction.directory, "backup", parent, &item.destination);
    let synced = sync_publication(parent, &transaction.directory);
    let verified = fs::open_directory(parent, &item.destination)
        .and_then(|directory| {
            fs::read_tree(
                &directory,
                false,
                &mut Budget::new(&CancellationToken::new()),
            )
        })
        .map(|tree| Some(super::planning::revision(&item.destination, &tree)) == item.expected);
    if restored.is_ok() && synced.is_ok() && verified == Ok(true) {
        receipt(item, NativeSkillItemOutcome::RolledBack, Some(kind), None)
    } else {
        receipt(
            item,
            NativeSkillItemOutcome::Indeterminate,
            Some(Kind::Indeterminate),
            None,
        )
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum InjectedFault {
    Publish,
    PublishAndRollback,
    Rollback,
    PostPublication,
    Sync,
    Cleanup,
    CancelAfterBackup,
}
#[cfg(test)]
thread_local! { static FAULT: std::cell::Cell<Option<InjectedFault>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(super) fn inject_fault(fault: InjectedFault) {
    FAULT.set(Some(fault));
}
#[cfg(test)]
fn take_fault(point: InjectedFault) -> bool {
    FAULT.with(|cell| {
        if point == InjectedFault::Publish && cell.get() == Some(InjectedFault::PublishAndRollback)
        {
            cell.set(Some(InjectedFault::Rollback));
            return true;
        }
        if cell.get() == Some(point) {
            cell.set(None);
            true
        } else {
            false
        }
    })
}
