use super::*;
use crate::mcp::submission::tests::Fixture;
use crate::mcp::submission::{McpSubmissionWriter, WriteGuard};
use futures_executor::block_on;
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::atomic::Ordering,
    task::{Context, Poll},
};

struct Writer {
    fail_write: bool,
    fail_flush: bool,
}
impl McpSubmissionWriter for Writer {
    fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        if self.fail_write {
            Poll::Ready(Err(io::Error::other("ambiguous")))
        } else {
            Poll::Ready(Ok(bytes.len()))
        }
    }
    fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.fail_flush {
            Poll::Ready(Err(io::Error::other("ambiguous")))
        } else {
            Poll::Ready(Ok(()))
        }
    }
}
fn poll<T: Future + Unpin>(future: &mut T) -> Poll<T::Output> {
    Pin::new(future).poll(&mut Context::from_waker(std::task::Waker::noop()))
}

#[test]
fn full_write_without_final_successful_flush_never_arms_continuation_marker() {
    for fail_flush in [false, true] {
        let fixture = Fixture::new();
        fixture.ready("call");
        let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
        let written = submission.write_completion();
        let mut writer = submission.into_writer(Writer {
            fail_write: false,
            fail_flush,
        });
        assert!(!written.load(Ordering::Acquire));
        assert!(poll(&mut writer).is_pending());
        assert!(!written.load(Ordering::Acquire));
        assert!(writer.acknowledged_bytes() > 0);
        assert!(poll(&mut writer).is_ready());
        assert_eq!(written.load(Ordering::Acquire), !fail_flush);
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
        );
    }
}

#[test]
fn partial_intermediate_flush_failed_write_and_final_revocation_do_not_arm() {
    for stage in 0..4 {
        let fixture = Fixture::new();
        fixture.ready("call");
        let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
        let written = submission.write_completion();
        let mut guard = WriteGuard::new(submission);
        guard.begin_delegate().unwrap();
        match stage {
            0 => {
                assert_eq!(
                    guard.flush_result(&Poll::Ready(Ok(()))),
                    Poll::Ready(Ok(()))
                );
            }
            1 => {
                assert_eq!(
                    guard.write_result(&Poll::Ready(Ok(1)), 1),
                    Poll::Ready(Ok(1))
                );
                assert_eq!(
                    guard.flush_result(&Poll::Ready(Ok(()))),
                    Poll::Ready(Ok(()))
                );
            }
            2 => {
                assert!(matches!(
                    guard.write_result(
                        &Poll::Ready(Err(io::Error::other("partial effect unknown"))),
                        1
                    ),
                    Poll::Ready(Err(_))
                ));
            }
            _ => {
                let len = guard.submission.ready.data.wire.len();
                assert_eq!(
                    guard.write_result(&Poll::Ready(Ok(len)), len),
                    Poll::Ready(Ok(len))
                );
                fixture.revoke();
                assert!(matches!(
                    guard.flush_result(&Poll::Ready(Ok(()))),
                    Poll::Ready(Err(_))
                ));
            }
        }
        assert!(!written.load(Ordering::Acquire));
        drop(guard);
        assert!(!written.load(Ordering::Acquire));
    }
}

#[test]
fn raw_original_submission_never_exposes_typed_continuation_custody() {
    let fixture = Fixture::new();
    fixture.ready("call");
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert!(matches!(
        submission.continuation_custody(),
        Err(McpSubmissionError::Denied)
    ));
}
