//! Deferred resolver capture has host-worker custody, not invocation custody.

use super::*;
use crate::NativeOwnedWorkerScope;
use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::task::{Context, Wake, Waker};
use std::thread::ThreadId;

const OBSERVATION_BOUND: Duration = Duration::from_secs(5);

fn runtime(paused: bool) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(paused)
        .build()
        .unwrap()
}

fn hostname() -> WebFetchRequest {
    canonical_request("https://example.com/").unwrap()
}

fn unavailable() -> WebFetchTransportError {
    transport_error(WebFetchTransportErrorKind::Unavailable)
}

fn transport(
    workers: &NativeOwnedWorkerScope,
    capture: impl FnOnce() -> Result<SocketAddr, WebFetchTransportError> + Send + 'static,
) -> NativeWebFetchTransport {
    NativeWebFetchTransport::with_owned_workers_and_capture(
        Duration::from_secs(1),
        root_tls_config().unwrap(),
        Ok([7; 32]),
        workers,
        capture,
    )
}

// Dropping a fixture releases its worker even if an assertion unwinds. These
// receives establish ordering; their timeout is only a failed-test safeguard.
struct Gate {
    entered: Receiver<ThreadId>,
    release: Option<Sender<()>>,
}

impl Gate {
    fn entered(&self) {
        assert_ne!(
            self.entered.recv_timeout(OBSERVATION_BOUND).unwrap(),
            std::thread::current().id(),
            "capture and thread cleanup run on the owned worker"
        );
    }

    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

fn gate() -> (Gate, impl FnOnce() + Send + 'static) {
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    (
        Gate {
            entered: observed,
            release: Some(release),
        },
        move || {
            entered.send(std::thread::current().id()).unwrap();
            released.recv_timeout(OBSERVATION_BOUND).unwrap();
        },
    )
}

struct WakeNotice(Sender<()>);

impl Wake for WakeNotice {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

fn wake_notice() -> (Waker, Receiver<()>) {
    let (notice, observed) = mpsc::channel();
    (Waker::from(Arc::new(WakeNotice(notice))), observed)
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(waker))
}

#[test]
fn deferred_capture_is_inert_through_preparation_and_non_hostname_admission() {
    let workers = NativeOwnedWorkerScope::new();
    let captures = Arc::new(AtomicU32::new(0));
    let native = Arc::new(transport(&workers, {
        let captures = captures.clone();
        move || {
            captures.fetch_add(1, Ordering::AcqRel);
            Err(unavailable())
        }
    }));
    let tool = WebFetchTool::with_bounded_transport(native.clone(), WebFetchLimits::default());
    tool.prepare(ToolCall {
        id: machine_god_core::ToolCallId::new("deferred-capture").unwrap(),
        name: ToolName::new(WEB_FETCH_TOOL_NAME).unwrap(),
        arguments: json!({"url":"https://example.com/"}),
    })
    .unwrap();
    assert_eq!(captures.load(Ordering::Acquire), 0);
    drop(native.fetch(hostname(), CancellationToken::new()));
    runtime(false).block_on(async {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            native
                .resolve_public_addresses(&hostname(), &cancellation)
                .await
                .unwrap_err()
                .kind(),
            WebFetchTransportErrorKind::Cancelled
        );
        let mut expired = hostname();
        expired.install_execution_deadline(Instant::now());
        assert_eq!(
            native
                .resolve_public_addresses(&expired, &CancellationToken::new())
                .await
                .unwrap_err()
                .kind(),
            WebFetchTransportErrorKind::Timeout
        );
        let addresses = native
            .resolve_public_addresses(
                &canonical_request("https://93.184.216.34/").unwrap(),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(addresses.len(), 1);
        assert_eq!(
            addresses[0].ip(),
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))
        );
    });
    assert_eq!(captures.load(Ordering::Acquire), 0);
    workers.close();
    assert!(workers.completion().is_complete());
    workers.completion().wait_on_worker().unwrap();
}

struct ThreadCleanup(Option<Box<dyn FnOnce() + Send>>);

