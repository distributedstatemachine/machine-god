use super::{BoxFuture, McpHttpPeerError, McpHttpPeerFrame, NegotiatedProtocol, Result};
use crate::mcp::{
    http::McpHttpBody,
    sse::{SseDecoder, SseEvent, SseLimits, SseMode},
};

pub(super) type Read = BoxFuture<'static, (Box<Reader>, Result<Option<SseEvent>>)>;
pub(super) struct Reader {
    body: McpHttpBody,
    decoder: SseDecoder,
    buffered: Box<[u8]>,
    offset: usize,
}
impl Reader {
    pub fn new(body: McpHttpBody, mode: SseMode, limits: SseLimits) -> Result<Box<Self>> {
        Ok(Box::new(Self {
            body,
            decoder: SseDecoder::new(mode, limits).map_err(|_| McpHttpPeerError::Limit)?,
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

pub(super) async fn json(mut body: McpHttpBody, maximum: usize) -> Result<McpHttpPeerFrame> {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await? {
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            return Err(McpHttpPeerError::Limit);
        }
        bytes.extend_from_slice(&chunk);
        tokio::task::yield_now().await;
    }
    McpHttpPeerFrame::parse(bytes.into_boxed_slice())
}

#[derive(Default)]
pub(super) struct Resume {
    pub id: Option<Box<str>>,
    pub retry_ms: u32,
    pub priming: bool,
}
impl Resume {
    /// Called only after this event's data and exact stream owner were admitted.
    pub fn commit(&mut self, event: &SseEvent) -> Result<()> {
        if let Some(id) = event.id() {
            if id.len() > 4096
                || !id
                    .bytes()
                    .all(|byte| byte == b'\t' || (byte >= b' ' && byte != 0x7f))
            {
                return Err(McpHttpPeerError::Protocol);
            }
            self.priming |= id.is_empty() && event.data().is_empty();
            self.id = Some(id.into());
        }
        if let Some(delay) = event.retry_ms() {
            self.retry_ms = delay;
        }
        Ok(())
    }
    pub fn resumable(&self, protocol: NegotiatedProtocol) -> bool {
        self.id.as_ref().is_some_and(|id| !id.is_empty())
            || protocol.allows_legacy_http_poll_close() && self.priming
    }
}
