//! Explicit recording selection and the CLI worker's independent cleanup fence.

use machine_god_core::CancellationToken;
use machine_god_native::{
    FileSessionStore, NativeOwnedWorkerScope, TerminalTapeRecorder,
    TerminalTapeRecordingCompletion, TerminalTapeRecordingDestination,
    TerminalTapeRecordingOptions, TerminalTapeRecordingRequest, TokioWebSearchRuntime,
};
use std::{
    cell::RefCell,
    ffi::OsString,
    fmt::Write as _,
    future::poll_fn,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_PATH_BYTES: usize = 4096;
pub(super) const UNAVAILABLE: &[u8] = b"[recording unavailable; continuing without a tape]\n";

enum Destination {
    Automatic,
    Explicit(PathBuf),
    Invalid,
}

pub(super) struct Selection {
    destination: Destination,
    required: bool,
    record_stdin: bool,
}

impl Selection {
    /// The interactive host calls this once, never from a native recorder.
    pub(super) fn capture(required: bool, workspace: &Path) -> Option<Self> {
        Self::from_values(
            required,
            workspace,
            std::env::var_os("FX_RECORD"),
            std::env::var_os("FX_RECORD_INPUT"),
        )
    }

    pub(super) fn from_values(
        required: bool,
        workspace: &Path,
        path: Option<OsString>,
        input: Option<OsString>,
    ) -> Option<Self> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let path = path.map(OsString::into_vec);
        let path = path.as_deref().map_or(&[][..], trim);
        let destination = if path.is_empty() {
            if !required {
                return None;
            }
            Destination::Automatic
        } else if path.len() > MAX_PATH_BYTES || path.contains(&0) {
            Destination::Invalid
        } else {
            let selected = workspace.join(std::ffi::OsStr::from_bytes(path));
            if !selected.is_absolute() || selected.as_os_str().as_bytes().len() > MAX_PATH_BYTES {
                Destination::Invalid
            } else {
                Destination::Explicit(selected)
            }
        };
        let record_stdin = input.is_some_and(|value| {
            let value = value.into_vec();
            let value = trim(&value);
            value == b"1"
                || value.eq_ignore_ascii_case(b"true")
                || value.eq_ignore_ascii_case(b"on")
        });
        Some(Self {
            destination,
            required,
            record_stdin,
        })
    }
}

fn trim(value: &[u8]) -> &[u8] {
    let padding = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
    let start = value
        .iter()
        .position(|byte| !padding(byte))
        .unwrap_or(value.len());
    let value = &value[start..];
    let end = value
        .iter()
        .rposition(|byte| !padding(byte))
        .map_or(0, |index| index + 1);
    &value[..end]
}

struct Worker {
    scope: NativeOwnedWorkerScope,
    recording: Option<TerminalTapeRecordingCompletion>,
}

/// Only ownership/observation handles live here; native owns the tape state.
#[derive(Default)]
pub(super) struct Settlement {
    worker: RefCell<Option<Worker>>,
}

pub(super) struct Started {
    pub(super) recorder: Option<TerminalTapeRecorder>,
    pub(super) record_stdin: bool,
    pub(super) notice: Option<Vec<u8>>,
}

impl Started {
    fn inactive(notice: Option<Vec<u8>>) -> Self {
        Self {
            recorder: None,
            record_stdin: false,
            notice,
        }
    }
}

impl Settlement {
    pub(super) fn is_live(&self) -> bool {
        self.worker.borrow().is_some()
    }
    /// Called on the dedicated CLI host thread, outside asynchronous polling.
    /// Failed optional setup is fully joined before the caller may admit a session.
    pub(super) fn start<G: super::SignalSource>(
        &self,
        runtime: &TokioWebSearchRuntime,
        selection: Option<Selection>,
        store: Arc<FileSessionStore>,
        state_path: PathBuf,
        mut options: TerminalTapeRecordingOptions,
        signals: &mut G,
    ) -> Result<Started, ()> {
        let Some(selection) = selection else {
            return Ok(Started::inactive(None));
        };
        if self.worker.borrow().is_some() {
            return Err(());
        }
        options.record_stdin = selection.record_stdin;
        let destination = match selection.destination {
            Destination::Automatic => {
                TerminalTapeRecordingDestination::Automatic { store, state_path }
            }
            Destination::Explicit(path) => TerminalTapeRecordingDestination::Explicit(path),
            Destination::Invalid => return optional_failure(selection.required),
        };
        let scope = NativeOwnedWorkerScope::new();
        self.worker.replace(Some(Worker {
            scope: scope.clone(),
            recording: None,
        }));
        let cancellation = CancellationToken::new();
        let mut interrupted = false;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut pending = TerminalTapeRecorder::start(
                TerminalTapeRecordingRequest {
                    destination,
                    options,
                },
                scope,
                cancellation.clone(),
            );
            runtime.block_on(poll_fn(|cx| {
                if !interrupted && signals.poll_signal(cx).is_ready() {
                    interrupted = true;
                    cancellation.cancel();
                }
                pending.as_mut().poll(cx)
            }))
        }))
        .map_err(std::mem::forget)
        .and_then(|result| result.map_err(|_| ()));
        if interrupted {
            drop(result);
            self.join(false)?;
            return Err(());
        }
        let Ok(recorder) = result else {
            self.join(false)?;
            return optional_failure(selection.required);
        };
        self.worker.borrow_mut().as_mut().ok_or(())?.recording = Some(recorder.completion());
        let Ok(notice) = active_notice(recorder.path(), selection.record_stdin) else {
            drop(recorder);
            self.join(false)?;
            return optional_failure(selection.required);
        };
        Ok(Started {
            recorder: Some(recorder),
            record_stdin: selection.record_stdin,
            notice: Some(notice),
        })
    }

    /// All output/tape owners must already be dropped. A last frame/file-close
    /// acknowledgement cannot substitute for this collector's actual join.
    pub(super) fn finish(&self) -> Result<(), ()> {
        self.join(true)
    }

    fn join(&self, require_complete: bool) -> Result<(), ()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.scope.close();
        worker.scope.completion().wait_on_worker().map_err(|_| ())?;
        if require_complete
            && worker
                .recording
                .is_some_and(|recording| !recording.status().complete)
        {
            return Err(());
        }
        Ok(())
    }
}

fn optional_failure(required: bool) -> Result<Started, ()> {
    if required {
        Err(())
    } else {
        Ok(Started::inactive(Some(UNAVAILABLE.to_vec())))
    }
}

fn active_notice(path: &Path, stdin: bool) -> Result<Vec<u8>, ()> {
    let mut output = crate::bounded_output::BoundedOutput::with_capacity(32 * 1024, 256);
    writeln!(
        output,
        "[recording \"{}\"; stdin {}]",
        path.as_os_str().as_encoded_bytes().escape_ascii(),
        if stdin { "included" } else { "excluded" }
    )
    .map_err(|_| ())?;
    Ok(output.finish().into_bytes())
}

#[cfg(test)]
mod tests;