impl Drop for ThreadCleanup {
    fn drop(&mut self) {
        self.0.take().unwrap()();
    }
}

thread_local! {
    static CLEANUP: RefCell<Option<ThreadCleanup>> = const { RefCell::new(None) };
}

#[test]
fn deferred_success_shares_capture_wakes_all_waiters_and_owns_thread_cleanup() {
    let workers = NativeOwnedWorkerScope::new();
    let completion = workers.completion();
    let captures = Arc::new(AtomicU32::new(0));
    let (mut capture_gate, hold_capture) = gate();
    let (mut cleanup_gate, hold_cleanup) = gate();
    let nameserver: SocketAddr = "127.0.0.1:5353".parse().unwrap();
    let native = transport(&workers, {
        let captures = captures.clone();
        move || {
            captures.fetch_add(1, Ordering::AcqRel);
            CLEANUP.with(|slot| {
                *slot.borrow_mut() = Some(ThreadCleanup(Some(Box::new(hold_cleanup))));
            });
            hold_capture();
            Ok(nameserver)
        }
    });
    let request = hostname();
    let cancellation = CancellationToken::new();
    let mut first = Box::pin(native.resolve_public_addresses(&request, &cancellation));
    let mut second = Box::pin(native.resolve_public_addresses(&request, &cancellation));
    let (first_waker, first_wake) = wake_notice();
    let (second_waker, second_wake) = wake_notice();
    assert!(poll_once(first.as_mut(), &first_waker).is_pending());
    capture_gate.entered();
    assert!(poll_once(second.as_mut(), &second_waker).is_pending());
    assert_eq!(captures.load(Ordering::Acquire), 1);
    // Both requests still receive the successful capture, but their boundary
    // must stop before DNS. No test-owned nameserver or public network is used.
    cancellation.cancel();
    workers.close();
    assert!(!completion.is_complete());
    capture_gate.release();
    first_wake.recv_timeout(OBSERVATION_BOUND).unwrap();
    second_wake.recv_timeout(OBSERVATION_BOUND).unwrap();
    for result in [
        futures_executor::block_on(first),
        futures_executor::block_on(second),
    ] {
        assert_eq!(
            result.unwrap_err().kind(),
            WebFetchTransportErrorKind::Cancelled
        );
    }
    cleanup_gate.entered();
    assert!(
        !completion.is_complete(),
        "a captured result is not a worker join"
    );
    // The shared successful result remains available after admission is closed;
    // another worker attempt would fail and cannot satisfy this assertion.
    assert_eq!(
        futures_executor::block_on(native.nameserver.snapshot()),
        Ok(nameserver)
    );
    assert_eq!(captures.load(Ordering::Acquire), 1);
    drop(native);
    assert!(
        !completion.is_complete(),
        "transport drop cannot discharge cleanup"
    );
    cleanup_gate.release();
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}

#[test]
fn abandoned_waiter_keeps_capture_and_caches_failure_after_scope_closure() {
    let workers = NativeOwnedWorkerScope::new();
    let captures = Arc::new(AtomicU32::new(0));
    let (mut capture_gate, hold_capture) = gate();
    let native = transport(&workers, {
        let captures = captures.clone();
        move || {
            captures.fetch_add(1, Ordering::AcqRel);
            hold_capture();
            Err(unavailable())
        }
    });
    let mut abandoned = Box::pin(native.nameserver.snapshot());
    assert!(poll_once(abandoned.as_mut(), Waker::noop()).is_pending());
    capture_gate.entered();
    drop(abandoned);
    workers.close();
    assert!(!workers.completion().is_complete());
    let mut replacement = Box::pin(native.nameserver.snapshot());
    assert!(poll_once(replacement.as_mut(), Waker::noop()).is_pending());
    assert_eq!(captures.load(Ordering::Acquire), 1);
    capture_gate.release();
    assert_eq!(futures_executor::block_on(replacement), Err(unavailable()));
    assert_eq!(
        futures_executor::block_on(native.nameserver.snapshot()),
        Err(unavailable())
    );
    assert_eq!(captures.load(Ordering::Acquire), 1);
    workers.completion().wait_on_worker().unwrap();
    assert!(workers.completion().is_complete());
}

