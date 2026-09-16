//! Three bounded presentation editors: parent, agent route, and modal prompt.
//! Stored decoders retain their original line provenance; chunks are never relabeled.
use super::{Composer, InputBinding, InputLines, Pin, Sleep};
use super::{ComposerContext, ComposerEvent, Context, LineError, Poll};
use machine_god_native::NativeManagedEditorIdentity;
use std::future::Future;

struct Parked {
    composer: Composer,
    binding: Option<InputBinding>,
    escape: Option<Pin<Box<Sleep>>>,
}
pub(super) struct Editors {
    parent: Parked,
    active: Option<NativeManagedEditorIdentity>,
    agent: Option<(NativeManagedEditorIdentity, Parked)>,
}
pub(super) struct Retired {
    editor: Parked,
    binding: InputBinding,
}

impl InputLines {
    pub(in crate::ask::production::interactive) fn seed_managed_form(
        &mut self,
        editor: &NativeManagedEditorIdentity,
        text: &str,
    ) -> Result<bool, ()> {
        if self
            .managed_editors
            .as_ref()
            .and_then(|editors| editors.active.as_ref())
            != Some(editor)
            || self
                .composer
                .as_ref()
                .is_none_or(Composer::has_pending_input)
        {
            return Ok(false);
        }
        let composer = self.composer.as_mut().ok_or(())?;
        composer
            .replace(0..composer.text().len(), text, text.len())
            .map_err(|_| ())?;
        Ok(true)
    }
    pub(super) fn managed_atomic_binding(&self, binding: InputBinding) -> InputBinding {
        // A modal/route transfer cannot split a paste or UTF-8 sequence.
        // Finish it under the original editor before changing owners.
        if self.managed_editors.is_some()
            && self
                .composer
                .as_ref()
                .is_some_and(Composer::has_pending_input)
            && let Some(original) = &self.line_binding
        {
            return original.clone();
        }
        binding
    }

    fn park_current(&mut self) -> Option<Parked> {
        Some(Parked {
            composer: self.composer.replace(Composer::default())?,
            binding: self.line_binding.take(),
            escape: self.escape_timer.take(),
        })
    }
    fn restore_parked(&mut self, mut parked: Parked) {
        parked.composer.resume_editor();
        self.composer = Some(parked.composer);
        self.line_binding = parked.binding;
        self.escape_timer = parked.escape;
    }

    pub(in crate::ask::production::interactive) fn open_managed_editor(
        &mut self,
        editor: NativeManagedEditorIdentity,
    ) -> Result<(), ()> {
        if self.managed_editors.is_some()
            || self.parked_composer.is_some()
            || self.retired_editor.is_some()
            || self
                .composer
                .as_ref()
                .is_none_or(Composer::has_pending_input)
        {
            return Err(());
        }
        let parent = self.park_current().ok_or(())?;
        self.managed_editors = Some(Editors {
            parent,
            active: Some(editor),
            agent: None,
        });
        Ok(())
    }

    pub(in crate::ask::production::interactive) fn sync_managed_editor(
        &mut self,
        desired: Option<&NativeManagedEditorIdentity>,
    ) {
        let Some(mut editors) = self.managed_editors.take() else {
            return;
        };
        if self
            .composer
            .as_ref()
            .is_some_and(Composer::has_pending_input)
        {
            self.managed_editors = Some(editors);
            return;
        }
        if editors.active.as_ref() != desired {
            if let Some(active) = editors.active.take() {
                if desired.is_none() {
                    editors.agent = self.park_current().map(|parked| (active, parked));
                } else {
                    self.composer = Some(Composer::default());
                    self.line_binding = None;
                    self.escape_timer = None;
                }
            }
            if let Some(desired) = desired {
                match editors.agent.take() {
                    Some((identity, parked)) if &identity == desired => self.restore_parked(parked),
                    _ => {
                        self.composer = Some(Composer::default());
                        self.line_binding = None;
                        self.escape_timer = None;
                    }
                }
                editors.active = Some(desired.clone());
            }
        }
        self.managed_editors = Some(editors);
    }

    pub(in crate::ask::production::interactive) fn close_managed_editor(
        &mut self,
        restore_parent: bool,
    ) {
        let Some(editors) = self.managed_editors.take() else {
            return;
        };
        if self
            .composer
            .as_ref()
            .is_some_and(Composer::has_pending_input)
        {
            let binding = self
                .line_binding
                .clone()
                .unwrap_or(InputBinding::AwaitingPrompt);
            if let Some(editor) = self.park_current() {
                self.retired_editor = Some(Retired { editor, binding });
            }
        }
        if restore_parent {
            self.restore_parked(editors.parent);
        } else {
            self.reset_raw_draft();
        }
    }

    pub(super) fn drain_retired_editor(
        &mut self,
        cx: &mut Context<'_>,
        received: &mut impl FnMut(&[u8]),
    ) -> Poll<Option<Result<(ComposerEvent, InputBinding), LineError>>> {
        let retired = self.retired_editor.as_mut().expect("retired decoder");
        if retired
            .editor
            .escape
            .as_mut()
            .is_some_and(|timer| timer.as_mut().poll(cx).is_ready())
        {
            retired.editor.composer.expire_escape();
            let retired = self.retired_editor.take().unwrap();
            self.chunk.take();
            return Poll::Ready(Some(Ok((ComposerEvent::StaleInput, retired.binding))));
        }
        if self.chunk.is_none() {
            match self.input.poll_chunk(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.ended = true;
                    self.retired_editor.take();
                    return Poll::Ready(Some(Err(LineError::Input(error))));
                }
                Poll::Ready(Ok(None)) => {
                    self.ended = true;
                    self.retired_editor.take();
                    return Poll::Ready(None);
                }
                Poll::Ready(Ok(Some(chunk))) => {
                    received(chunk.as_bytes());
                    self.chunk = Some(chunk);
                    self.offset = 0;
                }
            }
        }
        let retired = self.retired_editor.as_mut().expect("original decoder");
        let chunk = self.chunk.as_ref().expect("received chunk");
        let (consumed, _) = retired.editor.composer.feed_with_edits(
            &chunk.as_bytes()[self.offset..],
            ComposerContext {
                agents: true,
                ..ComposerContext::default()
            },
            |_, _, _| {},
        );
        self.offset += consumed;
        let finished = !retired.editor.composer.has_pending_input();
        let binding = retired.binding.clone();
        if self.offset == chunk.as_bytes().len() || finished {
            self.chunk.take();
        }
        if finished {
            self.retired_editor.take();
        }
        if consumed > 0 {
            cx.waker().wake_by_ref();
        }
        Poll::Ready(Some(Ok((ComposerEvent::StaleInput, binding))))
    }
}
