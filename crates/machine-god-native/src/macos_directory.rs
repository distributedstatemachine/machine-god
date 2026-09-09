//! Safe byte parsing over exactly metered macOS directory refills (ADR 0005).

use machine_god_terminal_sys::{DIRECTORY_READ_BUFFER_BYTES, read_directory_chunk};
use rustix::{fd::BorrowedFd, io::Errno};
use std::{fmt, io};

const NAME_OFFSET: usize = 21;
const MAX_NAME_BYTES: usize = 1023;

pub(crate) enum MacosDirectoryEntry {
    Name(Vec<u8>),
    Skipped,
}

impl fmt::Debug for MacosDirectoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => formatter
                .debug_struct("Name")
                .field("bytes", &name.len())
                .finish(),
            Self::Skipped => formatter.write_str("Skipped"),
        }
    }
}

pub(crate) struct MacosDirectoryReader<'fd> {
    fd: BorrowedFd<'fd>,
    buffer: [u8; DIRECTORY_READ_BUFFER_BYTES],
    offset: usize,
    length: usize,
    terminal: bool,
}

impl fmt::Debug for MacosDirectoryReader<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacosDirectoryReader")
            .field("buffered_bytes", &self.length.saturating_sub(self.offset))
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl<'fd> MacosDirectoryReader<'fd> {
    /// Inert: borrows the caller's cursor and initializes fixed scratch only.
    pub(crate) fn new(fd: BorrowedFd<'fd>) -> Self {
        Self {
            fd,
            buffer: [0; DIRECTORY_READ_BUFFER_BYTES],
            offset: 0,
            length: 0,
            terminal: false,
        }
    }

    /// The caller charges one read attempt before calling `next_name` when true.
    pub(crate) fn is_buffer_empty(&self) -> bool {
        self.offset == self.length
    }

    /// Consumes exactly one record, with at most one native refill and no retry.
    /// EOF and malformed data terminate iteration. INTR leaves the buffer empty
    /// so the caller can account for a later separate attempt.
    pub(crate) fn next_name(&mut self) -> Option<Result<MacosDirectoryEntry, Errno>> {
        self.next_with(read_directory_chunk)
    }

    fn next_with(
        &mut self,
        read: impl FnOnce(BorrowedFd<'_>, &mut [u8; DIRECTORY_READ_BUFFER_BYTES]) -> io::Result<usize>,
    ) -> Option<Result<MacosDirectoryEntry, Errno>> {
        if self.terminal {
            return None;
        }
        if self.is_buffer_empty() {
            self.offset = 0;
            self.length = 0;
            match read(self.fd, &mut self.buffer) {
                Ok(0) => {
                    self.terminal = true;
                    return None;
                }
                Ok(length) if length <= self.buffer.len() => self.length = length,
                Ok(_) => {
                    self.terminate();
                    return Some(Err(Errno::IO));
                }
                Err(error) => {
                    let errno = error
                        .raw_os_error()
                        .map_or(Errno::IO, Errno::from_raw_os_error);
                    if errno == Errno::INTR {
                        return Some(Err(errno));
                    }
                    self.terminate();
                    return Some(Err(errno));
                }
            }
        }
        match decode_record(&self.buffer[self.offset..self.length]) {
            Ok((length, entry)) => {
                self.offset += length;
                Some(Ok(entry))
            }
            Err(error) => {
                self.terminate();
                Some(Err(error))
            }
        }
    }

    fn terminate(&mut self) {
        self.terminal = true;
        self.offset = 0;
        self.length = 0;
    }
}

fn decode_record(bytes: &[u8]) -> Result<(usize, MacosDirectoryEntry), Errno> {
    if bytes.len() <= NAME_OFFSET {
        return Err(Errno::IO);
    }
    let record_bytes = usize::from(u16::from_ne_bytes([bytes[16], bytes[17]]));
    let name_bytes = usize::from(u16::from_ne_bytes([bytes[18], bytes[19]]));
    if record_bytes <= NAME_OFFSET
        || record_bytes > bytes.len()
        || !record_bytes.is_multiple_of(4)
        || name_bytes > MAX_NAME_BYTES
        || name_bytes >= record_bytes - NAME_OFFSET
    {
        return Err(Errno::IO);
    }
    let name = &bytes[NAME_OFFSET..NAME_OFFSET + name_bytes];
    if bytes[NAME_OFFSET + name_bytes] != 0 || name.contains(&0) || name.contains(&b'/') {
        return Err(Errno::IO);
    }
    let inode = u64::from_ne_bytes(bytes[..8].try_into().map_err(|_| Errno::IO)?);
    if inode == 0 {
        return Ok((record_bytes, MacosDirectoryEntry::Skipped));
    }
    if name.is_empty() {
        return Err(Errno::IO);
    }
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(name.len())
        .map_err(|_| Errno::NOMEM)?;
    owned.extend_from_slice(name);
    Ok((record_bytes, MacosDirectoryEntry::Name(owned)))
}

#[cfg(test)]
mod tests;
