use super::framing::{InteractiveInputFrameError, InteractiveInputFramer};
use machine_god_native::{
    NativeInteractiveInput, NativeInteractiveInputChunk, NativeInteractiveInputError,
    NativeInteractivePromptToken,
};
use std::task::{Context, Poll};

#[derive(Clone, Eq, PartialEq)]
pub(super) enum InputBinding {
    Command,
    AwaitingPrompt,
    Prompt {
        token: NativeInteractivePromptToken,
        question: usize,
    },
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
    chunk: Option<NativeInteractiveInputChunk>,
    offset: usize,
    chunk_binding: InputBinding,
    line_binding: Option<InputBinding>,
    ended: bool,
}

impl InputLines {
    pub fn new(input: NativeInteractiveInput) -> Self {
        Self {
            input,
            framer: InteractiveInputFramer::default(),
            chunk: None,
            offset: 0,
            chunk_binding: InputBinding::Command,
            line_binding: None,
            ended: false,
        }
    }

    pub fn poll_line(
        &mut self,
        cx: &mut Context<'_>,
        binding: InputBinding,
    ) -> Poll<Option<Result<(String, InputBinding), LineError>>> {
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
