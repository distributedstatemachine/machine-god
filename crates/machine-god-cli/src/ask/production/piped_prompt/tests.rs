use super::*;
use std::{io::Write, time::Duration};

#[test]
fn whole_prompt_trims_only_pinned_ascii_edges_and_preserves_internal_bytes() {
    for (input, expected) in [
        (" \t\r\nfirst\nsecond\r\n \t", "first\nsecond"),
        ("\u{a0}wide\u{a0}", "\u{a0}wide\u{a0}"),
        ("line\r\nline", "line\r\nline"),
        ("\u{4}", "\u{4}"),
    ] {
        assert_eq!(finish(input.as_bytes().to_vec()), Ok(expected.into()));
    }
}

#[test]
fn full_raw_bound_precedes_trim_and_invalid_inputs_never_return_prefixes() {
    assert_eq!(
        finish(vec![b'x'; MAX_ASK_PROMPT_BYTES]).unwrap().len(),
        MAX_ASK_PROMPT_BYTES
    );
    let mut over = vec![b' '; MAX_ASK_PROMPT_BYTES];
    over.push(b'x');
    assert_eq!(finish(over), Err(AskPromptInputError::TooLong));
    for bytes in [vec![], b" \t\r\n".to_vec()] {
        assert_eq!(finish(bytes), Err(AskPromptInputError::Missing));
    }
    for bytes in [
        b"valid\0suffix".to_vec(),
        b"valid\xf0\x9f".to_vec(),
        vec![0xff],
    ] {
        assert_eq!(finish(bytes), Err(AskPromptInputError::Invalid));
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn pipe() -> (NativeInteractiveInput, std::io::PipeWriter) {
    let (read, write) = std::io::pipe().unwrap();
    let descriptor: std::os::fd::OwnedFd = read.into();
    (
        NativeInteractiveInput::new(
            NativeInteractiveInputSource::AdoptNonblockingStatus(descriptor.into()),
            CancellationToken::new(),
        ),
        write,
    )
}

#[test]
fn collection_waits_for_eof_and_retains_multiline_split_utf8() {
    let (mut input, mut write) = pipe();
    let completion = input.completion();
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(receiver);
    let runtime = runtime();
    runtime.block_on(async {
        write.write_all(b" \nfirst\n\xf0\x9f").unwrap();
        let mut future = Box::pin(collect(&mut input, &mut signals));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut future)
                .await
                .is_err()
        );
        write.write_all(b"\xa6\x80 last \r\n").unwrap();
        drop(write);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), future)
                .await
                .unwrap(),
            Ok("first\n🦀 last".into())
        );
    });
    drop(input);
    completion.wait_on_worker().unwrap();
}

#[test]
fn oversized_stream_returns_without_eof_and_settles_its_reader() {
    let (input, mut write) = pipe();
    let completion = input.completion();
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(receiver);
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            write
                .write_all(&vec![b'x'; MAX_ASK_PROMPT_BYTES + 1])
                .unwrap();
            write // Keep writer open until reader has returned: EOF is not required.
        });
        assert_eq!(
            collect_and_settle(&runtime(), input, &mut signals),
            Err(AskCommandOutcome::PromptInput(AskPromptInputError::TooLong))
        );
        drop(writer.join().unwrap());
    });
    assert!(completion.is_complete());
}

#[test]
fn first_signal_precedes_ready_input_and_retains_cleaned_up_ownership() {
    for signal in [
        super::super::AskSignal::Interrupt,
        super::super::AskSignal::Terminate,
    ] {
        let (input, mut write) = pipe();
        write.write_all(b"valid prompt").unwrap();
        drop(write);
        let completion = input.completion();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.try_send(signal).unwrap();
        let mut signals = AskSignals::new(receiver);
        assert_eq!(
            collect_and_settle(&runtime(), input, &mut signals),
            Err(signal.outcome())
        );
        assert_eq!(signals.first_observed, Some(signal));
        assert!(completion.is_complete());
    }
}

#[test]
fn dropping_pending_collection_never_submits_and_retains_joinable_input() {
    let (mut input, mut write) = pipe();
    let completion = input.completion();
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(receiver);
    write.write_all(b"partial draft\n").unwrap();
    runtime().block_on(async {
        let mut pending = Box::pin(collect(&mut input, &mut signals));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut pending)
                .await
                .is_err()
        );
        drop(pending);
    });
    drop(input);
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
    drop(write);
}

#[test]
fn cancelled_pending_collection_drops_and_joins_without_waiting_for_eof() {
    let (input, _write) = pipe();
    let completion = input.completion();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(receiver);
    std::thread::scope(|scope| {
        let signal = scope.spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            sender
                .blocking_send(super::super::AskSignal::Terminate)
                .unwrap();
        });
        assert_eq!(
            collect_and_settle(&runtime(), input, &mut signals),
            Err(AskCommandOutcome::Terminated)
        );
        signal.join().unwrap();
    });
    assert!(completion.is_complete());
}
