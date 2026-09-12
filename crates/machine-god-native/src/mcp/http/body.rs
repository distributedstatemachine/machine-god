use super::{
    McpHttpError, McpHttpHeaders, McpHttpLimits, Result,
    io::{Buffered, IO_BYTES},
};
use std::fmt;

pub(super) enum Framing {
    Done,
    Length(u64),
    Eof,
    ChunkSize,
    ChunkData(u64),
    ChunkEnd,
}

/// Single-consumer bounded plaintext body. No decompression, JSON/SSE interpretation
/// or generation/session metadata publication is performed here.
pub struct McpHttpBody {
    io: Option<Buffered>,
    framing: Framing,
    limits: McpHttpLimits,
    remaining: u64,
    trailers: Option<McpHttpHeaders>,
}
impl fmt::Debug for McpHttpBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpBody { <redacted> }")
    }
}
impl McpHttpBody {
    // Only the owned modern subscription lane may select its independently
    // bounded response lifetime after admitting the response head. The same
    // socket, selected clock, cancellation and credential lease stay retained.
    pub(crate) fn subscription_deadline(&mut self, deadline: std::time::Instant) {
        if let Some(io) = &mut self.io {
            io.subscription_deadline(deadline);
        }
    }
    pub(super) fn new(io: Buffered, framing: Framing, limits: McpHttpLimits) -> Self {
        Self {
            io: if matches!(framing, Framing::Done) {
                None
            } else {
                Some(io)
            },
            framing,
            limits,
            remaining: limits.body_bytes,
            trailers: None,
        }
    }
    /// Returns at most 16 KiB, closing the socket at normal completion or error.
    /// Dropping a polled but incomplete read future also closes it: partial parser
    /// state can never be resumed as a different valid response.
    ///
    /// # Errors
    /// Reports cancellation, deadline, malformed/truncated framing and finite limits.
    pub async fn next_chunk(&mut self) -> Result<Option<Box<[u8]>>> {
        if matches!(self.framing, Framing::Done) {
            return Ok(None);
        }
        let mut io = self.io.take().ok_or(McpHttpError::Closed)?;
        let result = self.read(&mut io).await;
        if result.is_ok() && !matches!(self.framing, Framing::Done) {
            self.io = Some(io);
        }
        result
    }
    /// Final bounded trailer observations, separate from initial headers and never
    /// implicitly merged into authentication/session metadata.
    #[must_use]
    pub fn trailers(&self) -> Option<&McpHttpHeaders> {
        self.trailers.as_ref()
    }

    async fn read(&mut self, io: &mut Buffered) -> Result<Option<Box<[u8]>>> {
        loop {
            match self.framing {
                Framing::Done => return Ok(None),
                Framing::Length(count) | Framing::ChunkData(count) => {
                    let maximum = usize::try_from(count.min(IO_BYTES as u64))
                        .map_err(|_| McpHttpError::Limit)?;
                    let bytes = io.take(maximum).await?;
                    if bytes.is_empty() {
                        return Err(McpHttpError::Protocol);
                    }
                    self.charge(bytes.len())?;
                    let remaining = count - bytes.len() as u64;
                    self.framing = match (&self.framing, remaining) {
                        (Framing::Length(_), 0) => Framing::Done,
                        (Framing::Length(_), count) => Framing::Length(count),
                        (_, 0) => Framing::ChunkEnd,
                        (_, count) => Framing::ChunkData(count),
                    };
                    return Ok(Some(bytes));
                }
                Framing::Eof => {
                    let bytes = io.take(IO_BYTES).await?;
                    self.charge(bytes.len())?;
                    if bytes.is_empty() {
                        self.framing = Framing::Done;
                        return Ok(None);
                    }
                    return Ok(Some(bytes));
                }
                Framing::ChunkSize => {
                    let line = io.line(8192).await?;
                    let httparse::Status::Complete((used, count)) =
                        httparse::parse_chunk_size(&line).map_err(|_| McpHttpError::Protocol)?
                    else {
                        return Err(McpHttpError::Protocol);
                    };
                    if used != line.len() {
                        return Err(McpHttpError::Protocol);
                    }
                    if count > self.remaining {
                        return Err(McpHttpError::Limit);
                    }
                    if count == 0 {
                        self.trailers = Some(super::response::trailers(io, self.limits).await?);
                        self.framing = Framing::Done;
                    } else {
                        self.framing = Framing::ChunkData(count);
                    }
                }
                Framing::ChunkEnd => {
                    if io.line(2).await? != b"\r\n" {
                        return Err(McpHttpError::Protocol);
                    }
                    self.framing = Framing::ChunkSize;
                }
            }
            tokio::task::yield_now().await;
        }
    }
    fn charge(&mut self, count: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(count as u64)
            .ok_or(McpHttpError::Limit)?;
        Ok(())
    }
}
