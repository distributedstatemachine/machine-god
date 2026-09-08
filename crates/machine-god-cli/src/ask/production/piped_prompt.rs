//! Whole non-TTY stdin is one bounded prompt, never a line or interactive key.
//! The guardian and exact input completion outlive every read/drop/error path.

use super::{AskCommandOutcome, AskSignalControlSender, AskSignalController, AskSignals};
use crate::ask::{AskPromptInputError, MAX_ASK_PROMPT_BYTES};
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeInteractiveInput, NativeInteractiveInputHelper, NativeInteractiveInputSource,
};
use std::{future::poll_fn, io::IsTerminal, os::fd::AsFd, os::unix::fs::FileTypeExt, task::Poll};

#[cfg(test)]
mod tests;

pub(super) fn read(controller: &mut AskSignalController) -> Result<String, AskCommandOutcome> {
    let mut signals = controller
        .take_signals()
        .map_err(|()| AskCommandOutcome::OperationalFailure)?;
    let control = controller.control();
    let result = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("machine-god-stdin-prompt".into())
            .spawn_scoped(scope, || read_on_worker(&control, &mut signals))
            .map_err(|_| AskCommandOutcome::OperationalFailure)?;
        worker.join().unwrap_or_else(|payload| {
            std::mem::forget(payload);
            Err(AskCommandOutcome::OperationalFailure)
        })
    });
    // No input worker remains here. Setup may once again exit synchronously,
    // but its acknowledgement never erases a first signal already forwarded.
    let result = if result.is_ok() && control.enter_setup().is_err() {
        Err(AskCommandOutcome::OperationalFailure)
    } else {
        result
    };
    let first_signal = signals
        .first_observed
        .or_else(|| signals.receiver.try_recv().ok());
    controller.signals = Some(signals);
    first_signal.map_or(result, |signal| Err(signal.outcome()))
}

fn read_on_worker(
    control: &AskSignalControlSender,
    signals: &mut AskSignals,
) -> Result<String, AskCommandOutcome> {
    if std::io::stdin().is_terminal() {
        return Err(AskCommandOutcome::PromptInput(AskPromptInputError::Missing));
    }
    let source =
        capture().map_err(|()| AskCommandOutcome::PromptInput(AskPromptInputError::Read))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| AskCommandOutcome::OperationalFailure)?;
    control
        .activate_turn()
        .map_err(|()| AskCommandOutcome::OperationalFailure)?;
    let input = NativeInteractiveInput::new(source, CancellationToken::new());
    collect_and_settle(&runtime, input, signals)
}

fn capture() -> Result<NativeInteractiveInputSource, ()> {
    let path = std::env::current_exe().map_err(|_| ())?;
    let helper =
        NativeInteractiveInputHelper::new(&path, std::fs::File::open(&path).map_err(|_| ())?)
            .map_err(|_| ())?;
    let input = std::fs::File::from(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| ())?,
    );
    // A known null source is explicit authority, not permission to read arbitrary
    // devices. Native compares identities and returns EOF without a device read.
    let null_device = if input
        .metadata()
        .map_err(|_| ())?
        .file_type()
        .is_char_device()
    {
        std::fs::File::open("/dev/null").ok()
    } else {
        None
    };
    Ok(NativeInteractiveInputSource::PreserveSharedStream {
        input,
        helper,
        null_device,
    })
}

fn collect_and_settle(
    runtime: &tokio::runtime::Runtime,
    mut input: NativeInteractiveInput,
    signals: &mut AskSignals,
) -> Result<String, AskCommandOutcome> {
    let completion = input.completion();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(collect(&mut input, signals))
    }));
    drop(input);
    let settled = completion.wait_on_worker();
    let result = result.unwrap_or_else(|payload| {
        std::mem::forget(payload);
        Err(AskCommandOutcome::OperationalFailure)
    });
    if let Some(signal) = signals
        .first_observed
        .or_else(|| signals.receiver.try_recv().ok())
    {
        signals.first_observed.get_or_insert(signal);
        return Err(signal.outcome());
    }
    settled.map_err(|_| AskCommandOutcome::OperationalFailure)?;
    result
}

async fn collect(
    input: &mut NativeInteractiveInput,
    signals: &mut AskSignals,
) -> Result<String, AskCommandOutcome> {
    let mut bytes = Vec::with_capacity(MAX_ASK_PROMPT_BYTES);
    poll_fn(|cx| {
        if let Poll::Ready(signal) = signals.poll_signal(cx) {
            return Poll::Ready(Err(signal.outcome()));
        }
        match input.poll_chunk(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(AskCommandOutcome::PromptInput(
                AskPromptInputError::Read,
            ))),
            Poll::Ready(Ok(None)) => Poll::Ready(
                finish(std::mem::take(&mut bytes)).map_err(AskCommandOutcome::PromptInput),
            ),
            Poll::Ready(Ok(Some(chunk))) => {
                if chunk.as_bytes().len() > MAX_ASK_PROMPT_BYTES - bytes.len() {
                    return Poll::Ready(Err(AskCommandOutcome::PromptInput(
                        AskPromptInputError::TooLong,
                    )));
                }
                bytes.extend_from_slice(chunk.as_bytes());
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    })
    .await
}

fn finish(bytes: Vec<u8>) -> Result<String, AskPromptInputError> {
    if bytes.len() > MAX_ASK_PROMPT_BYTES {
        return Err(AskPromptInputError::TooLong);
    }
    if bytes.contains(&0) {
        return Err(AskPromptInputError::Invalid);
    }
    let mut text = String::from_utf8(bytes).map_err(|_| AskPromptInputError::Invalid)?;
    let trimmed = text.trim_matches([' ', '\t', '\r', '\n']);
    if trimmed.is_empty() {
        return Err(AskPromptInputError::Missing);
    }
    let start = text.len() - text.trim_start_matches([' ', '\t', '\r', '\n']).len();
    let end = start + trimmed.len();
    text.truncate(end);
    drop(text.drain(..start));
    Ok(text)
}
