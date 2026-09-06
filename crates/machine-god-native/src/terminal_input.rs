//! Ordered, bounded terminal input. Payloads and replies share one noninterleaving
//! transport queue, while only user payloads require a writer lease.

use std::collections::VecDeque;
use std::fmt;
use std::num::NonZeroU64;

use machine_god_core::{
    TerminalActorRole, TerminalNamedKey, TerminalWriteLeaseIntent, TerminalWritePayload,
    TerminalWriteRequest,
};

use crate::background_input::{
    BackgroundInputReceipt, BackgroundInputStatus, MAX_BACKGROUND_INPUT_BYTES,
};

const MAX_REPLY_BYTES: usize = 4096;
const MAX_REPLY_FRAMES: usize = 16;
const MAX_RECEIPTS: usize = 64;

/// Host-assigned identity inside an already authorized terminal session. This
/// identifier is not a capability and must not be accepted from model input.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct TerminalWriterId(NonZeroU64);
impl TerminalWriterId {
    pub(crate) const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }
}
impl fmt::Debug for TerminalWriterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TerminalWriterId { .. }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalInputError {
    Invalid,
    Cancelled,
    LeaseConflict,
    Busy,
    Closed,
    Capacity,
    NotFound,
}
type Result<T> = std::result::Result<T, TerminalInputError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalInputProgress {
    Pending,
    Complete,
    Closed,
    Failed,
}

/// Accepted means accepted by the native write, not merely queued. Pending
/// suffixes remain owned across attention cancellation and are never resent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalInputReceipt {
    pub(crate) operation_id: Option<NonZeroU64>,
    pub(crate) accepted_bytes: usize,
    pub(crate) encoded_bytes: usize,
    pub(crate) progress: TerminalInputProgress,
}
impl TerminalInputReceipt {
    fn lease() -> Self {
        Self {
            operation_id: None,
            accepted_bytes: 0,
            encoded_bytes: 0,
            progress: TerminalInputProgress::Complete,
        }
    }
}

struct Frame {
    bytes: Vec<u8>,
    offset: usize,
    operation: Option<NonZeroU64>,
}
struct SavedReceipt {
    writer: (TerminalActorRole, TerminalWriterId),
    receipt: TerminalInputReceipt,
}

pub(crate) struct TerminalInput {
    lease: Option<(TerminalActorRole, TerminalWriterId)>,
    quiesced: bool,
    next_operation: Option<NonZeroU64>,
    pending_operation: Option<NonZeroU64>,
    frames: VecDeque<Frame>,
    reply_bytes: usize,
    reply_frames: usize,
    receipts: VecDeque<SavedReceipt>,
}
impl fmt::Debug for TerminalInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalInput").finish_non_exhaustive()
    }
}

