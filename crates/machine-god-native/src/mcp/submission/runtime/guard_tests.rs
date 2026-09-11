use super::*;

fn guarded() -> (Fixture, Arc<[CancellationToken]>) {
    let mut fixture = Fixture::new();
    let guards: Arc<[CancellationToken]> = (0..MAX_MCP_RUNTIME_CANCELLATION_GUARDS)
        .map(|_| CancellationToken::new())
        .collect();
    fixture.runtime = fixture
        .runtime_owner
        .install_guarded(binding(), guards.clone())
        .unwrap();
    (fixture, guards)
}

#[test]
fn rejected_guards_preserve_the_active_generation() {
    let fixture = Fixture::new();
    let guards: Arc<[CancellationToken]> = (0..=MAX_MCP_RUNTIME_CANCELLATION_GUARDS)
        .map(|_| CancellationToken::new())
        .collect();
    assert!(matches!(
        fixture.runtime_owner.install_guarded(binding(), guards),
        Err(McpSubmissionError::Limit)
    ));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        fixture
            .runtime_owner
            .install_guarded(binding(), Arc::from([cancelled])),
        Err(McpSubmissionError::Cancelled)
    ));
    assert!(fixture.runtime.live().is_ok());
    fixture.ready("call");
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_ok());
}

#[test]
fn every_guard_revokes_preparation_and_an_admitted_claim() {
    for index in 0..MAX_MCP_RUNTIME_CANCELLATION_GUARDS {
        let (fixture, guards) = guarded();
        fixture.ready("call");
        let claim = fixture.claim("call", CancellationToken::new());
        guards[index].cancel();
        assert!(fixture.runtime.live().is_err());
        assert!(block_on(claim).is_err());
        assert!(
            block_on(fixture.prepare_request(
                &fixture.request("new"),
                &fixture.wire(),
                CancellationToken::new()
            ))
            .is_err()
        );
    }
}

#[test]
fn every_guard_wakes_response_observation_without_retaining_runtime() {
    for index in 0..MAX_MCP_RUNTIME_CANCELLATION_GUARDS {
        let (fixture, guards) = guarded();
        fixture.ready("call");
        let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
        let count = Arc::strong_count(&fixture.runtime);
        let mut waiting = submission.cancelled_owned();
        assert_eq!(Arc::strong_count(&fixture.runtime), count);
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        guards[index].cancel();
        assert!(counter.0.load(Ordering::SeqCst) > 0);
        assert!(poll(&mut waiting).is_ready());
        let state = Arc::new(Mutex::new(WriterState::default()));
        let mut writer = submission.into_writer(Writer(state.clone()));
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::Cancelled))
        );
        assert_eq!(state.lock().unwrap().calls, 0);
        assert!(!writer.was_attempted());
    }
}

#[test]
fn every_guard_wakes_an_independently_pending_writer() {
    for index in 0..MAX_MCP_RUNTIME_CANCELLATION_GUARDS {
        let (fixture, guards) = guarded();
        fixture.ready("call");
        let state = Arc::new(Mutex::new(WriterState {
            pending: true,
            ..WriterState::default()
        }));
        let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
            .unwrap()
            .into_writer(Writer(state.clone()));
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        assert!(
            Pin::new(&mut writer)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        guards[index].cancel();
        assert!(counter.0.load(Ordering::SeqCst) > 0);
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::Cancelled))
        );
        assert_eq!(state.lock().unwrap().calls, 1);
        assert_eq!(state.lock().unwrap().flushes, 0);
    }
}

#[test]
fn guard_checkpoint_prevents_first_write_suffix_and_flush() {
    for stage in 0..3 {
        let (fixture, guards) = guarded();
        fixture.ready("call");
        let state = Arc::new(Mutex::new(WriterState {
            chunk: usize::from(stage == 1),
            ..WriterState::default()
        }));
        let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
            .unwrap()
            .into_writer(Writer(state.clone()));
        if stage > 0 {
            assert!(poll(&mut writer).is_pending());
        }
        let calls = state.lock().unwrap().calls;
        let acknowledged = writer.acknowledged_bytes();
        guards[0].cancel();
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::Cancelled))
        );
        assert_eq!(state.lock().unwrap().calls, calls);
        assert_eq!(state.lock().unwrap().flushes, 0);
        assert_eq!(writer.acknowledged_bytes(), acknowledged);
        assert_eq!(
            poll(&mut writer),
            Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
        );
    }
}

#[test]
fn guard_revocation_during_flush_preserves_acknowledged_byte_receipt() {
    struct CancelOnFlush(CancellationToken);
    impl McpSubmissionWriter for CancelOnFlush {
        fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.0.cancel();
            Poll::Ready(Ok(()))
        }
    }
    let (fixture, guards) = guarded();
    fixture.ready("call");
    let mut writer = block_on(fixture.claim("call", CancellationToken::new()))
        .unwrap()
        .into_writer(CancelOnFlush(guards[0].clone()));
    assert!(poll(&mut writer).is_pending());
    assert_eq!(
        poll(&mut writer),
        Poll::Ready(Err(McpSubmissionError::Cancelled))
    );
    assert_eq!(writer.acknowledged_bytes(), fixture.wire().len() + 1);
    assert!(writer.was_attempted());
}
