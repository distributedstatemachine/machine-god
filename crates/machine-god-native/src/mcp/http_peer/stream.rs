use super::{BoxFuture, McpHttpPeerError, McpHttpPeerFrame, Result};
use crate::mcp::{
    http::McpHttpBody,
    sse::{SseDecoder, SseEvent, SseLimits},
};

pub(super) type Read = BoxFuture<'static, (Box<Reader>, Result<Option<SseEvent>>)>;
pub(super) struct Reader {
    body: McpHttpBody,
    decoder: SseDecoder,
    buffered: Box<[u8]>,
    offset: usize,
}
impl Reader {
    pub fn new(body: McpHttpBody, limits: SseLimits) -> Result<Box<Self>> {
        Ok(Box::new(Self {
            body,
            decoder: SseDecoder::new(limits).map_err(|_| McpHttpPeerError::Limit)?,
            buffered: Box::new([]),
            offset: 0,
        }))
    }
    pub fn next(mut self: Box<Self>) -> Read {
        Box::pin(async move {
            let result = self.read().await;
            (self, result)
        })
    }
    async fn read(&mut self) -> Result<Option<SseEvent>> {
        loop {
            if self.offset == self.buffered.len() {
                let Some(bytes) = self.body.next_chunk().await? else {
                    self.decoder
                        .finish()
                        .map_err(|_| McpHttpPeerError::Protocol)?;
                    return Ok(None);
                };
                self.buffered = bytes;
                self.offset = 0;
            }
            let progress = self
                .decoder
                .push(&self.buffered[self.offset..])
                .map_err(|_| McpHttpPeerError::Protocol)?;
            self.offset += progress.consumed;
            if let Some(event) = progress.event {
                return Ok(Some(event));
            }
            tokio::task::yield_now().await;
        }
    }
}

pub(super) async fn json(
    mut body: McpHttpBody,
    limits: super::WireLimits,
) -> Result<McpHttpPeerFrame> {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await? {
        if chunk.len() > limits.max_frame_bytes.saturating_sub(bytes.len()) {
            return Err(McpHttpPeerError::Limit);
        }
        bytes.extend_from_slice(&chunk);
        tokio::task::yield_now().await;
    }
    McpHttpPeerFrame::parse_with_limits(bytes.into_boxed_slice(), limits)
}

pub(super) fn response_limits(wire: super::WireLimits) -> SseLimits {
    SseLimits {
        max_line_bytes: wire.max_frame_bytes.saturating_add(6).min(16 * 1024 * 1024),
        max_data_bytes: wire.max_frame_bytes,
        ..SseLimits::default()
    }
}
