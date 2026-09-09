use super::super::{
    FileSessionScanControl, FileSessionStore, MAX_FILE_SESSION_BYTES,
    MAX_LIST_SESSION_DIRECTORY_ENTRIES, MAX_LIST_SESSION_TOTAL_RECORD_BYTES, ObjectOnly,
    SessionNames, StoredEnvelope, ensure_listing_root_is_linked, is_session_data_name,
    lock_name_for_data_name, serialize_record,
};
#[cfg(test)]
use super::tests;
use super::{Error, Source, decode, migrate_metadata, read_source_bounded, same};
use crate::session_maintenance::{
    NativeSessionCleanupMode as ModeChoice, NativeSessionCleanupReport as Report,
    NativeSessionCleanupStatus as Status,
};
use rustix::fd::AsFd;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

impl FileSessionStore {
    pub(crate) fn maintenance_cleanup(
        &self,
        mode: ModeChoice,
        control: &FileSessionScanControl,
    ) -> Result<Report, Error> {
        control.check()?;
        ensure_listing_root_is_linked(self.root.as_fd()).map_err(|_| Error::Unavailable)?;
        let root_stat = rustix::fs::fstat(&self.root)?;
        if root_stat.st_uid != rustix::process::geteuid().as_raw() || root_stat.st_mode & 0o077 != 0
        {
            return Ok(Report {
                outcomes: vec![Status::Untrusted],
                scan_complete: false,
            });
        }
        let mut directory = Dir::read_from(&self.root)?;
        let mut report = Report {
            outcomes: Vec::new(),
            scan_complete: true,
        };
        let mut entries = 0;
        let mut bytes = 0;
        for entry in &mut directory {
            if let Err(error) = control.check() {
                if report.outcomes.is_empty() {
                    return Err(error.into());
                }
                report.scan_complete = false;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if report.outcomes.is_empty() => return Err(error.into()),
                Err(_) => {
                    report.scan_complete = false;
                    break;
                }
            };
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            entries += 1;
            if entries > MAX_LIST_SESSION_DIRECTORY_ENTRIES {
                report.scan_complete = false;
                break;
            }
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            let Some(data) = staging_data_name(name) else {
                continue;
            };
            if bytes >= MAX_LIST_SESSION_TOTAL_RECORD_BYTES {
                report.scan_complete = false;
                break;
            }
            let outcome = cleanup_one(self.root.as_fd(), name, &data, mode, control, &mut bytes);
            match outcome {
                Ok(status) => report.outcomes.push(status),
                Err(Error::Cancelled) if report.outcomes.is_empty() => {
                    return Err(Error::Cancelled);
                }
                Err(Error::Cancelled) => {
                    report.scan_complete = false;
                    break;
                }
                Err(Error::Oversized) if bytes >= MAX_LIST_SESSION_TOTAL_RECORD_BYTES => {
                    report.scan_complete = false;
                    break;
                }
                Err(Error::Busy) => report.outcomes.push(Status::ActiveWriter),
                Err(_) => report.outcomes.push(Status::Untrusted),
            }
        }
        Ok(report)
    }
}
fn staging_data_name(name: &str) -> Option<String> {
    if let Some((data, suffix)) = name.split_once(".maintenance-") {
        let nonce = suffix.strip_suffix(".tmp")?;
        if nonce.len() == 32
            && nonce.bytes().all(|b| b.is_ascii_hexdigit())
            && is_session_data_name(data.as_bytes())
        {
            return Some(data.to_owned());
        }
    }
    let stem = name.strip_suffix(".tmp")?;
    let data = format!("{stem}.json");
    is_session_data_name(data.as_bytes()).then_some(data)
}
fn trusted(stat: &rustix::fs::Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode).is_file()
        && stat.st_uid == rustix::process::geteuid().as_raw()
        && stat.st_nlink == 1
        && stat.st_mode & 0o7777 == 0o600
}
fn cleanup_one(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    data: &str,
    mode: ModeChoice,
    control: &FileSessionScanControl,
    bytes: &mut usize,
) -> Result<Status, Error> {
    // Existing lock only: a name-shaped artifact cannot cause a new ownership claim.
    let lock_name = lock_name_for_data_name(data);
    let lock = rustix::fs::openat(
        root,
        &lock_name,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    let lock_stat = rustix::fs::fstat(&lock)?;
    if !trusted(&lock_stat) {
        return Ok(Status::Untrusted);
    }
    let _guard = control.lock(&lock)?;
    let staging = read_budgeted(root, name, control, bytes)?;
    if !trusted(&staging.stat) {
        return Ok(Status::Untrusted);
    }
    let current = read_budgeted(root, data, control, bytes)?;
    if !trusted(&current.stat) {
        return Ok(Status::Untrusted);
    }
    let ObjectOnly(envelope) = serde_json::from_slice::<ObjectOnly<StoredEnvelope>>(&current.bytes)
        .map_err(|_| Error::Corrupt)?;
    let id = envelope.record.0.id;
    if SessionNames::for_id(&id).data != data {
        return Ok(Status::Untrusted);
    }
    let mut record = decode(&current.bytes, &id)?;
    if crate::NativeSessionMetadata::from_metadata(&record.metadata).is_err() {
        return Ok(Status::Untrusted);
    }
    let redundant = staging.bytes == current.bytes;
    let reproducible = migrate_metadata(&mut record).is_ok_and(|changed| changed)
        && serialize_record(&record).is_ok_and(|expected| expected == staging.bytes);
    if !redundant && !reproducible {
        return Ok(Status::Untrusted);
    }
    control.check()?;
    staging.check(root, name)?;
    current.check(root, data)?;
    if !same(
        &lock_stat,
        &rustix::fs::statat(root, &lock_name, AtFlags::SYMLINK_NOFOLLOW)?,
    ) {
        return Ok(Status::Untrusted);
    }
    if mode == ModeChoice::ReportOnly {
        return Ok(Status::ReportOnly);
    }
    ensure_listing_root_is_linked(root).map_err(|_| Error::Unavailable)?;
    control.check()?;
    rustix::fs::unlinkat(root, name, AtFlags::empty())?;
    #[cfg(test)]
    if tests::publication_checkpoint(tests::PublicationStage::AfterCleanup).is_err() {
        return Ok(Status::Indeterminate);
    }
    Ok(if rustix::fs::fsync(root).is_ok() {
        Status::Completed
    } else {
        Status::Indeterminate
    })
}

fn read_budgeted(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    control: &FileSessionScanControl,
    bytes: &mut usize,
) -> Result<Source, Error> {
    let remaining = MAX_LIST_SESSION_TOTAL_RECORD_BYTES.saturating_sub(*bytes);
    let result = read_source_bounded(root, name, control, remaining, bytes);
    if matches!(result, Err(Error::Oversized)) && remaining < MAX_FILE_SESSION_BYTES {
        *bytes = MAX_LIST_SESSION_TOTAL_RECORD_BYTES;
    }
    result
}
