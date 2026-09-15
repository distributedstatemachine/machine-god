//! In-memory queue/receipt double. This does not model real partial pipe writes.
use super::*;
use crate::mcp::protocol::NdjsonDecoder;

pub(crate) struct Pipe {
    shared: Arc<Shared>,
    decoder: NdjsonDecoder,
}

pub(crate) struct Write {
    shared: Arc<Shared>,
    queued: Queued,
}

impl Pipe {
    pub(crate) fn register_handoff_owner(&self) {
        self.shared.handoff.register_owner();
    }

    pub(crate) fn register_handoff_child(&self) {
        self.shared.handoff.register_child();
    }

    pub(crate) fn promote_service(&self) -> bool {
        self.shared.handoff.promote()
    }

    pub(crate) fn new(connection: &McpStdioConnection) -> Self {
        Self {
            shared: connection.shared.clone(),
            decoder: NdjsonDecoder::new(connection.shared.limits).unwrap(),
        }
    }

    pub(crate) fn feed(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let progress = self.decoder.push(bytes).unwrap();
            bytes = &bytes[progress.consumed..];
            if let Some(frame) = progress.frame {
                let mut state = self.shared.state.lock().unwrap();
                assert!(state.frames.len() < MAX_MCP_STDIO_FRAMES);
                state.frames.push_back(frame);
            }
        }
        self.shared.reader.wake();
    }

    pub(crate) fn take_write(&self) -> Option<Write> {
        let queued = self.shared.state.lock().unwrap().queue.pop_front()?;
        Some(Write {
            shared: self.shared.clone(),
            queued,
        })
    }
}

impl Write {
    pub(crate) fn json(&self) -> serde_json::Value {
        let Payload::Control(control) = &self.queued.payload else {
            panic!("expected control")
        };
        serde_json::from_slice(&control.bytes).unwrap()
    }

    pub(crate) fn settle(self, outcome: Result<()>) {
        let Payload::Control(control) = &self.queued.payload else {
            panic!("expected control")
        };
        let acknowledged_bytes = if outcome.is_ok() {
            control.bytes.len()
        } else {
            1
        };
        self.shared.state.lock().unwrap().admitted -= 1;
        self.queued.response.complete(Ok(McpStdioWriteReceipt {
            outcome,
            attempted: true,
            acknowledged_bytes,
        }));
    }
}
