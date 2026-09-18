use super::*;
use std::sync::mpsc;
use std::task::{Context, Waker};

fn delayed_stream_cleanup(
    fixture: &Fixture,
    scope: crate::NativeOwnedWorkerScope,
) -> mpsc::Sender<()> {
    let (release, receiver) = mpsc::channel();
    *fixture.transport.stream_drop.lock().unwrap() = Some(Box::new(move || {
        scope
            .spawn(move || {
                // Dropping the sender on a failed assertion also releases the worker.
                let _ = receiver.recv_timeout(Duration::from_secs(30));
            })
            .unwrap();
    }));
    release
}

#[test]
fn foreground_successor_waits_for_actual_cleanup_without_failure_or_replay() {
    for cancel in [false, true] {
        let mut fixture = Fixture::with_options("auto", true, options);
        let path = journal_path(&fixture);
        run(async {
            let host = fixture.host.take().unwrap();
            let completion = host.terminal_shutdown_completion().unwrap();
            let release = delayed_stream_cleanup(&fixture, host.control_workers().unwrap());
            let options = NativeInteractiveSessionOptions::new(
                fixture.workspace.clone(),
                host.loaded_config().config().model_preferences(),
            )
            .unwrap();
            let mut owner = NativeInteractiveSession::open_managed(
                host,
                directory(&path),
                options,
                NativeInteractiveInitialSession::Fresh,
                1,
            )
            .await
            .unwrap();
            owner.enqueue("first".into()).unwrap();
            assert!(matches!(
                outcome(&mut owner).await,
                NativeInteractiveOutcome::Turn(Ok(_))
            ));
            owner.enqueue("successor".into()).unwrap();
            let _ = owner.poll_progress(&mut Context::from_waker(Waker::noop()), 11);
            assert!(
                owner.take_outcome().is_none(),
                "cleanup is not a failed successor"
            );
            assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
            if cancel {
                assert!(
                    owner.request_cancel(),
                    "waiting admission still owns cancellation"
                );
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeInteractiveOutcome::Turn(Err(_))
                ));
                assert_eq!(owner.runtime().status().queued_jobs, 0);
            }
            release.send(()).unwrap();
            if !cancel {
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeInteractiveOutcome::Turn(Ok(_))
                ));
            }
            owner.request_shutdown();
            while !owner.is_closed() {
                let _ = outcome(&mut owner).await;
            }
            drop(owner);
            completion.wait().await;
            assert_eq!(
                fixture.transport.requests.lock().unwrap().len(),
                if cancel { 1 } else { 2 }
            );
        });
    }
}

#[test]
fn acp_capacity_wait_retains_prompt_correlation_and_cancellation_without_replay() {
    for cancel in [false, true] {
        let mut fixture = Fixture::with_options("auto", true, options);
        run(async {
            let (mut session, host) = acp::open(&mut fixture).await;
            let completion = host.terminal_shutdown_completion().unwrap();
            let scope = host.control_workers().unwrap();
            let mut occupied = Vec::new();
            while let Ok(cohort) = scope.begin_run() {
                occupied.push(cohort);
            }
            assert!(!occupied.is_empty());
            let id = session.id();
            let input = || {
                crate::acp::prompt::decode_prompt_input(&serde_json::json!({
                    "prompt": [{"type":"text", "text":"one correlated prompt"}]
                }))
                .unwrap()
            };
            session.enqueue(&id, input()).unwrap();
            let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 10);
            assert!(
                session.take_outcome().is_none(),
                "capacity is not a failed prompt response"
            );
            assert!(session.has_pending_prompt());
            assert!(
                session.enqueue(&id, input()).is_err(),
                "original request still owns response lane"
            );
            assert!(fixture.transport.requests.lock().unwrap().is_empty());
            if cancel {
                assert!(session.request_cancel(&id).unwrap());
            } else {
                occupied.pop();
            }
            let result = poll_fn(|cx| {
                let _ = session.poll_progress(cx, 11);
                let _ = session.take_presentation();
                session.take_outcome().map_or(Poll::Pending, Poll::Ready)
            })
            .await;
            assert!(matches!(result, NativeInteractiveOutcome::Turn(_)));
            assert!(!session.has_pending_prompt());
            assert_eq!(session.runtime().status().queued_jobs, 0);
            drop(occupied);
            session.request_close(&id).unwrap();
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 12);
                let _ = session.take_presentation();
                let _ = session.take_outcome();
                if session.is_closed() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
            drop(session);
            drop(host);
            completion.wait().await;
            assert_eq!(
                fixture.transport.requests.lock().unwrap().len(),
                usize::from(!cancel)
            );
        });
    }
}
