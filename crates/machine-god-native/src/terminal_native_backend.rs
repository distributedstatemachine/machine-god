//! A single registry's owned choice of startup-gated native transport.
//! This adapter does not launch, discover or reconstruct native authority.

use crate::background_input::BackgroundInputReceipt;
use crate::terminal_pty::{TerminalPty, TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
use crate::terminal_session::TerminalSessionBackend;
use crate::terminal_startup::TerminalStartupBackend;
use crate::terminal_tmux_startup::NativeTerminalTmuxBackend;
use machine_god_core::{TerminalDimensions, TerminalSignal};
use std::task::Poll;

/// Owns exactly one already-created backend, including its startup gate and
/// cleanup authority. Boxing keeps registry entries independent of transport
/// sizes. The generic arms permit deterministic forwarding tests without native
/// process creation; production defaults retain the complete startup wrappers.
/// No additional Drop implementation may consume an inner cleanup receipt or
/// discard the authority needed for a retry or post-close write settlement.
pub(crate) enum TerminalNativeBackend<
    P = TerminalStartupBackend<TerminalPty>,
    T = TerminalStartupBackend<NativeTerminalTmuxBackend>,
> {
    Pty(Box<P>),
    Tmux(Box<T>),
}

impl<P: TerminalSessionBackend, T: TerminalSessionBackend> TerminalSessionBackend
    for TerminalNativeBackend<P, T>
{
    fn restore_startup_echo(&mut self) -> Result<(), ()> {
        match self {
            Self::Pty(backend) => backend.restore_startup_echo(),
            Self::Tmux(backend) => backend.restore_startup_echo(),
        }
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<TerminalPtyRead, ()> {
        match self {
            Self::Pty(backend) => backend.read(buffer),
            Self::Tmux(backend) => backend.read(buffer),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<BackgroundInputReceipt, ()> {
        match self {
            Self::Pty(backend) => backend.write(bytes),
            Self::Tmux(backend) => backend.write(bytes),
        }
    }

    fn write_with_paste(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> Result<BackgroundInputReceipt, ()> {
        match self {
            Self::Pty(backend) => backend.write_with_paste(bytes, paste),
            Self::Tmux(backend) => backend.write_with_paste(bytes, paste),
        }
    }

    fn input_write_limit(&self) -> usize {
        match self {
            Self::Pty(backend) => backend.input_write_limit(),
            Self::Tmux(backend) => backend.input_write_limit(),
        }
    }

    fn settle_write(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> Poll<Result<BackgroundInputReceipt, ()>> {
        match self {
            Self::Pty(backend) => backend.settle_write(bytes, paste),
            Self::Tmux(backend) => backend.settle_write(bytes, paste),
        }
    }

    fn status(&mut self) -> Result<TerminalPtyStatus, ()> {
        match self {
            Self::Pty(backend) => backend.status(),
            Self::Tmux(backend) => backend.status(),
        }
    }

    fn resize(&mut self, dimensions: &TerminalDimensions) -> Result<(), ()> {
        match self {
            Self::Pty(backend) => backend.resize(dimensions),
            Self::Tmux(backend) => backend.resize(dimensions),
        }
    }

    fn signal(&mut self, signal: TerminalSignal) -> Result<(), ()> {
        match self {
            Self::Pty(backend) => backend.signal(signal),
            Self::Tmux(backend) => backend.signal(signal),
        }
    }

    fn signal_may_discard_output(&self) -> bool {
        match self {
            Self::Pty(backend) => backend.signal_may_discard_output(),
            Self::Tmux(backend) => backend.signal_may_discard_output(),
        }
    }

    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> Result<TerminalPtyClose, ()> {
        match self {
            Self::Pty(backend) => backend.close(force, output),
            Self::Tmux(backend) => backend.close(force, output),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::BackgroundInputStatus;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, PartialEq)]
    enum Call {
        Echo,
        Read(usize),
        Write(Vec<u8>),
        Paste(Vec<u8>, bool),
        InputLimit,
        Settle(Vec<u8>, bool),
        Status,
        Resize(TerminalDimensions),
        Signal(TerminalSignal),
        Discard,
        Close(bool),
    }

    struct State {
        calls: Vec<Call>,
        fail: bool,
        read_closed: bool,
        receipt: BackgroundInputReceipt,
        settlements: VecDeque<Poll<Result<BackgroundInputReceipt, ()>>>,
        status: TerminalPtyStatus,
        incomplete: bool,
        drops: usize,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                fail: false,
                read_closed: false,
                receipt: BackgroundInputReceipt::new(2, false, BackgroundInputStatus::Written),
                settlements: VecDeque::new(),
                status: TerminalPtyStatus::Running,
                incomplete: false,
                drops: 0,
            }
        }
    }

    struct Fake<const TMUX: bool>(Arc<Mutex<State>>);

    impl<const TMUX: bool> TerminalSessionBackend for Fake<TMUX> {
        fn restore_startup_echo(&mut self) -> Result<(), ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Echo);
            if state.fail { Err(()) } else { Ok(()) }
        }
        fn read(&mut self, buffer: &mut [u8]) -> Result<TerminalPtyRead, ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Read(buffer.len()));
            if state.fail {
                return Err(());
            }
            let bytes = b"\0\n\xff";
            let count = bytes.len().min(buffer.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            Ok(TerminalPtyRead {
                bytes_read: count,
                closed: state.read_closed,
            })
        }
        fn write(&mut self, bytes: &[u8]) -> Result<BackgroundInputReceipt, ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Write(bytes.to_vec()));
            if state.fail {
                Err(())
            } else {
                Ok(state.receipt)
            }
        }
        fn write_with_paste(
            &mut self,
            bytes: &[u8],
            paste: bool,
        ) -> Result<BackgroundInputReceipt, ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Paste(bytes.to_vec(), paste));
            if state.fail {
                Err(())
            } else {
                Ok(state.receipt)
            }
        }
        fn input_write_limit(&self) -> usize {
            self.0.lock().unwrap().calls.push(Call::InputLimit);
            if TMUX { 65_536 } else { 8_192 }
        }
        fn settle_write(
            &mut self,
            bytes: &[u8],
            paste: bool,
        ) -> Poll<Result<BackgroundInputReceipt, ()>> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Settle(bytes.to_vec(), paste));
            state
                .settlements
                .pop_front()
                .expect("one scripted settlement per observation")
        }
        fn status(&mut self) -> Result<TerminalPtyStatus, ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Status);
            if state.fail {
                Err(())
            } else {
                Ok(state.status)
            }
        }
        fn resize(&mut self, dimensions: &TerminalDimensions) -> Result<(), ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Resize(dimensions.clone()));
            if state.fail { Err(()) } else { Ok(()) }
        }
        fn signal(&mut self, signal: TerminalSignal) -> Result<(), ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Signal(signal));
            if state.fail { Err(()) } else { Ok(()) }
        }
        fn signal_may_discard_output(&self) -> bool {
            self.0.lock().unwrap().calls.push(Call::Discard);
            TMUX
        }
        fn close(
            &mut self,
            force: bool,
            output: &mut dyn FnMut(&[u8]),
        ) -> Result<TerminalPtyClose, ()> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::Close(force));
            let result = if state.fail {
                Err(())
            } else {
                Ok(TerminalPtyClose {
                    status: state.status,
                    output_incomplete: state.incomplete,
                })
            };
            drop(state);
            output(b"\0\xff");
            output(b"tail\n");
            result
        }
    }

    impl<const TMUX: bool> Drop for Fake<TMUX> {
        fn drop(&mut self) {
            self.0.lock().unwrap().drops += 1;
        }
    }

    type Choice = TerminalNativeBackend<Fake<false>, Fake<true>>;

    fn fixture(tmux: bool) -> (Choice, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State::default()));
        let backend = if tmux {
            Choice::Tmux(Box::new(Fake(Arc::clone(&state))))
        } else {
            Choice::Pty(Box::new(Fake(Arc::clone(&state))))
        };
        assert!(state.lock().unwrap().calls.is_empty());
        (backend, state)
    }

    #[test]
    fn terminal_native_backend_forwards_all_methods_payloads_and_receipts_in_both_arms() {
        for tmux in [false, true] {
            let (mut backend, state) = fixture(tmux);
            backend.restore_startup_echo().unwrap();
            for closed in [false, true] {
                state.lock().unwrap().read_closed = closed;
                let mut output = [42; 5];
                assert_eq!(
                    backend.read(&mut output),
                    Ok(TerminalPtyRead {
                        bytes_read: 3,
                        closed
                    })
                );
                assert_eq!(output, [0, 10, 255, 42, 42]);
            }
            assert_eq!(
                backend.input_write_limit(),
                if tmux { 65_536 } else { 8_192 }
            );
            assert_eq!(backend.signal_may_discard_output(), tmux);
            let dimensions = TerminalDimensions::new(37, 111).unwrap();
            backend.resize(&dimensions).unwrap();
            for signal in [
                TerminalSignal::Hangup,
                TerminalSignal::Interrupt,
                TerminalSignal::Quit,
                TerminalSignal::Terminate,
                TerminalSignal::Kill,
            ] {
                backend.signal(signal).unwrap();
            }
            for status in [
                TerminalPtyStatus::Running,
                TerminalPtyStatus::Exited(37),
                TerminalPtyStatus::Signalled(9),
            ] {
                state.lock().unwrap().status = status;
                assert_eq!(backend.status(), Ok(status));
            }
            let mut expected = vec![
                Call::Echo,
                Call::Read(5),
                Call::Read(5),
                Call::InputLimit,
                Call::Discard,
                Call::Resize(dimensions),
            ];
            expected.extend(
                [
                    TerminalSignal::Hangup,
                    TerminalSignal::Interrupt,
                    TerminalSignal::Quit,
                    TerminalSignal::Terminate,
                    TerminalSignal::Kill,
                ]
                .into_iter()
                .map(Call::Signal),
            );
            expected.extend([Call::Status, Call::Status, Call::Status]);
            assert_eq!(state.lock().unwrap().calls, expected);
        }
    }

    #[test]
    fn terminal_native_backend_keeps_whole_paste_and_all_input_receipt_states() {
        for tmux in [false, true] {
            let (mut backend, state) = fixture(tmux);
            let whole = vec![0xff; 65_536];
            for receipt in [
                BackgroundInputReceipt::new(17, false, BackgroundInputStatus::Written),
                BackgroundInputReceipt::new(5, false, BackgroundInputStatus::Backpressure),
                BackgroundInputReceipt::new(9, true, BackgroundInputStatus::Closed),
                BackgroundInputReceipt::new(3, false, BackgroundInputStatus::Failed),
            ] {
                state.lock().unwrap().receipt = receipt;
                assert_eq!(backend.write(b"\0\n\xff"), Ok(receipt));
                assert_eq!(backend.write_with_paste(&whole, true), Ok(receipt));
                assert_eq!(backend.write_with_paste(b"\n", false), Ok(receipt));
                assert_eq!(backend.write(&[]), Ok(receipt));
            }
            let expected = (0..4)
                .flat_map(|_| {
                    [
                        Call::Write(b"\0\n\xff".to_vec()),
                        Call::Paste(whole.clone(), true),
                        Call::Paste(b"\n".to_vec(), false),
                        Call::Write(Vec::new()),
                    ]
                })
                .collect::<Vec<_>>();
            assert_eq!(state.lock().unwrap().calls, expected);
        }
    }

    #[test]
    fn terminal_native_backend_preserves_every_error_and_partial_close_output() {
        for tmux in [false, true] {
            let (mut backend, state) = fixture(tmux);
            state.lock().unwrap().fail = true;
            assert_eq!(backend.restore_startup_echo(), Err(()));
            let mut buffer = [42; 4];
            assert_eq!(backend.read(&mut buffer), Err(()));
            assert_eq!(buffer, [42; 4]);
            assert_eq!(backend.write(b"ordinary"), Err(()));
            assert_eq!(backend.write_with_paste(b"paste", true), Err(()));
            assert_eq!(backend.write_with_paste(b"text", false), Err(()));
            assert_eq!(backend.status(), Err(()));
            let dimensions = TerminalDimensions::new(24, 80).unwrap();
            assert_eq!(backend.resize(&dimensions), Err(()));
            assert_eq!(backend.signal(TerminalSignal::Interrupt), Err(()));
            let mut output = Vec::new();
            assert_eq!(
                backend.close(false, &mut |bytes| output.extend_from_slice(bytes)),
                Err(())
            );
            assert_eq!(output, b"\0\xfftail\n");
            assert_eq!(
                state.lock().unwrap().calls,
                vec![
                    Call::Echo,
                    Call::Read(4),
                    Call::Write(b"ordinary".to_vec()),
                    Call::Paste(b"paste".to_vec(), true),
                    Call::Paste(b"text".to_vec(), false),
                    Call::Status,
                    Call::Resize(dimensions),
                    Call::Signal(TerminalSignal::Interrupt),
                    Call::Close(false)
                ]
            );
            assert_eq!(state.lock().unwrap().drops, 0);
        }
    }

    #[test]
    fn terminal_native_backend_close_keeps_cleanup_receipts_retry_and_pending_settlement() {
        for tmux in [false, true] {
            let (mut backend, state) = fixture(tmux);
            let receipt = BackgroundInputReceipt::new(11, true, BackgroundInputStatus::Closed);
            state.lock().unwrap().settlements = VecDeque::from([
                Poll::Pending,
                Poll::Pending,
                Poll::Pending,
                Poll::Ready(Ok(receipt)),
                Poll::Ready(Err(())),
            ]);
            assert_eq!(backend.settle_write(b"submitted", true), Poll::Pending);
            state.lock().unwrap().fail = true;
            assert_eq!(backend.close(false, &mut |_| {}), Err(()));
            assert_eq!(backend.settle_write(b"submitted", true), Poll::Pending);
            state.lock().unwrap().fail = false;
            for (force, incomplete, status) in [
                (true, true, TerminalPtyStatus::Signalled(9)),
                (false, false, TerminalPtyStatus::Exited(23)),
            ] {
                state.lock().unwrap().status = status;
                state.lock().unwrap().incomplete = incomplete;
                let mut chunks = Vec::new();
                assert_eq!(
                    backend.close(force, &mut |bytes| chunks.push(bytes.to_vec())),
                    Ok(TerminalPtyClose {
                        status,
                        output_incomplete: incomplete
                    })
                );
                assert_eq!(chunks, vec![b"\0\xff".to_vec(), b"tail\n".to_vec()]);
                assert_eq!(state.lock().unwrap().drops, 0);
                if force {
                    assert_eq!(backend.settle_write(b"submitted", true), Poll::Pending);
                }
            }
            assert_eq!(
                backend.settle_write(b"submitted", true),
                Poll::Ready(Ok(receipt))
            );
            assert_eq!(backend.settle_write(b"other", false), Poll::Ready(Err(())));
            assert_eq!(
                state.lock().unwrap().calls,
                vec![
                    Call::Settle(b"submitted".to_vec(), true),
                    Call::Close(false),
                    Call::Settle(b"submitted".to_vec(), true),
                    Call::Close(true),
                    Call::Settle(b"submitted".to_vec(), true),
                    Call::Close(false),
                    Call::Settle(b"submitted".to_vec(), true),
                    Call::Settle(b"other".to_vec(), false)
                ]
            );
            drop(backend);
            assert_eq!(state.lock().unwrap().drops, 1);
        }
    }

    #[test]
    fn terminal_native_backend_construction_move_and_drop_add_no_operations() {
        for tmux in [false, true] {
            let (backend, state) = fixture(tmux);
            let mut owned = Some(backend);
            let backend = owned.take().unwrap();
            drop(owned);
            assert_eq!(state.lock().unwrap().drops, 0);
            drop(backend);
            let state = state.lock().unwrap();
            assert_eq!(state.drops, 1);
            assert!(state.calls.is_empty());
        }
    }

    #[test]
    fn terminal_native_backend_production_defaults_are_small_send_owned_session_backends() {
        fn assert_backend<B: TerminalSessionBackend + Send + 'static>() {}
        assert_backend::<TerminalNativeBackend>();
        assert!(std::mem::size_of::<TerminalNativeBackend>() <= 2 * std::mem::size_of::<usize>());
        // These typed constructors compile the actual startup-wrapped arms;
        // no fake replacement or runtime process launch satisfies this check.
        let _: fn(TerminalStartupBackend<TerminalPty>) -> TerminalNativeBackend =
            |backend| TerminalNativeBackend::Pty(Box::new(backend));
        let _: fn(TerminalStartupBackend<NativeTerminalTmuxBackend>) -> TerminalNativeBackend =
            |backend| TerminalNativeBackend::Tmux(Box::new(backend));
    }
}
