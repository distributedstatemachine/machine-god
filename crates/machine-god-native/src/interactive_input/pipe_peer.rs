//! Independent pipe-peer observation on the existing demand-gated input worker.

use super::{
    NativeInteractiveInput, NativeInteractiveInputError as Error, NativeInteractiveInputSource,
    Shared, discard_waker, wake,
};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::FileType,
};
use std::{
    fs::File,
    task::{Context, Poll, Waker},
};

#[derive(Default)]
pub(super) struct Observation {
    requested: bool,
    pub(super) result: Option<Result<bool, Error>>,
    pub(super) waker: Option<Waker>,
}

/// The worker, not the poller or helper socket, retains this exact FIFO alias.
pub(super) struct PipePeer(Option<File>);
impl PipePeer {
    pub(super) fn capture(
        source: &NativeInteractiveInputSource,
        shared: &Shared,
    ) -> Result<Self, Error> {
        let file = match source {
            NativeInteractiveInputSource::Disabled => None,
            NativeInteractiveInputSource::PreserveNonblocking(file)
            | NativeInteractiveInputSource::AdoptNonblockingStatus(file)
            | NativeInteractiveInputSource::PreserveShared { input: file, .. }
            | NativeInteractiveInputSource::PreserveSharedStream { input: file, .. } => Some(file),
        };
        let pipe = match file {
            Some(file)
                if FileType::from_raw_mode(
                    rustix::fs::fstat(file)
                        .map_err(|_| Error::InvalidDescriptor)?
                        .st_mode,
                ) == FileType::Fifo =>
            {
                Some(file.try_clone().map_err(|_| Error::Unavailable)?)
            }
            _ => None,
        };
        if pipe.is_none() {
            publish(shared, Ok(false));
        }
        Ok(Self(pipe))
    }

    pub(super) fn observe(&self, shared: &Shared) {
        let Some(pipe) = &self.0 else { return };
        let requested = {
            let state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.pipe_peer.requested && state.pipe_peer.result.is_none()
        };
        if !requested {
            return;
        }
        // HUP is reported even with unread bytes and no requested read events.
        // This is not a read credit and never changes shared descriptor flags.
        let mut descriptors = [PollFd::new(pipe, PollFlags::empty())];
        match poll(
            &mut descriptors,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        ) {
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => publish(shared, Err(Error::Read)),
            Ok(_) => {
                let events = descriptors[0].revents();
                if events.intersects(PollFlags::ERR | PollFlags::NVAL) {
                    publish(shared, Err(Error::Read));
                } else if events.contains(PollFlags::HUP) {
                    publish(shared, Ok(true));
                }
            }
        }
    }
}

fn publish(shared: &Shared, result: Result<bool, Error>) {
    let waker = {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.pipe_peer.result.is_none() {
            state.pipe_peer.result = Some(result);
        }
        state.pipe_peer.waker.take()
    };
    wake(waker);
}

impl NativeInteractiveInput {
    /// Requests an independent observation of the original FIFO's closed writer.
    /// `true` is peer disconnect, not drained EOF: unread bytes and a published
    /// chunk remain untouched. `false` means the source is not a pipe. Regular
    /// files, terminals and null streams retain their normal demand-gated EOF.
    ///
    /// This call performs no descriptor work, starts no worker and grants no
    /// read credit. First `poll_chunk` admits the existing worker, which captures
    /// the exact pipe alias and checks requested hangup while credit-paused.
    /// A separate waker preserves simultaneous chunk/peer observers. Hosts may
    /// use disconnect as an explicit terminal cutoff, not successful delivery.
    ///
    /// # Errors
    /// Reports cancellation, input-worker failure or an unavailable pipe
    /// observation. The ordinary chunk lane retains its own terminal outcome.
    pub fn poll_pipe_peer_closed(&self, cx: &mut Context<'_>) -> Poll<Result<bool, Error>> {
        if self.shared.cancelled() {
            return Poll::Ready(Err(Error::Cancelled));
        }
        let incoming = cx.waker().clone();
        let (result, previous) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.pipe_peer.result {
                Some(result) => (Poll::Ready(result), None),
                None => {
                    state.pipe_peer.requested = true;
                    (Poll::Pending, state.pipe_peer.waker.replace(incoming))
                }
            }
        };
        discard_waker(previous);
        self.shared.changed.notify_all();
        result
    }
}