#[test]
fn bounded_cancellation_and_timeout_end_only_the_waiter_not_shared_capture() {
    for timeout in [false, true] {
        let workers = NativeOwnedWorkerScope::new();
        let captures = Arc::new(AtomicU32::new(0));
        let (mut capture_gate, hold_capture) = gate();
        let native = Arc::new(transport(&workers, {
            let captures = captures.clone();
            move || {
                captures.fetch_add(1, Ordering::AcqRel);
                hold_capture();
                Err(unavailable())
            }
        }));
        let bounded = BoundedWebFetchTransport::new(
            native.clone(),
            WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1).unwrap(),
        );
        runtime(true).block_on(async {
            let cancellation = CancellationToken::new();
            let mut first = bounded.fetch(hostname(), cancellation.clone());
            assert!(poll_once(first.as_mut(), Waker::noop()).is_pending());
            capture_gate.entered();
            assert_eq!(bounded.permits.available_permits(), 0);
            if timeout {
                tokio::time::advance(Duration::from_secs(2)).await;
            } else {
                cancellation.cancel();
            }
            assert_eq!(
                first.await.unwrap_err().kind(),
                if timeout {
                    WebFetchTransportErrorKind::Timeout
                } else {
                    WebFetchTransportErrorKind::Cancelled
                }
            );
            assert_eq!(bounded.permits.available_permits(), 1);
            let mut next = bounded.fetch(hostname(), CancellationToken::new());
            assert!(poll_once(next.as_mut(), Waker::noop()).is_pending());
            assert_eq!(bounded.permits.available_permits(), 0);
            assert_eq!(captures.load(Ordering::Acquire), 1);
            workers.close();
            assert!(!workers.completion().is_complete());
            capture_gate.release();
            // Avoid paused-clock auto-advance while the real worker publishes.
            // Its exact scope join establishes that the cached result is ready.
            workers.completion().wait_on_worker().unwrap();
            assert_eq!(
                next.await.unwrap_err().kind(),
                WebFetchTransportErrorKind::Unavailable
            );
            assert_eq!(bounded.permits.available_permits(), 1);
            assert_eq!(native.nameserver.snapshot().await, Err(unavailable()));
        });
        assert_eq!(captures.load(Ordering::Acquire), 1);
        assert!(workers.completion().is_complete());
    }
}

#[test]
fn captured_configuration_rechecks_cancel_and_deadline_before_dns() {
    for timeout in [false, true] {
        let workers = NativeOwnedWorkerScope::new();
        let (mut capture_gate, hold_capture) = gate();
        let native = transport(&workers, move || {
            hold_capture();
            Ok("127.0.0.1:5353".parse().unwrap())
        });
        runtime(true).block_on(async {
            let cancellation = CancellationToken::new();
            let mut request = hostname();
            request.install_execution_deadline(Instant::now() + Duration::from_secs(2));
            let mut resolving = Box::pin(native.resolve_public_addresses(&request, &cancellation));
            assert!(poll_once(resolving.as_mut(), Waker::noop()).is_pending());
            capture_gate.entered();
            if timeout {
                tokio::time::advance(Duration::from_secs(2)).await;
            } else {
                cancellation.cancel();
            }
            workers.close();
            capture_gate.release();
            workers.completion().wait_on_worker().unwrap();
            assert_eq!(
                resolving.await.unwrap_err().kind(),
                if timeout {
                    WebFetchTransportErrorKind::Timeout
                } else {
                    WebFetchTransportErrorKind::Cancelled
                }
            );
            assert_eq!(
                native
                    .query_ids
                    .as_ref()
                    .unwrap()
                    .counter
                    .load(Ordering::Acquire),
                0,
                "no A/AAAA request is constructed after the capture boundary expires"
            );
        });
        assert!(workers.completion().is_complete());
    }
}
