//! Soft rejection sampling, never a hard filesystem quota or publication scan.
use super::{Kind, NativeSkillManagedError};
use machine_god_core::CancellationToken;
use rustix::fd::AsFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use std::fs::File;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) struct CloneWatchdog<'a> {
    root: Arc<File>,
    limit: u64,
    deadline: Instant,
    cancellation: &'a CancellationToken,
    next: Instant,
    operations: usize,
}
impl<'a> CloneWatchdog<'a> {
    pub(super) fn new(
        root: Arc<File>,
        limit: u64,
        deadline: Instant,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            root,
            limit,
            deadline,
            cancellation,
            next: Instant::now(),
            operations: 0,
        }
    }
    pub(super) fn check(&mut self, force: bool) -> Result<(), NativeSkillManagedError> {
        self.charge()?;
        if !force && Instant::now() < self.next {
            return Ok(());
        }
        let mut bytes = 0;
        let mut entries = 0;
        let root = Arc::clone(&self.root);
        self.scan(&root, 0, &mut bytes, &mut entries)?;
        self.next = Instant::now() + Duration::from_millis(250);
        Ok(())
    }
    fn charge(&mut self) -> Result<(), NativeSkillManagedError> {
        if self.cancellation.is_cancelled() {
            return Err(Kind::Cancelled.into());
        }
        if Instant::now() >= self.deadline {
            return Err(Kind::TimedOut.into());
        }
        self.operations += 1;
        if self.operations > 2_000_000 {
            return Err(Kind::ResourceLimit.into());
        }
        Ok(())
    }
    fn scan(
        &mut self,
        directory: &File,
        depth: usize,
        bytes: &mut u64,
        entries: &mut usize,
    ) -> Result<(), NativeSkillManagedError> {
        self.charge()?;
        if depth > 32 {
            return Err(Kind::ResourceLimit.into());
        }
        // Each scan gets an independent cursor without reopening a display path.
        let cursor = rustix::fs::openat(
            directory,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Kind::Unavailable)?;
        #[cfg(target_os = "linux")]
        let mut buffer = [std::mem::MaybeUninit::uninit(); 8192];
        #[cfg(target_os = "linux")]
        let mut stream = rustix::fs::RawDir::new(cursor.as_fd(), &mut buffer);
        #[cfg(target_os = "macos")]
        let mut stream = crate::macos_directory::MacosDirectoryReader::new(cursor.as_fd());
        loop {
            self.charge()?;
            #[cfg(target_os = "linux")]
            let name = {
                let Some(entry) = stream.next() else {
                    break;
                };
                entry.map_err(|_| Kind::Unavailable)?.file_name().to_owned()
            };
            #[cfg(target_os = "macos")]
            let name = {
                let Some(entry) = stream.next_name() else {
                    break;
                };
                match entry.map_err(|_| Kind::Unavailable)? {
                    crate::macos_directory::MacosDirectoryEntry::Name(bytes) => {
                        std::ffi::CString::new(bytes).map_err(|_| Kind::Unavailable)?
                    }
                    crate::macos_directory::MacosDirectoryEntry::Skipped => continue,
                }
            };
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            *entries += 1;
            if *entries > 16_384 {
                return Err(Kind::ResourceLimit.into());
            }
            let stat = match rustix::fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(rustix::io::Errno::NOENT) => continue,
                Err(_) => return Err(Kind::Unavailable.into()),
            };
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::RegularFile => {
                    *bytes = bytes
                        .checked_add(u64::try_from(stat.st_size).map_err(|_| Kind::Unavailable)?)
                        .ok_or(Kind::ResourceLimit)?;
                    if *bytes > self.limit {
                        return Err(Kind::ResourceLimit.into());
                    }
                }
                FileType::Directory => {
                    self.charge()?;
                    let child = match rustix::fs::openat(
                        directory,
                        &name,
                        OFlags::RDONLY
                            | OFlags::DIRECTORY
                            | OFlags::NOFOLLOW
                            | OFlags::NONBLOCK
                            | OFlags::CLOEXEC,
                        Mode::empty(),
                    ) {
                        Ok(child) => File::from(child),
                        Err(rustix::io::Errno::NOENT) => continue,
                        Err(_) => return Err(Kind::Changed.into()),
                    };
                    let opened = rustix::fs::fstat(&child).map_err(|_| Kind::Unavailable)?;
                    if opened.st_dev != stat.st_dev || opened.st_ino != stat.st_ino {
                        return Err(Kind::Changed.into());
                    }
                    self.scan(&child, depth + 1, bytes, entries)?;
                }
                // No symlink traversal or special-file IO. Final managed scan
                // independently validates the publishable skill trees.
                _ => {}
            }
        }
        Ok(())
    }
}
