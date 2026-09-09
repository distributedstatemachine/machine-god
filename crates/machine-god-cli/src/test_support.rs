use std::io;

#[derive(Debug, Default)]
pub(crate) struct BrokenWriter;

impl io::Write for BrokenWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(crate) struct ZeroProgressWriter {
    pub(crate) captured: Vec<u8>,
    pub(crate) calls: usize,
}

impl io::Write for ZeroProgressWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        assert!(!buffer.is_empty());
        self.calls += 1;
        Ok(0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct PartialThenBrokenWriter {
    pub(crate) prefix: Vec<u8>,
    pub(crate) prefix_limit: usize,
    pub(crate) accepted_first_write: bool,
}

impl PartialThenBrokenWriter {
    pub(crate) fn new(prefix_limit: usize) -> Self {
        assert!(prefix_limit > 0);
        Self {
            prefix: Vec::new(),
            prefix_limit,
            accepted_first_write: false,
        }
    }
}

impl io::Write for PartialThenBrokenWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.accepted_first_write {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        }

        let accepted = buffer.len().min(self.prefix_limit);
        assert!(accepted > 0);
        self.prefix.extend_from_slice(&buffer[..accepted]);
        self.accepted_first_write = true;
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(crate) struct FirstWriteFailsThenCaptures {
    pub(crate) captured: Vec<u8>,
    pub(crate) failed: bool,
}

impl io::Write for FirstWriteFailsThenCaptures {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if !self.failed {
            self.failed = true;
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        }
        self.captured.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
