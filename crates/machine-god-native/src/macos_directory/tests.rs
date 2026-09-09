use super::*;
use std::{
    fs::File,
    os::fd::AsFd,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-macos-directory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn record(inode: u64, name: &[u8]) -> Vec<u8> {
    let length = (NAME_OFFSET + name.len() + 1).next_multiple_of(8);
    let mut bytes = vec![0; length];
    bytes[..8].copy_from_slice(&inode.to_ne_bytes());
    bytes[16..18].copy_from_slice(&u16::try_from(length).unwrap().to_ne_bytes());
    bytes[18..20].copy_from_slice(&u16::try_from(name.len()).unwrap().to_ne_bytes());
    bytes[NAME_OFFSET..NAME_OFFSET + name.len()].copy_from_slice(name);
    bytes
}

#[test]
fn real_directory_refills_preserve_names_and_descriptor_ownership() {
    let fixture = Fixture::new();
    let mut expected = std::collections::BTreeSet::new();
    for index in 0..600 {
        let name = format!("name-{index:04}-{}", "x".repeat(30));
        std::fs::write(fixture.0.join(&name), []).unwrap();
        expected.insert(name.into_bytes());
    }
    let directory = File::open(&fixture.0).unwrap();
    let mut reader = MacosDirectoryReader::new(directory.as_fd());
    assert!(reader.is_buffer_empty());
    let mut calls = 0;
    let mut observed = std::collections::BTreeSet::new();
    loop {
        let charge = reader.is_buffer_empty();
        let before = calls;
        let next = reader.next_with(|fd, buffer| {
            calls += 1;
            read_directory_chunk(fd, buffer)
        });
        assert_eq!(calls - before, usize::from(charge));
        match next {
            Some(Ok(MacosDirectoryEntry::Name(name))) if name != b"." && name != b".." => {
                assert!(observed.insert(name));
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => panic!("directory read failed: {error}"),
            None => break,
        }
    }
    assert!(calls > 1);
    assert_eq!(observed, expected);
    assert!(
        reader
            .next_with(|_, _| panic!("EOF cannot refill"))
            .is_none()
    );
    assert!(directory.metadata().unwrap().is_dir());
}

#[test]
fn construction_is_inert_and_real_failure_is_terminal() {
    let file = File::open("/dev/null").unwrap();
    {
        let reader = MacosDirectoryReader::new(file.as_fd());
        assert!(reader.is_buffer_empty());
    }
    let mut reader = MacosDirectoryReader::new(file.as_fd());
    assert!(reader.next_name().unwrap().is_err());
    assert!(
        reader
            .next_with(|_, _| panic!("terminal failure cannot retry"))
            .is_none()
    );
    assert!(file.metadata().is_ok());
}

#[test]
fn skipped_and_dot_records_never_hide_another_refill() {
    let file = File::open("/dev/null").unwrap();
    let mut reader = MacosDirectoryReader::new(file.as_fd());
    let bytes = [record(0, b""), record(1, b"."), record(2, b"..")].concat();
    let first = reader
        .next_with(|_, buffer| {
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        })
        .unwrap()
        .unwrap();
    assert!(matches!(first, MacosDirectoryEntry::Skipped));
    assert!(!reader.is_buffer_empty());
    for expected in [b".".as_slice(), b".."] {
        let entry = reader
            .next_with(|_, _| panic!("buffered record cannot refill"))
            .unwrap()
            .unwrap();
        assert!(matches!(entry, MacosDirectoryEntry::Name(name) if name == expected));
    }
    assert!(reader.is_buffer_empty());
    for _ in 0..3 {
        let bytes = record(0, b"gone");
        assert!(matches!(
            reader.next_with(|_, buffer| {
                buffer[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }),
            Some(Ok(MacosDirectoryEntry::Skipped))
        ));
        assert!(reader.is_buffer_empty());
    }
}

#[test]
fn interrupted_partial_prefix_is_discarded_and_retry_is_separate() {
    let file = File::open("/dev/null").unwrap();
    let mut reader = MacosDirectoryReader::new(file.as_fd());
    for _ in 0..3 {
        let error = reader
            .next_with(|_, buffer| {
                buffer[..32].fill(0xff);
                Err(io::Error::from_raw_os_error(libc::EINTR))
            })
            .unwrap()
            .unwrap_err();
        assert_eq!(error, Errno::INTR);
        assert!(reader.is_buffer_empty());
    }
    let bytes = record(2, b"after");
    let next = reader
        .next_with(|_, buffer| {
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        })
        .unwrap()
        .unwrap();
    assert!(matches!(next, MacosDirectoryEntry::Name(name) if name == b"after"));
    assert!(reader.next_with(|_, _| Ok(0)).is_none());
}

#[test]
fn malformed_record_boundaries_and_names_fail_closed() {
    let valid = record(1, b"name");
    for length in 0..valid.len() {
        assert!(decode_record(&valid[..length]).is_err());
    }
    for length in [0_u16, 20, 21, 23, 8192, u16::MAX] {
        let mut bytes = valid.clone();
        bytes[16..18].copy_from_slice(&length.to_ne_bytes());
        assert!(decode_record(&bytes).is_err());
    }
    for length in [0_u16, 1024, u16::MAX] {
        let mut bytes = valid.clone();
        bytes[18..20].copy_from_slice(&length.to_ne_bytes());
        assert!(decode_record(&bytes).is_err());
    }
    for name in [b"a\0b".as_slice(), b"a/b"] {
        assert!(decode_record(&record(1, name)).is_err());
    }
    let mut missing_nul = valid;
    missing_nul[NAME_OFFSET + 4] = b'x';
    assert!(decode_record(&missing_nul).is_err());
    let max = vec![b'x'; MAX_NAME_BYTES];
    assert!(
        matches!(decode_record(&record(1, &max)), Ok((_, MacosDirectoryEntry::Name(name))) if name == max)
    );
    // The test volume rejects this spelling with EILSEQ. The byte parser still
    // must preserve it when supplied by a filesystem, without assuming UTF-8.
    let raw = vec![b'n', 0xff];
    assert!(
        matches!(decode_record(&record(1, &raw)), Ok((_, MacosDirectoryEntry::Name(name))) if name == raw)
    );
}

#[test]
fn oversized_refill_and_trailing_truncation_poison_without_refilling() {
    let file = File::open("/dev/null").unwrap();
    for oversized in [false, true] {
        let mut reader = MacosDirectoryReader::new(file.as_fd());
        let result = reader.next_with(|_, buffer| {
            buffer[0] = 0;
            Ok(if oversized { 8193 } else { 1 })
        });
        assert!(matches!(result, Some(Err(Errno::IO))));
        assert!(
            reader
                .next_with(|_, _| panic!("malformed stream must stay terminal"))
                .is_none()
        );
    }
    let mut reader = MacosDirectoryReader::new(file.as_fd());
    let mut bytes = record(1, b"ok");
    bytes.push(0);
    assert!(
        reader
            .next_with(|_, buffer| {
                buffer[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            })
            .unwrap()
            .is_ok()
    );
    assert!(!reader.is_buffer_empty());
    assert!(matches!(
        reader.next_with(|_, _| panic!("truncated buffered tail cannot refill")),
        Some(Err(Errno::IO))
    ));
}
