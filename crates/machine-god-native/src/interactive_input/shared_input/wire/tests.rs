use super::*;
use crate::NativeInteractiveInput;
use machine_god_core::CancellationToken;

#[test]
fn bounded_header_validation_rejects_oversize_unknown_and_partial_frames() {
    for bytes in [
        vec![DATA, 0, 0, 16, 1],
        vec![99, 0, 0, 0, 0],
        vec![DATA, 0, 0, 0, 0],
        vec![EOF, 0, 0, 0, 1],
        vec![FAILED, 0, 0, 0, 0],
        vec![DATA, 0],
        vec![DATA, 0, 0, 0, 2, b'x'],
    ] {
        let (channel, mut peer) = UnixStream::pair().unwrap();
        channel.set_nonblocking(true).unwrap();
        peer.write_all(&bytes).unwrap();
        peer.shutdown(std::net::Shutdown::Write).unwrap();
        let input = NativeInteractiveInput::default();
        assert_eq!(
            next_chunk(&channel, &input.shared).unwrap_err(),
            Error::Read
        );
    }
}

#[test]
fn parent_channel_cancellation_does_not_need_a_reply() {
    let (channel, _peer) = UnixStream::pair().unwrap();
    channel.set_nonblocking(true).unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    let input = NativeInteractiveInput::new(crate::NativeInteractiveInputSource::Disabled, stop);
    assert_eq!(handshake(&channel, &input.shared), Err(Error::Cancelled));
    assert_eq!(
        next_chunk(&channel, &input.shared).unwrap_err(),
        Error::Cancelled
    );
}
