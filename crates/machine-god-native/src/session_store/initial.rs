//! Exact initial-publication confirmation; observation alone is not durability.

use super::{
    FileSessionStore, SessionNames, ensure_regular, lock_exclusive, map_io_error, open_lock,
    revision_conflict, serialization_failed, serialize_record, sync_file, validate_record_json,
};
use crate::NativeSessionMetadata;
use machine_god_core::{
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStoreError,
};
use rustix::{
    fd::AsFd,
    fs::{AtFlags, Mode, OFlags},
};

pub(super) fn initial_record(
    id: SessionId,
    incarnation: SessionIncarnationId,
    metadata: &NativeSessionMetadata,
) -> Result<SessionRecord, SessionStoreError> {
    let mut record = SessionRecord::empty(id, incarnation);
    record.metadata.insert(
        crate::NATIVE_SESSION_METADATA_KEY.to_owned(),
        metadata.to_value(),
    );
    if NativeSessionMetadata::from_metadata(&record.metadata).as_ref() != Ok(metadata)
        || validate_record_json(&record).is_err()
    {
        return Err(serialization_failed());
    }
    Ok(record)
}

impl FileSessionStore {
    /// Reconcile only the exact canonical bytes originally published by typed
    /// creation. Never rewrite, regenerate, or execute anything on uncertainty.
    pub(crate) fn confirm_initial_record(
        &self,
        id: SessionId,
        incarnation: SessionIncarnationId,
        metadata: &NativeSessionMetadata,
    ) -> Result<Option<SessionRecord>, SessionStoreError> {
        self.confirm_initial_record_with(id, incarnation, metadata, |file| sync_file(file))
    }