impl TerminalInput {
    pub(crate) fn new() -> Self {
        Self {
            lease: None,
            quiesced: false,
            next_operation: NonZeroU64::new(1),
            pending_operation: None,
            frames: VecDeque::new(),
            reply_bytes: 0,
            reply_frames: 0,
            receipts: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn submit(
        &mut self,
        writer: TerminalWriterId,
        request: &TerminalWriteRequest,
        cancelled: bool,
    ) -> Result<TerminalInputReceipt> {
        self.submit_with_actor(TerminalActorRole::Agent, writer, request, cancelled)
    }

    /// Validate before the session publishes a lease transition. No transport
    /// calls, payload allocation, or authority mutation occur here.
    pub(crate) fn check_submit(
        &self,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        request: &TerminalWriteRequest,
        cancelled: bool,
    ) -> Result<()> {
        request
            .validate()
            .map_err(|_| TerminalInputError::Invalid)?;
        if cancelled {
            return Err(TerminalInputError::Cancelled);
        }
        if self.quiesced {
            return Err(TerminalInputError::Closed);
        }
        let writer = (actor, writer);
        match request.lease {
            TerminalWriteLeaseIntent::Acquire => self.check_holder(writer)?,
            TerminalWriteLeaseIntent::Release => {
                self.check_holder(writer)?;
                if self.pending_operation.is_some() {
                    return Err(TerminalInputError::Busy);
                }
            }
            TerminalWriteLeaseIntent::Use => {
                if self.lease != Some(writer) {
                    return Err(TerminalInputError::LeaseConflict);
                }
                if self.pending_operation.is_some() {
                    return Err(TerminalInputError::Busy);
                }
                self.next_operation.ok_or(TerminalInputError::Capacity)?;
            }
            TerminalWriteLeaseIntent::Revoke => {}
        }
        Ok(())
    }

    pub(crate) fn submit_with_actor(
        &mut self,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        request: &TerminalWriteRequest,
        cancelled: bool,
    ) -> Result<TerminalInputReceipt> {
        self.check_submit(actor, writer, request, cancelled)?;
        let writer = (actor, writer);
        match request.lease {
            TerminalWriteLeaseIntent::Revoke => self.quiesce(),
            TerminalWriteLeaseIntent::Acquire => {
                self.check_holder(writer)?;
                self.lease = Some(writer);
            }
            TerminalWriteLeaseIntent::Release => {
                self.check_holder(writer)?;
                if self.pending_operation.is_some() {
                    return Err(TerminalInputError::Busy);
                }
                self.lease = None;
            }
            TerminalWriteLeaseIntent::Use => {
                if self.lease != Some(writer) {
                    return Err(TerminalInputError::LeaseConflict);
                }
                if self.pending_operation.is_some() {
                    return Err(TerminalInputError::Busy);
                }
                let operation = self.next_operation.ok_or(TerminalInputError::Capacity)?;
                let bytes = encode(
                    request
                        .payload
                        .as_ref()
                        .ok_or(TerminalInputError::Invalid)?,
                )?;
                let receipt = TerminalInputReceipt {
                    operation_id: Some(operation),
                    accepted_bytes: 0,
                    encoded_bytes: bytes.len(),
                    progress: TerminalInputProgress::Pending,
                };
                self.next_operation = operation.get().checked_add(1).and_then(NonZeroU64::new);
                self.pending_operation = Some(operation);
                self.frames.push_back(Frame {
                    bytes,
                    offset: 0,
                    operation: Some(operation),
                });
                self.receipts.push_back(SavedReceipt { writer, receipt });
                if self.receipts.len() > MAX_RECEIPTS {
                    self.receipts.pop_front();
                }
                return Ok(receipt);
            }
        }
        Ok(TerminalInputReceipt::lease())
    }

    /// Admit a whole bounded reply batch, or none of it. Resident reply bytes
    /// (including already-written prefixes) count until their frame is freed.
    pub(crate) fn replies(&mut self, replies: Vec<Vec<u8>>) -> Result<()> {
        if self.quiesced {
            return Err(TerminalInputError::Closed);
        }
        let bytes = replies
            .iter()
            .try_fold(0_usize, |sum, bytes| {
                if bytes.is_empty() || bytes.len() > 256 {
                    None
                } else {
                    sum.checked_add(bytes.len())
                }
            })
            .ok_or(TerminalInputError::Invalid)?;
        if replies.len() > MAX_REPLY_FRAMES.saturating_sub(self.reply_frames)
            || bytes > MAX_REPLY_BYTES.saturating_sub(self.reply_bytes)
        {
            return Err(TerminalInputError::Capacity);
        }
        self.reply_bytes += bytes;
        self.reply_frames += replies.len();
        self.frames.extend(replies.into_iter().map(|bytes| Frame {
            bytes,
            offset: 0,
            operation: None,
        }));
        Ok(())
    }

    /// Exactly one bounded backend call. A short write retains its suffix at
    /// the queue front; neither other writers nor protocol replies interleave.
    pub(crate) fn flush(
        &mut self,
        write: impl FnOnce(&[u8]) -> std::result::Result<BackgroundInputReceipt, ()>,
    ) {
        let Some(frame) = self.frames.front_mut() else {
            return;
        };
        let end = frame
            .bytes
            .len()
            .min(frame.offset + MAX_BACKGROUND_INPUT_BYTES);
        let result = write(&frame.bytes[frame.offset..end]);
        let Ok(result) = result else {
            self.fail();
            return;
        };
        if result.bytes_written() > end - frame.offset {
            self.fail();
            return;
        }
        frame.offset += result.bytes_written();
        if let Some(operation) = frame.operation {
            let saved = self
                .receipts
                .iter_mut()
                .find(|saved| saved.receipt.operation_id == Some(operation))
                .expect("the sole pending receipt is retained");
            saved.receipt.accepted_bytes = frame.offset;
            if frame.offset == frame.bytes.len() {
                saved.receipt.progress = TerminalInputProgress::Complete;
                self.pending_operation = None;
            }
        }
        if frame.offset == frame.bytes.len() {
            let frame = self
                .frames
                .pop_front()
                .expect("front frame remains present");
            if frame.operation.is_none() {
                self.reply_bytes -= frame.bytes.len();
                self.reply_frames -= 1;
            }
        }
        match result.status() {
            BackgroundInputStatus::Failed => self.fail(),
            BackgroundInputStatus::Closed => self.quiesce(),
            BackgroundInputStatus::Written | BackgroundInputStatus::Backpressure => {
                if result.stdin_closed() {
                    self.quiesce();
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn receipt(
        &self,
        writer: TerminalWriterId,
        operation: NonZeroU64,
    ) -> Result<TerminalInputReceipt> {
        self.receipt_with_actor(TerminalActorRole::Agent, writer, operation)
    }

    pub(crate) fn receipt_with_actor(
        &self,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        operation: NonZeroU64,
    ) -> Result<TerminalInputReceipt> {
        let writer = (actor, writer);
        self.receipts
            .iter()
            .find(|saved| saved.writer == writer && saved.receipt.operation_id == Some(operation))
            .map(|saved| saved.receipt)
            .ok_or(TerminalInputError::NotFound)
    }

    pub(crate) fn quiesce(&mut self) {
        if let Some(operation) = self.pending_operation.take()
            && let Some(saved) = self
                .receipts
                .iter_mut()
                .find(|saved| saved.receipt.operation_id == Some(operation))
        {
            saved.receipt.progress = TerminalInputProgress::Closed;
        }
        self.lease = None;
        self.quiesced = true;
        self.frames.clear();
        self.reply_bytes = 0;
        self.reply_frames = 0;
    }

    pub(crate) fn is_quiesced(&self) -> bool {
        self.quiesced
    }
    pub(crate) fn has_pending_bytes(&self) -> bool {
        !self.frames.is_empty()
    }

    fn fail(&mut self) {
        let pending = self.pending_operation;
        self.quiesce();
        if let Some(saved) = self
            .receipts
            .iter_mut()
            .find(|saved| saved.receipt.operation_id == pending)
        {
            saved.receipt.progress = TerminalInputProgress::Failed;
        }
    }
    pub(crate) fn check_cancel(
        &self,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
    ) -> Result<()> {
        if self
            .lease
            .is_some_and(|(role, holder)| role == actor && holder != writer)
        {
            Err(TerminalInputError::LeaseConflict)
        } else {
            Ok(())
        }
    }

    /// Attention cancellation revokes only this claimant's authority. The
    /// admitted payload, offset, and receipt remain owned by the transport.
    pub(crate) fn cancel_claim(&mut self, actor: TerminalActorRole, writer: TerminalWriterId) {
        if self.lease == Some((actor, writer)) {
            self.lease = None;
        }
    }

    fn check_holder(&self, writer: (TerminalActorRole, TerminalWriterId)) -> Result<()> {
        if self.lease.is_some_and(|holder| holder != writer) {
            Err(TerminalInputError::LeaseConflict)
        } else {
            Ok(())
        }
    }
}

/// Pinned native-session encoding: paste is byte-preserving, not an implicit
/// bracketed-paste wrapper; named key sequences do not depend on screen modes.
fn encode(payload: &TerminalWritePayload) -> Result<Vec<u8>> {
    payload
        .validate()
        .map_err(|_| TerminalInputError::Invalid)?;
    Ok(match payload {
        TerminalWritePayload::Text { text } | TerminalWritePayload::Paste { text } => {
            text.as_bytes().to_vec()
        }
        TerminalWritePayload::Controls { controls } => controls
            .iter()
            .map(|byte| {
                if *byte == b'?' {
                    0x7f
                } else {
                    byte.to_ascii_uppercase() & 0x1f
                }
            })
            .collect(),
        TerminalWritePayload::Keys { keys } => {
            let size = keys.iter().map(|key| key_sequence(*key).len()).sum();
            let mut bytes = Vec::with_capacity(size);
            for key in keys {
                bytes.extend_from_slice(key_sequence(*key));
            }
            bytes
        }
    })
}

fn key_sequence(key: TerminalNamedKey) -> &'static [u8] {
    match key {
        TerminalNamedKey::Enter => b"\r",
        TerminalNamedKey::Tab => b"\t",
        TerminalNamedKey::Escape => b"\x1b",
        TerminalNamedKey::Backspace => b"\x7f",
        TerminalNamedKey::Delete => b"\x1b[3~",
        TerminalNamedKey::Insert => b"\x1b[2~",
        TerminalNamedKey::ArrowUp => b"\x1b[A",
        TerminalNamedKey::ArrowDown => b"\x1b[B",
        TerminalNamedKey::ArrowLeft => b"\x1b[D",
        TerminalNamedKey::ArrowRight => b"\x1b[C",
        TerminalNamedKey::Home => b"\x1b[H",
        TerminalNamedKey::End => b"\x1b[F",
        TerminalNamedKey::PageUp => b"\x1b[5~",
        TerminalNamedKey::PageDown => b"\x1b[6~",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn writer(id: u64) -> TerminalWriterId {
        TerminalWriterId::new(NonZeroU64::new(id).unwrap())
    }
    fn lease(intent: TerminalWriteLeaseIntent) -> TerminalWriteRequest {
        TerminalWriteRequest {
            lease: intent,
            payload: None,
        }
    }
    fn text(text: &str) -> TerminalWriteRequest {
        TerminalWriteRequest {
            lease: TerminalWriteLeaseIntent::Use,
            payload: Some(TerminalWritePayload::Text { text: text.into() }),
        }
    }
    fn accepted(bytes: usize) -> BackgroundInputReceipt {
        BackgroundInputReceipt::new(bytes, false, BackgroundInputStatus::Written)
    }
    fn acquire(input: &mut TerminalInput, id: u64) {
        input
            .submit(writer(id), &lease(TerminalWriteLeaseIntent::Acquire), false)
            .unwrap();
    }

    #[test]
    fn exact_native_key_control_text_and_paste_encoding() {
        use TerminalNamedKey::*;
        let keys = vec![
            Enter, Tab, Escape, Backspace, Delete, Insert, ArrowUp, ArrowDown, ArrowLeft,
            ArrowRight, Home, End, PageUp, PageDown,
        ];
        assert_eq!(
            encode(&TerminalWritePayload::Keys { keys }).unwrap(),
            b"\r\t\x1b\x7f\x1b[3~\x1b[2~\x1b[A\x1b[B\x1b[D\x1b[C\x1b[H\x1b[F\x1b[5~\x1b[6~"
        );
        assert_eq!(
            encode(&TerminalWritePayload::Controls {
                controls: b"@cZ[\\]^_?".to_vec()
            })
            .unwrap(),
            [0, 3, 26, 27, 28, 29, 30, 31, 127]
        );
        for payload in [
            TerminalWritePayload::Text {
                text: "a\0界".into(),
            },
            TerminalWritePayload::Paste {
                text: "a\0界".into(),
            },
        ] {
            assert_eq!(encode(&payload).unwrap(), "a\0界".as_bytes());
        }
        assert_eq!(
            encode(&TerminalWritePayload::Controls { controls: vec![3] }),
            Err(TerminalInputError::Invalid)
        );
        assert_eq!(
            encode(&TerminalWritePayload::Text {
                text: "x".repeat(65537)
            }),
            Err(TerminalInputError::Invalid)
        );
    }

    #[test]
    fn leases_require_the_exact_holder_and_revoke_quiesces_every_input_source() {
        let mut input = TerminalInput::new();
        assert_eq!(
            input.submit(writer(1), &text("x"), false),
            Err(TerminalInputError::LeaseConflict)
        );
        acquire(&mut input, 1);
        acquire(&mut input, 1);
        for request in [
            lease(TerminalWriteLeaseIntent::Acquire),
            lease(TerminalWriteLeaseIntent::Release),
            text("x"),
        ] {
            assert_eq!(
                input.submit(writer(2), &request, false),
                Err(TerminalInputError::LeaseConflict)
            );
        }
        input
            .submit(writer(1), &lease(TerminalWriteLeaseIntent::Release), false)
            .unwrap();
        acquire(&mut input, 2);
        let receipt = input.submit(writer(2), &text("pending"), false).unwrap();
        input
            .submit(writer(1), &lease(TerminalWriteLeaseIntent::Revoke), false)
            .unwrap();
        assert_eq!(
            input
                .receipt(writer(2), receipt.operation_id.unwrap())
                .unwrap()
                .progress,
            TerminalInputProgress::Closed
        );
        input.flush(|_| panic!("revoked input reached backend"));
        assert_eq!(
            input.replies(vec![b"reply".to_vec()]),
            Err(TerminalInputError::Closed)
        );
        assert_eq!(
            input.submit(writer(1), &lease(TerminalWriteLeaseIntent::Acquire), false),
            Err(TerminalInputError::Closed)
        );
    }

    #[test]
    fn partial_writes_preserve_utf8_suffix_and_never_interleave_replies() {
        let mut input = TerminalInput::new();
        acquire(&mut input, 1);
        let receipt = input.submit(writer(1), &text("a界\0b"), false).unwrap();
        let id = receipt.operation_id.unwrap();
        let mut actual = Vec::new();
        input.flush(|bytes| {
            actual.push(bytes[0]);
            Ok(accepted(1))
        });
        input.replies(vec![b"REPLY".to_vec()]).unwrap();
        assert_eq!(
            input.submit(writer(1), &text("other"), false),
            Err(TerminalInputError::Busy)
        );
        assert_eq!(
            input.submit(writer(1), &lease(TerminalWriteLeaseIntent::Release), false),
            Err(TerminalInputError::Busy)
        );
        input.flush(|_| {
            Ok(BackgroundInputReceipt::new(
                0,
                false,
                BackgroundInputStatus::Backpressure,
            ))
        });
        assert_eq!(input.receipt(writer(1), id).unwrap().accepted_bytes, 1);
        for _ in 0..32 {
            input.flush(|bytes| {
                actual.push(bytes[0]);
                Ok(accepted(1))
            });
        }
        assert_eq!(actual, "a界\0bREPLY".as_bytes());
        let receipt = input.receipt(writer(1), id).unwrap();
        assert_eq!(receipt.accepted_bytes, "a界\0b".len());
        assert_eq!(receipt.progress, TerminalInputProgress::Complete);
        assert!(!input.has_pending_bytes());
        assert_eq!(
            input.receipt(writer(2), id),
            Err(TerminalInputError::NotFound)
        );
    }

    #[test]
    fn each_flush_is_one_eight_kib_call_and_attention_cancellation_does_not_rewind() {
        let mut input = TerminalInput::new();
        assert_eq!(
            input.submit(writer(1), &lease(TerminalWriteLeaseIntent::Acquire), true),
            Err(TerminalInputError::Cancelled)
        );
        assert!(input.lease.is_none());
        acquire(&mut input, 1);
        let payload = text(&"x".repeat(64 * 1024));
        let receipt = input.submit(writer(1), &payload, false).unwrap();
        assert_eq!(
            input.submit(writer(1), &text("cancelled"), true),
            Err(TerminalInputError::Cancelled)
        );
        let mut bytes_written = 0;
        for _ in 0..8 {
            input.flush(|bytes| {
                assert_eq!(bytes.len(), 8192);
                bytes_written += bytes.len();
                Ok(accepted(bytes.len()))
            });
        }
        assert_eq!(bytes_written, 65536);
        assert_eq!(
            input
                .receipt(writer(1), receipt.operation_id.unwrap())
                .unwrap()
                .progress,
            TerminalInputProgress::Complete
        );
        input.flush(|_| panic!("completed write repeated"));
    }

    #[test]
    fn reply_batches_are_atomic_and_resident_prefixes_still_count_against_capacity() {
        let mut input = TerminalInput::new();
        input.replies(vec![vec![b'x'; 256]; 16]).unwrap();
        assert_eq!(
            input.replies(vec![b"extra".to_vec()]),
            Err(TerminalInputError::Capacity)
        );
        input.flush(|_| Ok(accepted(1)));
        assert_eq!(input.reply_bytes, 4096);
        assert_eq!(
            input.replies(vec![b"extra".to_vec()]),
            Err(TerminalInputError::Capacity)
        );
        input.flush(|bytes| Ok(accepted(bytes.len())));
        assert_eq!(input.reply_bytes, 15 * 256);
        assert_eq!(
            input.replies(vec![b"good".to_vec(), vec![]]),
            Err(TerminalInputError::Invalid)
        );
        assert_eq!(input.reply_frames, 15);
        input.replies(vec![b"good".to_vec()]).unwrap();
    }

    #[test]
    fn closed_and_failed_writes_retain_exact_accepted_prefixes() {
        for status in [BackgroundInputStatus::Closed, BackgroundInputStatus::Failed] {
            let mut input = TerminalInput::new();
            acquire(&mut input, 1);
            let id = input
                .submit(writer(1), &text("abc"), false)
                .unwrap()
                .operation_id
                .unwrap();
            input.flush(|_| Ok(BackgroundInputReceipt::new(1, true, status)));
            let receipt = input.receipt(writer(1), id).unwrap();
            assert_eq!(receipt.accepted_bytes, 1);
            assert_eq!(
                receipt.progress,
                if status == BackgroundInputStatus::Closed {
                    TerminalInputProgress::Closed
                } else {
                    TerminalInputProgress::Failed
                }
            );
            input.flush(|_| panic!("terminal failure retried"));
        }
        let mut input = TerminalInput::new();
        acquire(&mut input, 1);
        let id = input
            .submit(writer(1), &text("abc"), false)
            .unwrap()
            .operation_id
            .unwrap();
        input.flush(|_| {
            Ok(BackgroundInputReceipt::new(
                3,
                true,
                BackgroundInputStatus::Closed,
            ))
        });
        assert_eq!(
            input.receipt(writer(1), id).unwrap().progress,
            TerminalInputProgress::Complete
        );
        assert!(input.is_quiesced());
    }

    #[test]
    fn receipt_retention_and_counter_exhaustion_are_bounded_and_debug_is_redacted() {
        let mut input = TerminalInput::new();
        acquire(&mut input, 1);
        for _ in 0..65 {
            input.submit(writer(1), &text("PRIVATE"), false).unwrap();
            input.flush(|bytes| Ok(accepted(bytes.len())));
        }
        assert_eq!(input.receipts.len(), 64);
        assert_eq!(
            input.receipt(writer(1), NonZeroU64::new(1).unwrap()),
            Err(TerminalInputError::NotFound)
        );
        input.next_operation = NonZeroU64::new(u64::MAX);
        input.submit(writer(1), &text("last"), false).unwrap();
        input.flush(|bytes| Ok(accepted(bytes.len())));
        assert_eq!(
            input.submit(writer(1), &text("overflow"), false),
            Err(TerminalInputError::Capacity)
        );
        assert!(!input.has_pending_bytes());
        assert_eq!(format!("{input:?}"), "TerminalInput { .. }");
        assert_eq!(format!("{:?}", writer(99)), "TerminalWriterId { .. }");
    }
}
