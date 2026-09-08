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
    assert_eq!(
        handshake(&channel, &input.shared, InputMode::Interactive),
        Err(Error::Cancelled)
    );
    assert_eq!(
        next_chunk(&channel, &input.shared).unwrap_err(),
        Error::Cancelled
    );
}

#[test]
fn helper_negotiates_regular_files_only_in_explicit_stream_mode_before_credit() {
    use std::io::Seek as _;
    for (hello, expected, ready) in [
        (HELLO, Error::InvalidDescriptor, None),
        (STREAM_HELLO, Error::Read, Some(STREAM_READY)),
        (b"UNKNOWN!", Error::Read, None),
    ] {
        let mut file = std::fs::File::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        )
        .unwrap();
        let (channel, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(hello).unwrap();
        if ready.is_some() {
            // Invalid credit: no source read is authorized.
            peer.write_all(&[0]).unwrap();
        }
        assert_eq!(run_endpoint(&file, &channel), Err(expected));
        assert_eq!(file.stream_position().unwrap(), 0);
        drop(channel);
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert_eq!(
            received.as_slice(),
            ready.map_or(&[][..], |value| &value[..])
        );
    }
}

#[test]
fn stream_handshake_rejects_old_or_mismatched_ready() {
    for ready in [READY, b"BADREADY"] {
        let (channel, mut peer) = UnixStream::pair().unwrap();
        channel.set_nonblocking(true).unwrap();
        peer.write_all(ready).unwrap();
        let input = NativeInteractiveInput::default();
        assert_eq!(
            handshake(&channel, &input.shared, InputMode::Stream),
            Err(Error::Read)
        );
    }
}
