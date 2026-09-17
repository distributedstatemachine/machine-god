use super::super::preparation::set_worker_hook;
use super::*;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, mpsc};

fn names(id: &SessionId) -> (String, String) {
    let mut digest = Sha256::new();
    digest.update(b"machine-god:file-session:v1:");
    digest.update(id.as_str().as_bytes());
    let stem = format!("session-{:x}", digest.finalize());
    (format!("{stem}.lock"), format!("{stem}.json"))
}

// A watchdog releases a genuinely held native lock if the regression returns.
// It prevents a broken blocking implementation from hanging the complete gate.
fn held_lock(
    f: &FactoryFixture,
    id: &SessionId,
) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let root = f
        .factory
        .0
        .services
        .session_store
        .try_clone_root_descriptor()
        .unwrap();
    let lock = rustix::fs::openat(
        &root,
        names(id).0,
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CREATE | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let (release, receive) = mpsc::channel();
    let joined = std::thread::spawn(move || {
        let result = receive.recv_timeout(Duration::from_secs(5));
        drop(lock);
        assert!(result.is_ok(), "preparation blocked on the session lock");
    });
    (release, joined)
}

fn close(mut child: PreparedManagedRuntime) {
    block_on(poll_fn(|cx| child.resources.poll_closed(cx))).unwrap();
}

#[test]
fn restore_rejects_held_session_lock_without_blocking_or_replacing_transcript() {
    let f = FactoryFixture::new();
    let mut request = f.request("controlled-restore");
    let child = f.prepare(request.clone());
    let original = child.runtime.record();
    close(child);
    request.kind = ManagedRuntimePreparationKind::Restore;
    let (release, joined) = held_lock(&f, &request.transcript.session_id);
    let mut future = f.factory.prepare(request.clone(), CancellationToken::new());
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    assert!(block_on(future).is_err());
    release.send(()).unwrap();
    joined.join().unwrap();
    assert_eq!(
        block_on(
            f.factory
                .0
                .services
                .session_store
                .load(request.transcript.session_id.clone())
        )
        .unwrap(),
        Some(original)
    );
    let restored = f.prepare(request);
    assert!(!restored.runtime.status().active);
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
    close(restored);
}

#[test]
fn contended_create_retains_exact_candidate_and_reconcile_never_replays_publication() {
    let f = FactoryFixture::new();
    let request = f.request("controlled-create");
    let cancel = CancellationToken::new();
    let (release, joined) = held_lock(&f, &request.transcript.session_id);
    // Only the lock exists. The controlled preflight returns absent before
    // lock admission, so this exercises attempted publication, not a Busy read.
    let root = f
        .factory
        .0
        .services
        .session_store
        .try_clone_root_descriptor()
        .unwrap();
    assert!(matches!(
        rustix::fs::statat(
            &root,
            names(&request.transcript.session_id).1,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW
        ),
        Err(rustix::io::Errno::NOENT)
    ));
    let mut future = f.factory.prepare(request.clone(), cancel.clone());
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    let ManagedPreparation::Ambiguous(mut receipt) = block_on(future).unwrap() else {
        panic!("attempted initial publication must retain its exact receipt");
    };
    cancel.cancel();
    assert!(receipt.poll_reconcile(&mut cx).is_pending());
    assert!(block_on(poll_fn(|cx| receipt.poll_reconcile(cx))).is_err());
    release.send(()).unwrap();
    joined.join().unwrap();
    assert!(
        block_on(poll_fn(|cx| receipt.poll_reconcile(cx)))
            .unwrap()
            .is_none()
    );
    assert!(
        block_on(
            f.factory
                .0
                .services
                .session_store
                .load(request.transcript.session_id)
        )
        .unwrap()
        .is_none()
    );
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

struct TlsGate {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Drop for TlsGate {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
thread_local! { static TLS_GATE: std::cell::RefCell<Option<TlsGate>> = const { std::cell::RefCell::new(None) }; }

fn tls_gate() -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered, observed) = mpsc::channel();
    let (release, waiting) = mpsc::channel();
    let gate = Mutex::new(Some(TlsGate {
        entered,
        release: waiting,
    }));
    set_worker_hook(Arc::new(move || {
        TLS_GATE.with(|slot| *slot.borrow_mut() = gate.lock().unwrap().take());
    }));
    (observed, release)
}

#[test]
fn create_restore_and_reconciliation_wait_for_actual_owned_worker_tls_completion() {
    let f = FactoryFixture::new();
    let request = f.request("controlled-tls");
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    for restore in [false, true] {
        let mut selected = request.clone();
        if restore {
            selected.kind = ManagedRuntimePreparationKind::Restore;
        }
        let (entered, release) = tls_gate();
        let mut future = f.factory.prepare(selected, CancellationToken::new());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            future.as_mut().poll(&mut cx).is_pending(),
            "worker result is not TLS settlement"
        );
        release.send(()).unwrap();
        let ManagedPreparation::Ready(child) = block_on(future).unwrap() else {
            panic!("initial publication and exact restoration should confirm");
        };
        close(child);
    }
    let request = f.request("controlled-reconcile-tls");
    REJECT_PUBLICATION.with(|reject| reject.set(true));
    let ManagedPreparation::Ambiguous(mut receipt) =
        block_on(f.factory.prepare(request, CancellationToken::new())).unwrap()
    else {
        panic!("injected observation failure must retain the candidate");
    };
    let (entered, release) = tls_gate();
    assert!(receipt.poll_reconcile(&mut cx).is_pending());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        receipt.poll_reconcile(&mut cx).is_pending(),
        "reconciliation handoff must join the original worker"
    );
    release.send(()).unwrap();
    close(
        block_on(poll_fn(|cx| receipt.poll_reconcile(cx)))
            .unwrap()
            .unwrap(),
    );
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn cancelled_and_abandoned_restore_settle_original_worker_without_touching_saved_history() {
    for abandoned in [false, true] {
        let f = FactoryFixture::new();
        let mut request = f.request("controlled-cancel");
        let child = f.prepare(request.clone());
        let original = child.runtime.record();
        close(child);
        request.kind = ManagedRuntimePreparationKind::Restore;
        let cancel = CancellationToken::new();
        let (entered, observed) = mpsc::channel();
        let (release, waiting) = mpsc::channel();
        let waiting = Mutex::new(waiting);
        set_worker_hook(Arc::new(move || {
            entered.send(()).unwrap();
            waiting
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }));
        let mut future = f.factory.prepare(request.clone(), cancel.clone());
        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        if abandoned {
            drop(future);
            release.send(()).unwrap();
        } else {
            cancel.cancel();
            release.send(()).unwrap();
            assert!(block_on(future).is_err());
        }
        // Scope completion includes the abandoned worker, not its response observer.
        let workers = f.factory.0.services.control_workers.as_ref().unwrap();
        workers.close();
        block_on(workers.completion().wait());
        assert_eq!(
            block_on(
                f.factory
                    .0
                    .services
                    .session_store
                    .load(request.transcript.session_id)
            )
            .unwrap(),
            Some(original)
        );
        assert!(f.host.transport.requests.lock().unwrap().is_empty());
    }
}