    fn confirm_initial_record_with(
        &self,
        id: SessionId,
        incarnation: SessionIncarnationId,
        metadata: &NativeSessionMetadata,
        mut sync: impl FnMut(rustix::fd::BorrowedFd<'_>) -> Result<(), rustix::io::Errno>,
    ) -> Result<Option<SessionRecord>, SessionStoreError> {
        let mut expected = initial_record(id, incarnation, metadata)?;
        expected.revision = SessionRevision(1);
        let bytes = serialize_record(&expected)?;
        let names = SessionNames::for_id(&expected.id);
        let lock = open_lock(self.root.as_fd(), &names.lock)?;
        let _guard = lock_exclusive(&lock)?;
        let file = match rustix::fs::openat(
            &self.root,
            &names.data,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(file) => file,
            Err(rustix::io::Errno::NOENT) => {
                // Absence is confirmed under the same writer lock and directory
                // sync. A leftover temp is not a published session and is not retried.
                sync(self.root.as_fd()).map_err(map_io_error)?;
                return Ok(None);
            }
            Err(error) => return Err(map_io_error(error)),
        };
        let observed = ensure_regular(&file)?;
        if usize::try_from(observed.st_size).ok() != Some(bytes.len()) {
            return Err(revision_conflict());
        }
        // Expected bytes are bounded typed metadata, not an arbitrary transcript.
        // Stream comparison retains one fixed buffer and one overflow byte.
        let mut offset = 0;
        let mut chunk = [0_u8; 4096];
        loop {
            let limit = (bytes.len() - offset + 1).min(chunk.len());
            let read = match rustix::io::read(&file, &mut chunk[..limit]) {
                Ok(read) => read,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(map_io_error(error)),
            };
            if read == 0 {
                if offset != bytes.len() {
                    return Err(revision_conflict());
                }
                break;
            }
            if bytes.get(offset..offset + read) != Some(&chunk[..read]) {
                return Err(revision_conflict());
            }
            offset += read;
        }
        sync(file.as_fd()).map_err(map_io_error)?;
        sync(self.root.as_fd()).map_err(map_io_error)?;
        let retained = ensure_regular(&file)?;
        let linked = rustix::fs::statat(&self.root, &names.data, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(map_io_error)?;
        if !same_version(&observed, &retained) || !same_version(&retained, &linked) {
            return Err(revision_conflict());
        }
        Ok(Some(expected))
    }
}

fn same_version(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_mode == right.st_mode
        && left.st_size == right.st_size
        && left.st_mtime == right.st_mtime
        && left.st_mtime_nsec == right.st_mtime_nsec
        && left.st_ctime == right.st_ctime
        && left.st_ctime_nsec == right.st_ctime_nsec
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mg-initial-confirm-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn identities() -> (SessionId, SessionIncarnationId, NativeSessionMetadata) {
        (
            SessionId::new("initial").unwrap(),
            SessionIncarnationId::new("exact").unwrap(),
            NativeSessionMetadata::default(),
        )
    }

    #[test]
    fn confirmation_requires_both_file_and_directory_sync_and_can_retry_same_candidate() {
        let root = Root::new();
        let store = FileSessionStore::open(&root.0).unwrap();
        let (id, incarnation, metadata) = identities();
        let record = store
            .create_record_with_metadata(id.clone(), incarnation.clone(), &metadata)
            .unwrap();
        for fail_at in [1, 2] {
            let mut calls = 0;
            let result = store.confirm_initial_record_with(
                id.clone(),
                incarnation.clone(),
                &metadata,
                |fd| {
                    calls += 1;
                    if calls == fail_at {
                        Err(rustix::io::Errno::IO)
                    } else {
                        sync_file(fd)
                    }
                },
            );
            assert!(result.is_err());
            assert_eq!(calls, fail_at);
            assert_eq!(
                store
                    .confirm_initial_record(id.clone(), incarnation.clone(), &metadata)
                    .unwrap(),
                Some(record.clone())
            );
        }
    }

    #[test]
    fn absence_requires_directory_sync_without_publication() {
        let root = Root::new();
        let store = FileSessionStore::open(&root.0).unwrap();
        let (id, incarnation, metadata) = identities();
        assert!(
            store
                .confirm_initial_record_with(id.clone(), incarnation.clone(), &metadata, |_| Err(
                    rustix::io::Errno::IO
                ))
                .is_err()
        );
        assert_eq!(
            store
                .confirm_initial_record(id.clone(), incarnation, &metadata)
                .unwrap(),
            None
        );
        let names = SessionNames::for_id(&id);
        assert!(!root.0.join(names.data).exists());
        assert!(!root.0.join(names.temp).exists());
    }

    #[test]
    fn foreign_identity_metadata_or_advanced_record_is_never_confirmed_or_overwritten() {
        let root = Root::new();
        let store = FileSessionStore::open(&root.0).unwrap();
        let (id, incarnation, metadata) = identities();
        let record = store
            .create_record_with_metadata(id.clone(), incarnation.clone(), &metadata)
            .unwrap();
        let path = root.0.join(SessionNames::for_id(&id).data);
        let original = std::fs::read(&path).unwrap();
        assert!(
            store
                .confirm_initial_record(
                    id.clone(),
                    SessionIncarnationId::new("other").unwrap(),
                    &metadata
                )
                .is_err()
        );
        let mut changed = record.clone();
        changed
            .metadata
            .insert("extra".into(), serde_json::Value::Bool(true));
        for candidate in [
            changed,
            SessionRecord {
                revision: SessionRevision(2),
                ..record
            },
        ] {
            let bytes = serialize_record(&candidate).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(
                store
                    .confirm_initial_record(id.clone(), incarnation.clone(), &metadata)
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(&path, original).unwrap();
        assert!(
            store
                .confirm_initial_record(id, incarnation, &metadata)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn equal_bytes_replacement_during_confirmation_is_not_the_retained_file() {
        let root = Root::new();
        let store = FileSessionStore::open(&root.0).unwrap();
        let (id, incarnation, metadata) = identities();
        store
            .create_record_with_metadata(id.clone(), incarnation.clone(), &metadata)
            .unwrap();
        let path = root.0.join(SessionNames::for_id(&id).data);
        let replacement = root.0.join("replacement");
        std::fs::write(&replacement, std::fs::read(&path).unwrap()).unwrap();
        let mut calls = 0;
        let result = store.confirm_initial_record_with(id, incarnation, &metadata, |fd| {
            sync_file(fd)?;
            calls += 1;
            if calls == 2 {
                std::fs::rename(&replacement, &path).unwrap();
            }
            Ok(())
        });
        assert_eq!(calls, 2);
        assert!(result.is_err());
    }
}
