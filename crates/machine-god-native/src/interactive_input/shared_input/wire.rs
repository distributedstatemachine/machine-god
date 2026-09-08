//! Fixed private credit/result protocol; never reads input before a valid credit.

use super::{Error, InputMode, NativeInteractiveInputChunk, Outcome, Shared};
use crate::interactive_input::NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES;
use rustix::fd::AsFd;
use rustix::fs::FileType;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

const HELLO: &[u8; 8] = b"MGI1HEL!";
const READY: &[u8; 8] = b"MGI1RDY!";
const STREAM_HELLO: &[u8; 8] = b"MGI1STR!";
const STREAM_READY: &[u8; 8] = b"MGI1SRD!";
const CREDIT: u8 = 1;
const DATA: u8 = 1;
const EOF: u8 = 2;
const FAILED: u8 = 3;

pub(super) fn handshake(channel: &UnixStream, shared: &Shared, mode: InputMode) -> Outcome {
    let (hello, expected) = match mode {
        InputMode::Interactive => (HELLO, READY),
        InputMode::Stream => (STREAM_HELLO, STREAM_READY),
    };
    write_parent(channel, hello, shared)?;
    let mut ready = [0; 8];
    read_parent(channel, &mut ready, shared)?;
    if &ready == expected {
        Ok(())
    } else {
        Err(Error::Read)
    }
}

pub(super) fn next_chunk(
    channel: &UnixStream,
    shared: &Shared,
) -> Result<Option<NativeInteractiveInputChunk>, Error> {
    write_parent(channel, &[CREDIT], shared)?;
    let mut header = [0; 5];
    read_parent(channel, &mut header, shared)?;
    let len = u32::from_be_bytes(header[1..].try_into().map_err(|_| Error::Read)?) as usize;
    match (header[0], len) {
        (EOF, 0) => Ok(None),
        (DATA, 1..=NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES) => {
            let mut chunk = NativeInteractiveInputChunk {
                bytes: [0; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES],
                len,
            };
            read_parent(channel, &mut chunk.bytes[..len], shared)?;
            Ok(Some(chunk))
        }
        _ => Err(Error::Read),
    }
}

fn read_parent(mut channel: &UnixStream, mut bytes: &mut [u8], shared: &Shared) -> Outcome {
    while !bytes.is_empty() {
        if shared.cancelled() {
            return Err(Error::Cancelled);
        }
        match channel.read(bytes) {
            Ok(0) => return Err(Error::Read),
            Ok(len) => bytes = &mut bytes[len..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => shared.pause(),
            Err(_) => return Err(Error::Read),
        }
    }
    Ok(())
}
fn write_parent(mut channel: &UnixStream, mut bytes: &[u8], shared: &Shared) -> Outcome {
    while !bytes.is_empty() {
        if shared.cancelled() {
            return Err(Error::Cancelled);
        }
        match channel.write(bytes) {
            Ok(0) => return Err(Error::Read),
            Ok(len) => bytes = &bytes[len..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => shared.pause(),
            Err(_) => return Err(Error::Read),
        }
    }
    Ok(())
}

/// Runs only through the exact private CLI helper dispatch. The invoking native
/// owner supplies stream stdin and a private duplex socket on stderr. No shell,
/// configuration, provider, or ordinary output initialization is performed.
///
/// # Errors
/// Rejects missing descriptor/protocol authority and native read/write errors.
/// Blocking pipe/file reads occur only inside this exact owned, killable process.
#[doc(hidden)]
pub fn run_interactive_input_helper() -> Outcome {
    let input = std::io::stdin();
    let channel = std::io::stderr();
    if FileType::from_raw_mode(
        rustix::fs::fstat(channel.as_fd())
            .map_err(|_| Error::InvalidDescriptor)?
            .st_mode,
    ) != FileType::Socket
    {
        return Err(Error::InvalidDescriptor);
    }
    run_endpoint(input, channel)
}

fn read_exact(channel: &impl AsFd, mut bytes: &mut [u8]) -> Outcome {
    while !bytes.is_empty() {
        match rustix::io::read(channel, &mut *bytes) {
            Ok(0) => return Err(Error::Read),
            Ok(len) => bytes = &mut bytes[len..],
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Err(Error::Read),
        }
    }
    Ok(())
}
fn write_exact(channel: &impl AsFd, mut bytes: &[u8]) -> Outcome {
    while !bytes.is_empty() {
        match rustix::io::write(channel, bytes) {
            Ok(0) => return Err(Error::Read),
            Ok(len) => bytes = &bytes[len..],
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Err(Error::Read),
        }
    }
    Ok(())
}
fn run_endpoint(input: impl AsFd, channel: impl AsFd) -> Outcome {
    let mut hello = [0; 8];
    read_exact(&channel, &mut hello)?;
    let (mode, ready) = match &hello {
        value if value == HELLO => (InputMode::Interactive, READY),
        value if value == STREAM_HELLO => (InputMode::Stream, STREAM_READY),
        _ => return Err(Error::Read),
    };
    let kind = FileType::from_raw_mode(
        rustix::fs::fstat(&input)
            .map_err(|_| Error::InvalidDescriptor)?
            .st_mode,
    );
    if kind != FileType::Fifo
        && !(matches!(mode, InputMode::Stream) && kind == FileType::RegularFile)
    {
        return Err(Error::InvalidDescriptor);
    }
    let flags = rustix::fs::fcntl_getfl(&input).map_err(|_| Error::InvalidDescriptor)?;
    if flags.contains(rustix::fs::OFlags::WRONLY) {
        return Err(Error::InvalidDescriptor);
    }
    write_exact(&channel, ready)?;
    loop {
        let mut credit = [0];
        read_exact(&channel, &mut credit)?;
        if credit != [CREDIT] {
            return Err(Error::Read);
        }
        let mut bytes = [0; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES];
        let result = loop {
            match rustix::io::read(&input, &mut bytes[..]) {
                Err(rustix::io::Errno::INTR) => {}
                result => break result,
            }
        };
        let (tag, len) = match result {
            Ok(0) => (EOF, 0),
            Ok(len) => (DATA, len),
            Err(_) => (FAILED, 0),
        };
        let mut header = [0; 5];
        header[0] = tag;
        header[1..].copy_from_slice(&u32::try_from(len).map_err(|_| Error::Read)?.to_be_bytes());
        write_exact(&channel, &header)?;
        write_exact(&channel, &bytes[..len])?;
        match tag {
            EOF => return Ok(()),
            FAILED => return Err(Error::Read),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
