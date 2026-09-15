use super::*;
use crate::mcp::{
    peer::{McpPeerTimer, McpStdioPeer},
    stdio::{McpStdioConnection, testing::Pipe},
};
use std::sync::mpsc::sync_channel;

struct HandoffTimer(Instant);

impl McpPeerTimer for HandoffTimer {
    fn now(&self) -> Instant {
        self.0
    }

    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

fn selected(
    runtime: &NativeMcpRuntime,
    connection: McpStdioConnection,
) -> NativeMcpRuntimeCandidate {
    runtime
        .prepare_candidate(vec![server(connection)], &[])
        .unwrap()
}

fn server(connection: McpStdioConnection) -> NativeMcpServerCandidate {
    NativeMcpServerCandidate {
        server: Arc::from("isolated"),
        configuration: Arc::from(&b"configuration"[..]),
        authentication: Arc::from(&b"authentication"[..]),
        catalogs: vec![],
        refresh: None,
        catalog_epoch: Instant::now(),
        peer: NativeMcpOwnedPeer::Stdio(McpStdioPeer::inert_for_test(
            connection,
            Arc::new(HandoffTimer(Instant::now())),
        )),
        operation_timeout: Duration::from_secs(5),
        authority_cancellations: Arc::from([]),
    }
}

#[test]
fn deferred_addition_transfers_only_after_successful_exact_publication() {
    for accepted in [false, true] {
        let runtime = standalone();
        let initial = candidate(&runtime, "initial", &[], Arc::default());
        let checkpoint = initial.publication_checkpoint();
        runtime.publish(initial).unwrap();
        let connection = McpStdioConnection::inert_for_test();
        let pipe = Pipe::new(&connection);
        let addition = runtime
            .prepare_addition(vec![server(connection)], &[], &checkpoint)
            .unwrap();
        assert!(!pipe.promote_service());
        if accepted {
            runtime.publish_addition(addition).unwrap();
        } else {
            runtime
                .publish(candidate(&runtime, "replacement", &[], Arc::default()))
                .unwrap();
            assert!(runtime.publish_addition(addition).is_err());
        }
        assert_eq!(pipe.promote_service(), accepted);
        runtime.close();
    }
}

#[test]
fn only_successful_exact_publication_promotes_original_stdio_service() {
    for outcome in 0..5 {
        let runtime = standalone();
        let connection = McpStdioConnection::inert_for_test();
        let pipe = Pipe::new(&connection);
        let candidate = selected(&runtime, connection);
        // Negotiated peer and private candidate preparation are not activation.
        assert!(!pipe.promote_service());
        match outcome {
            0 => runtime.publish(candidate).unwrap(),
            1 => drop(candidate),
            2 => {
                runtime.close();
                assert!(runtime.publish(candidate).is_err());
            }
            3 => assert!(standalone().publish(candidate).is_err()),
            _ => {
                let previous = super::candidate(&runtime, "previous", &[], Arc::default());
                let checkpoint = previous.publication_checkpoint();
                runtime.publish(previous).unwrap();
                runtime
                    .publish(super::candidate(
                        &runtime,
                        "replacement",
                        &[],
                        Arc::default(),
                    ))
                    .unwrap();
                assert!(runtime.publish_if(candidate, &checkpoint).is_err());
            }
        }
        assert_eq!(pipe.promote_service(), outcome == 0);
        runtime.close();
    }
}

#[test]
fn published_service_releases_original_run_but_keeps_actual_host_cleanup() {
    let runtime = standalone();
    let host = NativeOwnedWorkerScope::new();
    let run = host.begin_run().unwrap();
    let sibling = host.begin_run().unwrap();
    let connection = McpStdioConnection::inert_for_test();
    let owner_pipe = Pipe::new(&connection);
    let child_pipe = Pipe::new(&connection);
    let candidate = selected(&runtime, connection);
    let (ready, ready_rx) = sync_channel(1);
    let (publish, publish_rx) = sync_channel(1);
    let (promoted, promoted_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    let source_host = host.clone();
    run.with_poll(|| {
        host.spawn(move || {
            owner_pipe.register_handoff_owner();
            let child = NativeOwnedWorkerScope::new();
            child
                .with_inherited_run_from(&source_host, || {
                    child.spawn(move || {
                        child_pipe.register_handoff_child();
                        let mut process_cleanup =
                            NativeOwnedWorkerScope::retain_current_cleanup().unwrap();
                        assert!(!child_pipe.promote_service());
                        ready.send(process_cleanup.clone()).unwrap();
                        publish_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                        assert!(child_pipe.promote_service());
                        process_cleanup.promote_to_service();
                        promoted.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    })
                })
                .unwrap()
                .unwrap();
            child.close();
            child.completion().wait_on_worker().unwrap();
        })
    })
    .unwrap();
    let failed_cleanup = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    run.close();
    sibling.close();
    assert!(sibling.completion().is_complete());
    assert!(!run.completion().is_complete());
    runtime.publish(candidate).unwrap();
    publish.send(()).unwrap();
    promoted_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!run.completion().is_complete());
    drop(failed_cleanup);
    run.completion().wait_on_worker().unwrap();
    host.close();
    assert!(!host.completion().is_complete());
    runtime.close();
    release.send(()).unwrap();
    host.completion().wait_on_worker().unwrap();
}

#[test]
fn rejected_publication_retains_original_run_through_actual_worker_tls() {
    struct TlsGate(
        std::sync::mpsc::SyncSender<()>,
        std::sync::mpsc::Receiver<()>,
    );
    impl Drop for TlsGate {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
            self.1.recv_timeout(Duration::from_secs(10)).unwrap();
        }
    }
    thread_local! {
        static GATE: std::cell::RefCell<Option<TlsGate>> = const { std::cell::RefCell::new(None) };
    }
    let runtime = standalone();
    let host = NativeOwnedWorkerScope::new();
    let run = host.begin_run().unwrap();
    let connection = McpStdioConnection::inert_for_test();
    let pipe = Pipe::new(&connection);
    let candidate = selected(&runtime, connection);
    let (ready, ready_rx) = sync_channel(1);
    let (reject, reject_rx) = sync_channel(1);
    let (entered, entered_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    run.with_poll(|| {
        host.spawn(move || {
            pipe.register_handoff_owner();
            GATE.with(|slot| *slot.borrow_mut() = Some(TlsGate(entered, release_rx)));
            ready.send(()).unwrap();
            reject_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(!pipe.promote_service());
        })
    })
    .unwrap();
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    runtime.close();
    assert!(runtime.publish(candidate).is_err());
    run.close();
    reject.send(()).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!run.completion().is_complete());
    release.send(()).unwrap();
    run.completion().wait_on_worker().unwrap();
    host.close();
    host.completion().wait_on_worker().unwrap();
}
