use super::composer::{Composer, ComposerContext, ComposerEvent};
use super::framing::{InteractiveInputFrameError, InteractiveInputFramer};
use machine_god_native::{
    NativeInteractiveInput, NativeInteractiveInputChunk, NativeInteractiveInputError,
    NativeInteractivePromptToken,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Sleep;

const ESCAPE_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone, Eq, PartialEq)]
pub(super) enum InputBinding {
    Command,
    AwaitingPrompt,
    AwaitingPicker {
        generation: u64,
    },
    Picker {
        generation: u64,
        revision: u64,
    },
    Prompt {
        token: NativeInteractivePromptToken,
        question: usize,
    },
}

impl InputBinding {
    pub fn picker_view(&self) -> Option<(u64, Option<u64>)> {
        match self {
            Self::Picker {
                generation,
                revision,
            } => Some((*generation, Some(*revision))),
            Self::AwaitingPicker { generation } => Some((*generation, None)),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(super) enum LineError {
    Frame(InteractiveInputFrameError),
    Input(NativeInteractiveInputError),
}

/// Bytes already received retain their original presentation identity, including
/// later lines in the same chunk and a partial line spanning later chunks.
pub(super) struct InputLines {
    pub input: NativeInteractiveInput,
    framer: InteractiveInputFramer,
    composer: Option<Composer>,
    chunk: Option<NativeInteractiveInputChunk>,
    offset: usize,
    chunk_binding: InputBinding,
    line_binding: Option<InputBinding>,
    ended: bool,
    cancel_disarm: bool,
    escape_timer: Option<Pin<Box<Sleep>>>,
}

impl InputLines {
    pub fn new(input: NativeInteractiveInput) -> Self {
        Self {
            input,
            framer: InteractiveInputFramer::default(),
            composer: None,
            chunk: None,
            offset: 0,
            chunk_binding: InputBinding::Command,
            line_binding: None,
            ended: false,
            cancel_disarm: false,
            escape_timer: None,
        }
    }

    /// Raw editing uses the same exact input allocation and chunk ownership;
    /// construction does not poll, acquire, or reconfigure that input.
    pub fn new_raw(input: NativeInteractiveInput) -> Self {
        Self {
            composer: Some(Composer::default()),
            ..Self::new(input)
        }
    }

    /// Returns raw draft text and UTF-8 byte cursor, or None in canonical mode.
    pub fn raw_draft(&self) -> Option<(&str, usize)> {
        self.composer
            .as_ref()
            .map(|composer| (composer.text(), composer.cursor()))
    }

    /// Restore presentation-owned search after ignoring stale bytes. The
    /// remainder of an already received chunk retains its original identity.
    pub fn restore_picker_query(&mut self, query: &str) -> Result<(), ()> {
        self.composer
            .as_mut()
            .ok_or(())?
            .restore_picker_query(query)?;
        self.line_binding = None;
        Ok(())
    }

    /// Consumed ordinary raw input disarms a pending repeated-Ctrl-C gesture,
    /// even if the composer intentionally emits no event for that input.
    pub fn take_cancel_disarm(&mut self) -> bool {
        std::mem::take(&mut self.cancel_disarm)
    }

    /// Explicitly discards the raw draft and decoder. Already received bytes
    /// still retain their old chunk binding; reset cannot relabel pasted input.
    pub fn reset_raw_draft(&mut self) {
        if let Some(composer) = &mut self.composer {
            composer.reset();
            self.line_binding = None;
            self.escape_timer = None;
        }
    }

    pub fn poll_line(
        &mut self,
        cx: &mut Context<'_>,
        binding: InputBinding,
    ) -> Poll<Option<Result<(String, InputBinding), LineError>>> {
        self.poll_line_recorded(cx, binding, |_| {})
    }

    /// Observe each exact native chunk once, before framing or rejected-input
    /// transformations. The observer must remain bounded and nonblocking.
    pub fn poll_line_recorded(
        &mut self,
        cx: &mut Context<'_>,
        binding: InputBinding,
        mut received: impl FnMut(&[u8]),
    ) -> Poll<Option<Result<(String, InputBinding), LineError>>> {
        if self.composer.is_some() {
            return Poll::Ready(Some(Err(LineError::Input(
                NativeInteractiveInputError::Unavailable,
            ))));
        }
        if self.ended {
            return Poll::Ready(None);
        }
        if self.chunk.is_none() {
            match self.input.poll_chunk(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.ended = true;
                    return Poll::Ready(Some(Err(LineError::Input(error))));
                }
                Poll::Ready(Ok(None)) => {
                    self.ended = true;
                    let context = self.line_binding.take().unwrap_or(binding);
                    return Poll::Ready(self.framer.finish().map(|result| {
                        result.map(|line| (line, context)).map_err(LineError::Frame)
                    }));
                }
                Poll::Ready(Ok(Some(chunk))) => {
                    received(chunk.as_bytes());
                    self.chunk = Some(chunk);
                    self.offset = 0;
                    self.chunk_binding = binding;
                }
            }
        }
        let chunk = self.chunk.as_ref().expect("a received chunk");
        self.line_binding
            .get_or_insert_with(|| self.chunk_binding.clone());
        let (consumed, frame) = self.framer.feed(&chunk.as_bytes()[self.offset..]);
        self.offset += consumed;
        if self.offset == chunk.as_bytes().len() {
            self.chunk.take();
        }
        // Only one bounded chunk is processed per poll. The next poll either
        // processes its remainder or issues the next demand, without busy idle IO.
        cx.waker().wake_by_ref();
        match frame {
            Some(result) => {
                let context = self.line_binding.take().expect("line has an input binding");
                Poll::Ready(Some(
                    result.map(|line| (line, context)).map_err(LineError::Frame),
                ))
            }
            None => Poll::Pending,
        }
    }

    /// At most one bounded received chunk and one composer event per poll.
    /// Picker events start a new epoch for newly received chunks, but partial
    /// UTF-8/paste and already received chunk remainders keep their first epoch.
    /// Ordinary command/prompt drafts retain their original modal identity.
    /// Physical EOF never submits the retained draft, unlike canonical finish.
    pub fn poll_event(
        &mut self,
        cx: &mut Context<'_>,
        binding: InputBinding,
        context: ComposerContext,
    ) -> Poll<Option<Result<(ComposerEvent, InputBinding), LineError>>> {
        self.poll_event_recorded(cx, binding, context, |_| {})
    }

    pub fn poll_event_recorded(
        &mut self,
        cx: &mut Context<'_>,
        binding: InputBinding,
        context: ComposerContext,
        mut received: impl FnMut(&[u8]),
    ) -> Poll<Option<Result<(ComposerEvent, InputBinding), LineError>>> {
        if self.composer.is_none() {
            return Poll::Ready(Some(Err(LineError::Input(
                NativeInteractiveInputError::Unavailable,
            ))));
        }
        if self.ended {
            return Poll::Ready(None);
        }
        if self
            .escape_timer
            .as_mut()
            .is_some_and(|timer| timer.as_mut().poll(cx).is_ready())
        {
            self.escape_timer = None;
            if let Some(event) = self
                .composer
                .as_mut()
                .expect("raw mode checked")
                .expire_escape()
            {
                let binding = self
                    .line_binding
                    .as_ref()
                    .expect("escape has an input binding")
                    .clone();
                if binding.picker_view().is_some() {
                    self.line_binding = None;
                }
                return Poll::Ready(Some(Ok((event, binding))));
            }
        }
        if self.chunk.is_none() {
            match self.input.poll_chunk(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.ended = true;
                    return Poll::Ready(Some(Err(LineError::Input(error))));
                }
                Poll::Ready(Ok(None)) => {
                    self.ended = true;
                    return Poll::Ready(None);
                }
                Poll::Ready(Ok(Some(chunk))) => {
                    received(chunk.as_bytes());
                    self.chunk = Some(chunk);
                    self.offset = 0;
                    self.chunk_binding = binding;
                }
            }
        }
        self.line_binding
            .get_or_insert_with(|| self.chunk_binding.clone());
        let chunk = self.chunk.as_ref().expect("a received chunk");
        let composer = self.composer.as_mut().expect("raw mode checked");
        let effective_context = ComposerContext {
            session_picker: self
                .line_binding
                .as_ref()
                .is_some_and(|binding| binding.picker_view().is_some()),
            ..context
        };
        let (consumed, event) = composer.feed(&chunk.as_bytes()[self.offset..], effective_context);
        if composer.waiting_for_escape() {
            if self.escape_timer.is_none() {
                self.escape_timer = Some(Box::pin(tokio::time::sleep(ESCAPE_TIMEOUT)));
            }
        } else {
            self.escape_timer = None;
        }
        self.cancel_disarm |= chunk.as_bytes()[self.offset..self.offset + consumed]
            .iter()
            .any(|byte| !matches!(byte, 3 | 27));
        self.offset += consumed;
        if self.offset == chunk.as_bytes().len() {
            self.chunk.take();
        }
        // A consumed prefix creates bounded follow-up work, not an idle poll.
        if consumed > 0 {
            cx.waker().wake_by_ref();
        }
        let Some(event) = event else {
            return Poll::Pending;
        };
        let binding = self
            .line_binding
            .as_ref()
            .expect("received-byte binding")
            .clone();
        if matches!(
            event,
            ComposerEvent::Submit(_) | ComposerEvent::ExitRequested
        ) || matches!(event, ComposerEvent::CancelRequested) && !context.active_response
            || binding.picker_view().is_some() && !composer.has_pending_input()
            || matches!(event, ComposerEvent::SessionPickerRequested)
                && matches!(binding, InputBinding::Command)
        {
            self.line_binding = None;
        }
        Poll::Ready(Some(Ok((event, binding))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::CancellationToken;
    use machine_god_native::NativeInteractiveInputSource;
    use std::{future::poll_fn, io::Write, os::fd::OwnedFd, time::Duration};

    fn source() -> (InputLines, std::io::PipeWriter) {
        let (read, write) = std::io::pipe().unwrap();
        let input = NativeInteractiveInput::new(
            NativeInteractiveInputSource::AdoptNonblockingStatus(OwnedFd::from(read).into()),
            CancellationToken::new(),
        );
        (InputLines::new(input), write)
    }

    async fn line(input: &mut InputLines, binding: InputBinding) -> (String, InputBinding) {
        let value = tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| input.poll_line(cx, binding.clone())),
        )
        .await
        .unwrap()
        .expect("line before EOF");
        value.expect("valid input line")
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn raw_source() -> (InputLines, std::io::PipeWriter) {
        let (canonical, write) = source();
        (InputLines::new_raw(canonical.input), write)
    }

    #[test]
    fn restored_picker_query_never_relabels_remaining_old_scope_bytes() {
        let (mut input, mut write) = raw_source();
        runtime().block_on(async {
            let old = InputBinding::Picker {
                generation: 1,
                revision: 1,
            };
            let new = InputBinding::Picker {
                generation: 2,
                revision: 0,
            };
            write.write_all(b"query\t\r").unwrap();
            let _ = event(&mut input, old.clone(), false).await;
            assert!(matches!(
                event(&mut input, old.clone(), false).await.0,
                ComposerEvent::PickerToggleScope
            ));
            input.restore_picker_query("query").unwrap();
            let (event, binding) = event(&mut input, new, false).await;
            assert!(matches!(event, ComposerEvent::Submit(text) if text == "query"));
            assert!(binding == old);
            assert_eq!(input.raw_draft().unwrap().0, "query");
            assert!(input.restore_picker_query(&"x".repeat(257)).is_err());
            assert_eq!(input.raw_draft().unwrap().0, "query");
        });
        finish(input);
    }

    fn prompt_binding() -> InputBinding {
        use machine_god_core::{
            BackgroundOutputOwner, Capability, PermissionRequest, PermissionRequestId,
            PermissionRisk, SessionId, SessionIncarnationId, TurnId,
        };
        use machine_god_native::{
            NativeInteractivePromptBridge, NativeInteractivePromptLimits, PermissionPrompter,
        };
        let (bridge, mut inbox) =
            NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
        let session = SessionId::new("binding-session").unwrap();
        let incarnation = SessionIncarnationId::new("binding-incarnation").unwrap();
        inbox
            .activate(BackgroundOutputOwner::new(
                session.clone(),
                incarnation.clone(),
            ))
            .unwrap();
        let mut prompt = bridge.prompt(PermissionRequest {
            id: PermissionRequestId::new("binding-request").unwrap(),
            session_id: session,
            session_incarnation_id: incarnation,
            turn_id: TurnId::new("binding-turn").unwrap(),
            capability: Capability::Filesystem {
                access: machine_god_core::FilesystemAccess::Read,
                path: "fixture".into(),
            },
            risk: PermissionRisk::Low,
            reason: "fixture".into(),
        });
        let mut cx = Context::from_waker(std::task::Waker::noop());
        assert!(prompt.as_mut().poll(&mut cx).is_pending());
        let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut cx) else {
            panic!("prompt view");
        };
        InputBinding::Prompt {
            token: view.token().clone(),
            question: 0,
        }
    }

    #[test]
    fn raw_draft_keeps_exact_prompt_token_and_page_after_display_changes() {
        let first = prompt_binding();
        let InputBinding::Prompt { token, .. } = &first else {
            unreachable!();
        };
        let next_page = InputBinding::Prompt {
            token: token.clone(),
            question: 1,
        };
        let replacement = prompt_binding();
        let (mut input, mut write) = raw_source();
        write.write_all(b"par").unwrap();
        runtime().block_on(async {
            assert!(event(&mut input, first.clone(), false).await.1 == first);
            write.write_all(b"tial\n").unwrap();
            assert!(event(&mut input, next_page, false).await.1 == first);
            let result = event(&mut input, replacement, false).await;
            assert!(matches!(result.0, ComposerEvent::Submit(text) if text == "partial"));
            assert!(result.1 == first);
        });
        finish(input);
    }

    fn picker(revision: u64) -> InputBinding {
        InputBinding::Picker {
            generation: 7,
            revision,
        }
    }

    #[test]
    fn picker_events_refresh_new_chunks_but_never_relabel_received_remainders() {
        let (mut input, mut write) = raw_source();
        runtime().block_on(async {
            write.write_all(b"\x1b[B\r").unwrap();
            let navigation = event(&mut input, picker(1), false).await;
            assert!(matches!(navigation.0, ComposerEvent::PickerNext));
            assert!(navigation.1 == picker(1));
            let stale_submit = event(&mut input, picker(2), false).await;
            assert!(matches!(stale_submit.0, ComposerEvent::Submit(_)));
            assert!(stale_submit.1 == picker(1));
            write.write_all(b"query").unwrap();
            let changed = event(&mut input, picker(2), false).await;
            assert!(matches!(changed.0, ComposerEvent::Changed));
            assert!(changed.1 == picker(2));
            write.write_all(b"\r").unwrap();
            let submit = event(&mut input, picker(3), false).await;
            assert!(matches!(submit.0, ComposerEvent::Submit(text) if text == "query"));
            assert!(submit.1 == picker(3));
        });
        finish(input);
    }

    #[test]
    fn command_chunk_cannot_become_picker_selection_after_shortcut() {
        let (mut input, mut write) = raw_source();
        runtime().block_on(async {
            write.write_all(b"\x1b[114;9u\r").unwrap();
            assert!(matches!(
                event(&mut input, InputBinding::Command, false).await,
                (ComposerEvent::SessionPickerRequested, InputBinding::Command)
            ));
            input.reset_raw_draft();
            assert!(matches!(
                event(&mut input, picker(1), false).await,
                (ComposerEvent::Submit(_), InputBinding::Command)
            ));
            write.write_all(b"\r").unwrap();
            let fresh = event(&mut input, picker(1), false).await;
            assert!(matches!(fresh.0, ComposerEvent::Submit(_)));
            assert!(fresh.1 == picker(1));
        });
        finish(input);
    }

    #[test]
    fn picker_split_sequences_and_paste_keep_first_received_view() {
        for (first, second, expected) in [
            (b"\x1b[".as_slice(), b"1;5A".as_slice(), "PickerPrevious"),
            (b"\xf0\x9f", b"\xa6\x80", "Changed"),
            (b"\x1b[200~query", b"\x1b[201~", "Changed"),
        ] {
            let (mut input, mut write) = raw_source();
            runtime().block_on(async {
                write.write_all(first).unwrap();
                partial(&mut input, picker(1)).await;
                write.write_all(second).unwrap();
                let result = event(&mut input, picker(2), false).await;
                assert_eq!(format!("{:?}", result.0), expected);
                assert!(result.1 == picker(1));
                write.write_all(b"\t").unwrap();
                let fresh = event(&mut input, picker(2), false).await;
                assert!(matches!(fresh.0, ComposerEvent::PickerToggleScope));
                assert!(fresh.1 == picker(2));
            });
            finish(input);
        }
    }

    #[test]
    fn standalone_escape_waits_for_timer_and_keeps_its_original_picker_view() {
        let (mut input, mut write) = raw_source();
        runtime().block_on(async {
            write.write_all(b"\x1b").unwrap();
            partial(&mut input, picker(1)).await;
            assert!(input.escape_timer.is_some());
            let escape = event(&mut input, picker(2), false).await;
            assert!(matches!(escape.0, ComposerEvent::EscapeRequested));
            assert!(escape.1 == picker(1));
            assert!(input.escape_timer.is_none());
            assert!(input.line_binding.is_none());
            write.write_all(b"\x1b").unwrap();
            partial(&mut input, picker(2)).await;
            write.write_all(b"[A").unwrap();
            let arrow = event(&mut input, picker(3), false).await;
            assert!(matches!(arrow.0, ComposerEvent::PickerPrevious));
            assert!(arrow.1 == picker(2));
            assert!(input.escape_timer.is_none());
        });
        finish(input);
    }

    #[test]
    fn ignored_picker_shortcut_and_escape_do_not_retarget_a_modal_draft() {
        let (mut input, mut write) = raw_source();
        let original = prompt_binding();
        let replacement = prompt_binding();
        runtime().block_on(async {
            write.write_all(b"draft").unwrap();
            let _ = event(&mut input, original.clone(), false).await;
            write.write_all(b"\x1b[114;9u").unwrap();
            let shortcut = event(&mut input, replacement.clone(), false).await;
            assert!(matches!(shortcut.0, ComposerEvent::SessionPickerRequested));
            assert!(shortcut.1 == original);
            write.write_all(b"\x1b").unwrap();
            let escape = event(&mut input, replacement.clone(), false).await;
            assert!(matches!(escape.0, ComposerEvent::EscapeRequested));
            assert!(escape.1 == original);
            write.write_all(b"\r").unwrap();
            let submitted = event(&mut input, replacement, false).await;
            assert!(matches!(submitted.0, ComposerEvent::Submit(text) if text == "draft"));
            assert!(submitted.1 == original);
        });
        finish(input);
    }

    async fn event(
        input: &mut InputLines,
        binding: InputBinding,
        active: bool,
    ) -> (ComposerEvent, InputBinding) {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                input.poll_event(
                    cx,
                    binding.clone(),
                    ComposerContext {
                        active_response: active,
                        session_picker: matches!(binding, InputBinding::Picker { .. }),
                    },
                )
            }),
        )
        .await
        .unwrap()
        .expect("event before EOF")
        .expect("valid native input")
    }

    fn finish(input: InputLines) {
        let completion = input.input.completion();
        drop(input);
        completion.wait_on_worker().unwrap();
    }

    async fn partial(input: &mut InputLines, binding: InputBinding) {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                assert!(
                    input
                        .poll_event(cx, binding.clone(), ComposerContext::default())
                        .is_pending()
                );
                if input.line_binding.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
    }

    #[test]
    fn raw_partial_draft_keeps_original_binding_through_changed_and_submit() {
        let (mut input, mut write) = raw_source();
        write.write_all(b"par").unwrap();
        runtime().block_on(async {
            let changed = event(&mut input, InputBinding::AwaitingPrompt, false).await;
            assert!(matches!(changed, (ComposerEvent::Changed, InputBinding::AwaitingPrompt)));
            assert_eq!(input.raw_draft(), Some(("par", 3)));
            write.write_all(b"tial\n").unwrap();
            let changed = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(changed, (ComposerEvent::Changed, InputBinding::AwaitingPrompt)));
            let submitted = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(submitted, (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == "partial"));
        });
        finish(input);
    }

    #[test]
    fn raw_received_chunk_remainder_and_reset_cannot_relabel_old_bytes() {
        let (mut input, mut write) = raw_source();
        write.write_all(b"one\ntwo\n").unwrap();
        runtime().block_on(async {
            let _ = event(&mut input, InputBinding::AwaitingPrompt, false).await;
            let first = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(first, (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == "one"));
            input.reset_raw_draft();
            let changed = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(changed, (ComposerEvent::Changed, InputBinding::AwaitingPrompt)));
            let second = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(second, (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == "two"));
            write.write_all(b"fresh\n").unwrap();
            let changed = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(changed, (ComposerEvent::Changed, InputBinding::Command)));
        });
        finish(input);
    }

    #[test]
    fn raw_utf8_and_atomic_paste_retain_first_binding_even_with_empty_visible_draft() {
        for (first, last, expected) in [
            (b"\xf0\x9f".as_slice(), b"\xa6\x80\n".as_slice(), "🦀"),
            (b"\x1b[200~secret", b"\x1b[201~\n", "secret"),
        ] {
            let (mut input, mut write) = raw_source();
            write.write_all(first).unwrap();
            runtime().block_on(async {
                partial(&mut input, InputBinding::AwaitingPrompt).await;
                assert_eq!(input.raw_draft(), Some(("", 0)));
                write.write_all(last).unwrap();
                let changed = event(&mut input, InputBinding::Command, false).await;
                assert!(matches!(changed, (ComposerEvent::Changed, InputBinding::AwaitingPrompt)));
                let submitted = event(&mut input, InputBinding::Command, false).await;
                assert!(matches!(submitted, (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == expected));
            });
            finish(input);
        }
    }

    #[test]
    fn raw_rejected_input_recovers_without_retargeting_or_submitting_a_prefix() {
        let (mut input, mut write) = raw_source();
        write.write_all(b"keep\0ignored\n").unwrap();
        runtime().block_on(async {
            let _ = event(&mut input, InputBinding::AwaitingPrompt, false).await;
            let rejected = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(rejected, (ComposerEvent::InputError(_), InputBinding::AwaitingPrompt)));
            let recovered = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(recovered, (ComposerEvent::Changed, InputBinding::AwaitingPrompt)));
            assert_eq!(input.raw_draft(), Some(("keep", 4)));
            write.write_all(b"\n").unwrap();
            let submitted = event(&mut input, InputBinding::Command, false).await;
            assert!(matches!(submitted, (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == "keep"));
        });
        finish(input);
    }

    #[test]
    fn raw_ignored_active_ctrl_d_disarms_but_unconsumed_bytes_do_not() {
        let (mut input, mut write) = raw_source();
        assert!(!input.take_cancel_disarm());
        write.write_all(b"\x04").unwrap();
        runtime().block_on(async {
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    assert!(
                        input
                            .poll_event(
                                cx,
                                InputBinding::Command,
                                ComposerContext {
                                    active_response: true,
                                    session_picker: false,
                                }
                            )
                            .is_pending()
                    );
                    if input.take_cancel_disarm() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }),
            )
            .await
            .unwrap();
            assert!(!input.take_cancel_disarm());
            assert_eq!(input.raw_draft(), Some(("", 0)));
            write.write_all(b"\x03x\x03").unwrap();
            assert!(matches!(
                event(&mut input, InputBinding::Command, true).await.0,
                ComposerEvent::CancelRequested
            ));
            assert!(
                !input.take_cancel_disarm(),
                "unconsumed x must not disarm first Ctrl-C"
            );
            assert!(matches!(
                event(&mut input, InputBinding::Command, true).await.0,
                ComposerEvent::Changed
            ));
            assert!(input.take_cancel_disarm());
            assert!(!input.take_cancel_disarm());
            assert!(matches!(
                event(&mut input, InputBinding::Command, true).await.0,
                ComposerEvent::CancelRequested
            ));
            assert!(!input.take_cancel_disarm());
            input.reset_raw_draft();
            write.write_all(b"\x1b").unwrap();
            partial(&mut input, InputBinding::Command).await;
            assert!(!input.take_cancel_disarm());
        });
        finish(input);
    }

    #[test]
    fn raw_ctrl_d_is_not_eof_and_idle_ctrl_c_only_clears_the_draft_binding() {
        let (mut input, mut write) = raw_source();
        runtime().block_on(async {
            write.write_all(&[4]).unwrap();
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    assert!(
                        input
                            .poll_event(
                                cx,
                                InputBinding::AwaitingPrompt,
                                ComposerContext {
                                    active_response: true,
                                    session_picker: false,
                                }
                            )
                            .is_pending()
                    );
                    if input.line_binding.is_some() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }),
            )
            .await
            .unwrap();
            assert!(!input.ended);
            write.write_all(&[4]).unwrap();
            assert!(matches!(
                event(&mut input, InputBinding::Command, false).await,
                (ComposerEvent::ExitRequested, InputBinding::AwaitingPrompt)
            ));
            assert!(!input.ended);
            write.write_all(b"draft\x03tail\n").unwrap();
            let _ = event(&mut input, InputBinding::AwaitingPrompt, false).await;
            assert!(matches!(
                event(&mut input, InputBinding::Command, false).await,
                (ComposerEvent::CancelRequested, InputBinding::AwaitingPrompt)
            ));
            assert_eq!(input.raw_draft(), Some(("", 0)));
            let _ = event(&mut input, InputBinding::Command, false).await;
            assert!(
                matches!(event(&mut input, InputBinding::Command, false).await,
                (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == "tail")
            );
        });
        finish(input);
    }

    #[test]
    fn raw_active_cancellation_preserves_existing_draft_and_binding() {
        let (mut input, mut write) = raw_source();
        write.write_all(b"keep\x03").unwrap();
        runtime().block_on(async {
            let _ = event(&mut input, InputBinding::AwaitingPrompt, true).await;
            assert!(matches!(
                event(&mut input, InputBinding::Command, true).await,
                (ComposerEvent::CancelRequested, InputBinding::AwaitingPrompt)
            ));
            assert_eq!(input.raw_draft(), Some(("keep", 4)));
            write.write_all(b"\n").unwrap();
            assert!(
                matches!(event(&mut input, InputBinding::Command, false).await,
                (ComposerEvent::Submit(text), InputBinding::AwaitingPrompt) if text == "keep")
            );
        });
        finish(input);
    }

    #[test]
    fn raw_eof_never_submits_draft_and_ended_poll_does_not_self_wake() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::task::{Wake, Waker};
        #[derive(Default)]
        struct Counter(AtomicUsize);
        impl Wake for Counter {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let (mut input, mut write) = raw_source();
        write.write_all(b"unfinished").unwrap();
        runtime().block_on(async {
            let _ = event(&mut input, InputBinding::Command, false).await;
            drop(write);
            let eof = tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    input.poll_event(cx, InputBinding::Command, ComposerContext::default())
                }),
            )
            .await
            .unwrap();
            assert!(eof.is_none());
            assert_eq!(input.raw_draft(), Some(("unfinished", 10)));
            let counter = Arc::new(Counter::default());
            let waker = Waker::from(counter.clone());
            for _ in 0..3 {
                assert!(matches!(
                    input.poll_event(
                        &mut Context::from_waker(&waker),
                        InputBinding::Command,
                        ComposerContext::default()
                    ),
                    Poll::Ready(None)
                ));
            }
            assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        });
        finish(input);
    }

    #[test]
    fn mode_mismatch_is_explicit_and_does_not_read_or_switch_the_input() {
        let (mut canonical, _first_write) = source();
        let (mut raw, _second_write) = raw_source();
        let mut cx = Context::from_waker(std::task::Waker::noop());
        assert!(matches!(
            canonical.poll_event(&mut cx, InputBinding::Command, ComposerContext::default()),
            Poll::Ready(Some(Err(LineError::Input(
                NativeInteractiveInputError::Unavailable
            ))))
        ));
        assert!(matches!(
            raw.poll_line(&mut cx, InputBinding::Command),
            Poll::Ready(Some(Err(LineError::Input(
                NativeInteractiveInputError::Unavailable
            ))))
        ));
        assert!(canonical.raw_draft().is_none());
        assert_eq!(raw.raw_draft(), Some(("", 0)));
        assert!(canonical.chunk.is_none() && raw.chunk.is_none());
        finish(canonical);
        finish(raw);
    }

    #[test]
    fn canonical_eof_and_frame_errors_keep_the_existing_contract() {
        let (mut input, mut write) = source();
        write.write_all(b"\0\npartial").unwrap();
        drop(write);
        runtime().block_on(async {
            let rejected = tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| input.poll_line(cx, InputBinding::Command)),
            )
            .await
            .unwrap();
            assert!(matches!(
                rejected,
                Some(Err(LineError::Frame(
                    InteractiveInputFrameError::ContainsNul
                )))
            ));
            let (text, _) = line(&mut input, InputBinding::Command).await;
            assert_eq!(text, "partial");
            assert!(matches!(
                input.poll_line(
                    &mut Context::from_waker(std::task::Waker::noop()),
                    InputBinding::Command
                ),
                Poll::Ready(None)
            ));
        });
        finish(input);
    }

    #[test]
    fn received_chunk_remainder_keeps_its_original_binding() {
        let (mut input, mut write) = source();
        write.write_all(b"one\ntwo\n").unwrap();
        runtime().block_on(async {
            let first = line(&mut input, InputBinding::Command).await;
            assert_eq!(first.0, "one");
            assert!(matches!(first.1, InputBinding::Command));
            let second = line(&mut input, InputBinding::AwaitingPrompt).await;
            assert_eq!(second.0, "two");
            assert!(matches!(second.1, InputBinding::Command));
        });
        let completion = input.input.completion();
        drop(input);
        completion.wait_on_worker().unwrap();
    }

    #[test]
    fn partial_line_crossing_a_new_page_keeps_the_first_byte_binding() {
        let (mut input, mut write) = source();
        write.write_all(b"par").unwrap();
        runtime().block_on(async {
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    assert!(
                        input
                            .poll_line(cx, InputBinding::AwaitingPrompt)
                            .is_pending()
                    );
                    if input.line_binding.is_some() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }),
            )
            .await
            .unwrap();
            write.write_all(b"tial\n").unwrap();
            let result = line(&mut input, InputBinding::Command).await;
            assert_eq!(result.0, "partial");
            assert!(matches!(result.1, InputBinding::AwaitingPrompt));
        });
        let completion = input.input.completion();
        drop(input);
        completion.wait_on_worker().unwrap();
    }
}
