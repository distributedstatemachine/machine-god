//! Composed command scenarios use the real owner/runtime/journals and scripted
//! scheduling boundaries, never a browser or reconstructed process authority.

use super::*;
use crate::background_commands::service::{
    BackgroundRequests, NativeBackgroundControlError as Error,
    NativeBackgroundControlReceipt as Receipt, execute_with,
};
use crate::terminal_journal::{TerminalJournal, TerminalJournalMutation};
use crate::terminal_profile::TerminalJournalPersistence;
use crate::{
    NativeBackgroundCommand as Command, NativeBackgroundOpenOutcome,
    NativeBackgroundTarget as Target, NativeBackgroundUrlOpener, NativeOwnedWorkerScope,
};

#[derive(Default)]
struct Script {
    selections: usize,
    inspections: usize,
    reads: usize,
    handoff_after_selection: bool,
    close_before_second_inspection: bool,
    revoke_after_second_inspection: bool,
    cancel_after_second_inspection: Option<CancellationToken>,
    after_first_read: Option<Vec<u8>>,
    page_cap: Option<usize>,
    gap_after_read: Option<usize>,
}

#[derive(Clone)]
struct Adapter {
    requester: TerminalRuntimeRequester<Backend, HostState>,
    principals: TerminalAccessPrincipals,
    script: Arc<Mutex<Script>>,
}
impl Adapter {
    fn new(fixture: &Fixture) -> Self {
        Self {
            requester: fixture.requester(),
            principals: fixture.principals.clone(),
            script: Arc::new(Mutex::new(Script::default())),
        }
    }
    fn action(
        &self,
        target: NativeTerminalBackgroundTarget,
        request: TerminalActionRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<crate::terminal_host_dispatch::TerminalHostReply>> {
        action_request(
            self.requester.clone(),
            self.principals.clone(),
            target,
            request,
            cancellation,
        )
    }
}

impl BackgroundRequests for Adapter {
    fn snapshot(
        &self,
        owner: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundSnapshot>> {
        snapshot_request(
            self.requester.clone(),
            self.principals.clone(),
            owner,
            cancellation,
        )
    }
    fn select(
        &self,
        owner: BackgroundOutputOwner,
        id: Option<TerminalSessionId>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundTarget>> {
        let adapter = self.clone();
        Box::pin(async move {
            let target = select_request(
                adapter.requester.clone(),
                adapter.principals.clone(),
                owner,
                id,
                cancellation,
            )
            .await?;
            let handoff = {
                let mut script = adapter.script.lock().unwrap();
                script.selections += 1;
                script.handoff_after_selection
            };
            if handoff {
                adapter
                    .requester
                    .request_with_context(CancellationToken::new(), |context| {
                        lifecycle::handoff(context, &super::owner("a"), &super::owner("b"))
                    })
                    .await
                    .unwrap()
                    .unwrap();
            }
            Ok(target)
        })
    }
    fn inspect(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundInspection>> {
        let adapter = self.clone();
        let target = target.clone();
        Box::pin(async move {
            let close = {
                let mut script = adapter.script.lock().unwrap();
                script.inspections += 1;
                script.close_before_second_inspection && script.inspections == 2
            };
            if close {
                adapter.stop(&target, CancellationToken::new()).await?;
            }
            let request = TerminalActionRequest::Inspect {
                session_id: target.id().clone(),
                events: TerminalEventQuery {
                    after_event_id: 0,
                    acknowledge_event_id: None,
                    max_events: 1,
                },
            };
            let reply = adapter.action(target, request, cancellation).await?;
            let (revoke, cancel) = {
                let script = adapter.script.lock().unwrap();
                if script.inspections == 2 {
                    (
                        script.revoke_after_second_inspection,
                        script.cancel_after_second_inspection.clone(),
                    )
                } else {
                    (false, None)
                }
            };
            if revoke {
                adapter.principals.retire_all();
            }
            if let Some(cancel) = cancel {
                cancel.cancel();
            }
            Ok(NativeTerminalBackgroundInspection {
                result: reply.result,
                owned_backend_at_admission: reply.owned_backend_at_admission,
            })
        })
    }
    fn read(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cursor: TerminalCursor,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundPage>> {
        let adapter = self.clone();
        let target = target.clone();
        Box::pin(async move {
            let (maximum, append, gap) = {
                let mut script = adapter.script.lock().unwrap();
                script.reads += 1;
                (
                    script.page_cap.map_or(maximum, |cap| cap.min(maximum)),
                    if script.reads == 1 {
                        script.after_first_read.take()
                    } else {
                        None
                    },
                    script
                        .gap_after_read
                        .is_some_and(|read| script.reads > read),
                )
            };
            let mut page = read_request(
                adapter.requester.clone(),
                adapter.principals.clone(),
                target.clone(),
                cursor.clone(),
                maximum,
                cancellation,
            )
            .await?;
            if gap {
                // Explicit scheduling seam: retained output disappeared after
                // the first page. The service must not use this next span.
                page.page.gap = Some(TerminalGap::new(cursor, page.next().clone()).unwrap());
            }
            if let Some(bytes) = append {
                append_history(&adapter, target.id().clone(), bytes).await;
            }
            Ok(page)
        })
    }
    fn tail_start(
        &self,
        target: &NativeTerminalBackgroundTarget,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalCursor>> {
        tail_request(
            self.requester.clone(),
            self.principals.clone(),
            target.clone(),
            maximum,
            cancellation,
        )
    }
    fn stop(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundStopReceipt>> {
        let request = TerminalActionRequest::Close {
            session_id: target.id().clone(),
            policy: TerminalClosePolicy::Graceful,
        };
        let action = self.action(target.clone(), request, cancellation);
        Box::pin(async move {
            let reply = action.await?;
            Ok(NativeTerminalBackgroundStopReceipt {
                result: reply.result,
                was_live: reply.owned_backend_at_admission,
            })
        })
    }
}

async fn append_history(adapter: &Adapter, id: TerminalSessionId, bytes: Vec<u8>) {
    adapter
        .requester
        .request_with_context(CancellationToken::new(), move |context| {
            let catalog = context
                .state
                .catalogs
                .catalog(context.store, &owner("a"), context.cancellation)
                .unwrap();
            let snapshot = catalog.snapshot().unwrap();
            let mut journal =
                TerminalJournal::open_for_retention(snapshot.open(&id).unwrap(), &id).unwrap();
            let mut transaction = context.store.transaction().unwrap();
            let mut persistence = TerminalProfileMutationContext::new(
                &mut transaction,
                *context.budget,
                catalog.namespace_key(),
            );
            let receipt = persistence
                .mutate(&mut journal, TerminalJournalMutation::Append(&bytes))
                .unwrap();
            receipt.operation.unwrap();
            receipt.accounting.unwrap();
        })
        .await
        .unwrap();
}

fn small_history(bytes: Vec<u8>) -> Fixture {
    let mut row = Row::new(1, 10, true);
    row.bytes = bytes;
    row.history_limits = Some(TerminalJournalLimits {
        segment_bytes: 256,
        session_bytes: 16 * 1024,
    });
    Fixture::new(vec![row])
}

fn execute(adapter: Adapter, command: Command) -> std::result::Result<Receipt, Error> {
    block_on(execute_with(
        adapter,
        owner("a"),
        command,
        None,
        CancellationToken::new(),
    ))
}

struct Launcher {
    opener: NativeBackgroundUrlOpener,
    workers: NativeOwnedWorkerScope,
}
impl Launcher {
    fn new() -> Self {
        let program = PathBuf::from("/usr/bin/true");
        let workers = NativeOwnedWorkerScope::new();
        let opener = NativeBackgroundUrlOpener::new(
            program.clone(),
            std::fs::File::open(program).unwrap(),
            vec![],
            workers.clone(),
        )
        .unwrap();
        Self { opener, workers }
    }
}
impl Drop for Launcher {
    fn drop(&mut self) {
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
    }
}

#[test]
fn list_and_last_preserve_exact_creation_order_and_history_ownership() {
    let fixture = Fixture::new(vec![
        Row::new(1, 10, false),
        Row::new(2, 20, true),
        Row::new(3, 20, true),
    ]);
    let adapter = Adapter::new(&fixture);
    let Receipt::Listed(snapshot) = execute(adapter.clone(), Command::List).unwrap() else {
        panic!("list receipt")
    };
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(|entry| entry.id().clone())
            .collect::<Vec<_>>(),
        vec![id(3), id(2), id(1)]
    );
    let Receipt::Logs(logs) = execute(adapter.clone(), Command::Logs(Target::Last)).unwrap() else {
        panic!("logs receipt")
    };
    assert_eq!(logs.session_id(), &id(3));
    assert_eq!(adapter.script.lock().unwrap().selections, 1);
    assert!(!format!("{logs:?}").contains("hello"));
    assert_eq!(fixture.backend.lock().unwrap().closes, 0);
}

#[test]
fn logs_are_binary_lossless_bounded_and_cross_segment_tail_boundaries() {
    let bytes: Vec<u8> = (0..139_264)
        .map(|n| u8::try_from(n % 256).unwrap())
        .collect();
    let mut row = Row::new(1, 10, true);
    row.bytes = bytes.clone();
    let fixture = Fixture::new(vec![row]);
    let Receipt::Logs(logs) = execute(Adapter::new(&fixture), Command::Logs(Target::Last)).unwrap()
    else {
        panic!("logs receipt")
    };
    assert_eq!(logs.head().bytes(), &bytes[..16 * 1024]);
    assert_eq!(logs.tail().bytes(), &bytes[bytes.len() - 16 * 1024..]);
    assert!(logs.head().truncated());
    assert!(!logs.tail().truncated());
    assert!(!logs.head().has_gap());
    assert!(!logs.tail().has_gap());
    assert_eq!(
        logs.head().bytes().len() + logs.tail().bytes().len(),
        32 * 1024
    );
}

#[test]
fn empty_and_retained_prefix_logs_are_explicit() {
    let empty = small_history(vec![]);
    let Receipt::Logs(logs) = execute(Adapter::new(&empty), Command::Logs(Target::Last)).unwrap()
    else {
        panic!("empty logs receipt")
    };
    assert!(logs.head().bytes().is_empty());
    assert!(logs.tail().bytes().is_empty());
    assert!(!logs.head().truncated());
    let bytes = vec![b'x'; 30_000];
    let retained = small_history(bytes);
    let Receipt::Logs(logs) =
        execute(Adapter::new(&retained), Command::Logs(Target::Last)).unwrap()
    else {
        panic!("retained logs receipt")
    };
    assert!(logs.head().has_gap());
    assert!(logs.head().source().segment() > 1);
    assert!(logs.head().bytes().iter().all(|byte| *byte == b'x'));
    assert!(!logs.tail().has_gap());
    assert_eq!(retained.backend.lock().unwrap().closes, 0);
}

#[test]
fn snapshot_end_clips_growth_after_cross_segment_read() {
    let original = vec![b'a'; 356];
    let fixture = small_history(original.clone());
    let adapter = Adapter::new(&fixture);
    adapter.script.lock().unwrap().after_first_read = Some(vec![b'b'; 100]);
    let Receipt::Logs(logs) = execute(adapter, Command::Logs(Target::Last)).unwrap() else {
        panic!("logs receipt")
    };
    assert_eq!(logs.head().bytes(), original);
    assert_eq!(logs.head().next(), logs.head().snapshot_end());
    assert_eq!(
        logs.head().snapshot_end(),
        &TerminalCursor::new(2, 100).unwrap()
    );
    assert_eq!(logs.tail().bytes().len(), 456);
    assert_eq!(&logs.tail().bytes()[356..], &[b'b'; 100]);
}

#[test]
fn retention_between_pages_never_joins_discontinuous_output() {
    let fixture = small_history(vec![b'a'; 700]);
    let adapter = Adapter::new(&fixture);
    adapter.script.lock().unwrap().after_first_read = Some(vec![b'b'; 20_000]);
    let Receipt::Logs(logs) = execute(adapter, Command::Logs(Target::Last)).unwrap() else {
        panic!("logs receipt")
    };
    assert_eq!(logs.head().bytes(), &[b'a'; 256]);
    assert!(logs.head().has_gap());
    assert!(logs.head().truncated());
    assert!(logs.tail().bytes().iter().all(|byte| *byte == b'b'));
}

#[test]
fn stop_exact_last_history_and_post_commit_cancellation_keep_receipts() {
    let fixture = Fixture::new(vec![Row::new(1, 10, false), Row::new(2, 20, true)]);
    let adapter = Adapter::new(&fixture);
    let Receipt::Stopped {
        session_id,
        receipt,
    } = execute(adapter.clone(), Command::Stop(Target::Last)).unwrap()
    else {
        panic!("stop receipt")
    };
    assert_eq!(session_id, id(2));
    assert!(!receipt.was_live());
    let cancel = CancellationToken::new();
    fixture.backend.lock().unwrap().cancel_on_close = Some(cancel.clone());
    let Receipt::Stopped {
        session_id,
        receipt,
    } = block_on(execute_with(
        adapter,
        owner("a"),
        Command::Stop(Target::Session(id(1))),
        None,
        cancel.clone(),
    ))
    .unwrap()
    else {
        panic!("live stop receipt")
    };
    assert_eq!(session_id, id(1));
    assert!(receipt.was_live());
    assert!(cancel.is_cancelled());
    assert!(!fixture.backend.lock().unwrap().forced);
    assert_eq!(fixture.backend.lock().unwrap().closes, 1);
}

#[test]
fn selection_handoff_revokes_without_reselecting_last_or_closing() {
    let fixture = Fixture::new(vec![Row::new(1, 10, false)]);
    let adapter = Adapter::new(&fixture);
    adapter.script.lock().unwrap().handoff_after_selection = true;
    assert!(matches!(
        execute(adapter.clone(), Command::Stop(Target::Last)),
        Err(Error::Terminal(NativeTerminalBackgroundError::Revoked))
    ));
    assert_eq!(adapter.script.lock().unwrap().selections, 1);
    assert_eq!(fixture.backend.lock().unwrap().closes, 0);
    let Receipt::Stopped {
        session_id,
        receipt,
    } = block_on(execute_with(
        Adapter::new(&fixture),
        owner("b"),
        Command::Stop(Target::Last),
        None,
        CancellationToken::new(),
    ))
    .unwrap()
    else {
        panic!("transferred stop receipt")
    };
    assert_eq!(session_id, id(1));
    assert!(receipt.was_live());
}

#[test]
fn dropped_and_cancelled_before_poll_commands_are_inert() {
    let fixture = Fixture::new(vec![]);
    for command in [
        Command::List,
        Command::Stop(Target::Last),
        Command::Logs(Target::Last),
        Command::Open(Target::Last),
    ] {
        drop(execute_with(
            Adapter::new(&fixture),
            owner("a"),
            command.clone(),
            None,
            CancellationToken::new(),
        ));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            block_on(execute_with(
                Adapter::new(&fixture),
                owner("a"),
                command,
                None,
                cancel
            )),
            Err(Error::Terminal(NativeTerminalBackgroundError::Cancelled))
        ));
    }
    assert_eq!(fixture.initialized.load(Ordering::SeqCst), 0);
}

#[test]
fn dropping_an_admitted_service_stop_keeps_native_effect_ownership() {
    let fixture = Fixture::new(vec![Row::new(1, 10, false)]);
    fixture.snapshot().unwrap();
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    fixture.backend.lock().unwrap().gate = Some((entered, wait));
    let mut operation = execute_with(
        Adapter::new(&fixture),
        owner("a"),
        Command::Stop(Target::Last),
        None,
        CancellationToken::new(),
    );
    let mut context = Context::from_waker(Waker::noop());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(operation.as_mut().poll(&mut context).is_pending());
        if observed.try_recv().is_ok() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "close admission deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(operation);
    release.send(()).unwrap();
    fixture.snapshot().unwrap();
    assert_eq!(fixture.backend.lock().unwrap().closes, 1);
    assert!(!fixture.backend.lock().unwrap().forced);
}

#[test]
fn empty_list_and_uncertain_stop_are_not_reinterpreted_as_success() {
    let empty = Fixture::new(vec![]);
    let Receipt::Listed(snapshot) = execute(Adapter::new(&empty), Command::List).unwrap() else {
        panic!("list receipt")
    };
    assert!(snapshot.entries().is_empty());
    let failing = Fixture::new(vec![Row::new(1, 10, false)]);
    failing.snapshot().unwrap();
    failing.backend.lock().unwrap().fail_once = true;
    let adapter = Adapter::new(&failing);
    assert!(matches!(
        execute(adapter.clone(), Command::Stop(Target::Last)),
        Err(Error::Terminal(NativeTerminalBackgroundError::Uncertain))
    ));
    assert_eq!(adapter.script.lock().unwrap().selections, 1);
}

#[test]
fn open_distinguishes_unknown_history_and_no_url_without_native_control() {
    let fixture = Fixture::new(vec![Row::new(1, 10, false), Row::new(2, 20, true)]);
    assert!(matches!(
        execute(
            Adapter::new(&fixture),
            Command::Open(Target::Session(id(99)))
        ),
        Err(Error::Terminal(NativeTerminalBackgroundError::NotFound))
    ));
    assert!(
        matches!(execute(Adapter::new(&fixture), Command::Open(Target::Last)).unwrap(), Receipt::NotRunning { session_id } if session_id == id(2))
    );
    let launcher = Launcher::new();
    let receipt = block_on(execute_with(
        Adapter::new(&fixture),
        owner("a"),
        Command::Open(Target::Session(id(1))),
        Some(launcher.opener.clone()),
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(matches!(receipt, Receipt::NoKnownUrl { session_id } if session_id == id(1)));
    assert_eq!(fixture.backend.lock().unwrap().closes, 0);
}

#[test]
fn explicit_nonbrowser_launcher_and_final_liveness_recheck() {
    for close_before_launch in [false, true] {
        let mut row = Row::new(1, 10, false);
        row.bytes = b"ready http://localhost:3000/private-service\n".to_vec();
        let fixture = Fixture::new(vec![row]);
        let adapter = Adapter::new(&fixture);
        adapter
            .script
            .lock()
            .unwrap()
            .close_before_second_inspection = close_before_launch;
        let launcher = Launcher::new();
        let receipt = block_on(execute_with(
            adapter.clone(),
            owner("a"),
            Command::Open(Target::Last),
            Some(launcher.opener.clone()),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(adapter.script.lock().unwrap().selections, 1);
        assert_eq!(adapter.script.lock().unwrap().inspections, 2);
        assert!(!format!("{receipt:?}").contains("private-service"));
        if close_before_launch {
            assert!(matches!(receipt, Receipt::NotRunning { session_id } if session_id == id(1)));
        } else {
            assert!(
                matches!(receipt, Receipt::Opened { session_id, url, outcome: NativeBackgroundOpenOutcome::Opened } if session_id == id(1) && url == "http://localhost:3000/private-service")
            );
        }
    }
}

#[test]
fn final_url_admission_rejects_cancellation_and_generation_revocation() {
    for revoke in [false, true] {
        let mut row = Row::new(1, 10, false);
        row.bytes = b"http://localhost:3000/ready\n".to_vec();
        let fixture = Fixture::new(vec![row]);
        let adapter = Adapter::new(&fixture);
        let cancel = CancellationToken::new();
        {
            let mut script = adapter.script.lock().unwrap();
            script.revoke_after_second_inspection = revoke;
            script.cancel_after_second_inspection = (!revoke).then(|| cancel.clone());
        }
        let launcher = Launcher::new();
        assert!(matches!(
            block_on(execute_with(
                adapter,
                owner("a"),
                Command::Open(Target::Last),
                Some(launcher.opener.clone()),
                cancel
            )),
            Err(Error::Open(crate::NativeBackgroundOpenError::Cancelled))
        ));
        assert_eq!(fixture.backend.lock().unwrap().closes, 0);
    }
}

#[test]
fn url_evidence_and_page_iterations_are_bounded_and_truncation_is_explicit() {
    let mut row = Row::new(1, 10, false);
    row.bytes = vec![b'x'; 65_536];
    row.bytes
        .extend_from_slice(b"\nhttp://localhost:3000/outside-bound\n");
    let fixture = Fixture::new(vec![row]);
    let adapter = Adapter::new(&fixture);
    let launcher = Launcher::new();
    let receipt = block_on(execute_with(
        adapter.clone(),
        owner("a"),
        Command::Open(Target::Last),
        Some(launcher.opener.clone()),
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(matches!(receipt, Receipt::NoKnownUrl { .. }));
    assert_eq!(adapter.script.lock().unwrap().reads, 1);

    let paged = small_history(vec![b'a'; 2048]);
    let adapter = Adapter::new(&paged);
    adapter.script.lock().unwrap().page_cap = Some(1);
    let Receipt::Logs(logs) = execute(adapter.clone(), Command::Logs(Target::Last)).unwrap() else {
        panic!("logs receipt")
    };
    assert_eq!(adapter.script.lock().unwrap().reads, 2048);
    assert_eq!(logs.head().bytes(), &[b'a'; 1024]);
    assert_eq!(logs.tail().bytes(), &[b'a'; 1024]);
    assert!(logs.head().truncated());
    assert!(logs.tail().truncated());
}

#[test]
fn url_capture_boundaries_never_open_incomplete_candidates() {
    for (boundary, page_cap, retention_gap) in [
        (65_536, None, false),
        (1024, Some(1), false),
        (256, Some(256), true),
    ] {
        for oversized in [false, true] {
            for earlier_complete in [false, true] {
                let mut row = Row::new(1, 10, false);
                row.bytes = vec![b' '; boundary - b"http://localhost:3000/".len()];
                if earlier_complete {
                    let earlier = b"http://example.test/complete\n";
                    row.bytes[..earlier.len()].copy_from_slice(earlier);
                }
                row.bytes.extend_from_slice(b"http://localhost:3000/");
                row.bytes.extend_from_slice(if oversized {
                    &[b'a'; 2100]
                } else {
                    b"private-path"
                });
                row.bytes.push(b'\n');
                let fixture = Fixture::new(vec![row]);
                let adapter = Adapter::new(&fixture);
                {
                    let mut script = adapter.script.lock().unwrap();
                    script.page_cap = page_cap;
                    script.gap_after_read = retention_gap.then_some(1);
                }
                let launcher = Launcher::new();
                let receipt = block_on(execute_with(
                    adapter.clone(),
                    owner("a"),
                    Command::Open(Target::Last),
                    Some(launcher.opener.clone()),
                    CancellationToken::new(),
                ))
                .unwrap();
                if earlier_complete {
                    assert!(matches!(receipt, Receipt::Opened {
                        url, outcome: NativeBackgroundOpenOutcome::Opened, ..
                    } if url == "http://example.test/complete"));
                } else {
                    assert!(
                        matches!(receipt, Receipt::NoKnownUrl { .. }),
                        "boundary={boundary}, oversized={oversized}"
                    );
                }
                assert_eq!(
                    adapter.script.lock().unwrap().reads,
                    if retention_gap {
                        2
                    } else if page_cap.is_some() {
                        1024
                    } else {
                        1
                    }
                );
                assert_eq!(fixture.backend.lock().unwrap().closes, 0);
            }
        }
    }
}
