//! Both phases of an ambiguous clear use the real native retry policy.
use super::*;
use crate::{mcp::runtime::NativeMcpRuntimeClock, reference_host::ManagedRecovery};
use std::{
    future::poll_fn,
    sync::Mutex,
    time::{Duration, Instant},
};

const DRIVE_LIMIT: Duration = Duration::from_secs(10);

struct Clock {
    now: Mutex<Instant>,
    wake: AtomicWaker,
}
impl Clock {
    fn advance(&self, duration: Duration) {
        *self.now.lock().unwrap() += duration;
        self.wake.wake();
    }
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(poll_fn(move |cx| {
            self.wake.register(cx.waker());
            if self.now() >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Counts {
    loads: usize,
    saves: usize,
}
impl Counts {
    fn read(control: &Control) -> Self {
        Self {
            loads: control.loads.load(Ordering::Acquire),
            saves: control.saves.load(Ordering::Acquire),
        }
    }
}
#[derive(Debug)]
struct Observation {
    before: Counts,
    before_deadline: Counts,
    after: Counts,
    pending: bool,
    charged: bool,
    cleared: bool,
}
struct Report {
    observations: Vec<Observation>,
    original_uncertain: bool,
    final_cleared: bool,
    no_replay: bool,
    residents: usize,
}

async fn drive(f: &mut Fixture, settle: bool, phase: &str) -> Result<(), String> {
    tokio::time::timeout(
        DRIVE_LIMIT,
        poll_fn(|cx| {
            if settle {
                return f
                    .manager
                    .poll_shutdown(cx, 101)
                    .map_err(|error| format!("{phase}: {error:?}"));
            }
            let progress = f.manager.poll_progress(cx, 101);
            if let Poll::Ready(Err(error)) = progress {
                return Poll::Ready(Err(format!("{phase}: {error:?}")));
            }
            if f.manager.progress().recovery_required == Some(ManagerBlock::NoticeClear) {
                return Poll::Ready(Ok(()));
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }),
    )
    .await
    .map_err(|_| format!("{phase}: timed out with {:?}", f.manager.progress()))?
}

async fn exercise(
    f: &mut Fixture,
    store: &InMemorySessionStore,
    control: &Control,
    clock: &Clock,
    recovery: &mut ManagedRecovery,
) -> Result<Report, String> {
    drive(f, false, "initial committed error").await?;
    let receipt = context(f).delivery().ok_or("missing original delivery")?;
    let original_uncertain = !has_outbox(store) && !receipt.is_cleared();
    let prepared = f.factory.prepared.load(Ordering::Acquire);
    let requests = f.factory.provider.requests().len();
    let mut observations = Vec::new();
    f.manager.request_shutdown();
    // A committed error advances the store but not the in-memory expected
    // revision. First retry reconciles that revision and rejects the stale patch;
    // only the following retry may save the exact original clear again.
    for cycle in 0..4 {
        control.error_next.store(cycle < 3, Ordering::Release);
        for save in [false, true] {
            let before = Counts::read(control);
            let mut cx = Context::from_waker(Waker::noop());
            recovery.poll(&f.manager, &mut cx);
            clock.advance(Duration::from_millis(999));
            let mut pending = true;
            for _ in 0..64 {
                pending &= f.manager.poll_shutdown(&mut cx, 101).is_pending();
                recovery.poll(&f.manager, &mut cx);
            }
            let before_deadline = Counts::read(control);
            let charged = f.manager.progress().residents > 0;
            clock.advance(Duration::from_millis(1));
            recovery.poll(&f.manager, &mut cx);
            let phase = format!("cycle {cycle}, {}", if save { "save" } else { "readback" });
            drive(f, cycle == 3 && save, &phase).await?;
            observations.push(Observation {
                before,
                before_deadline,
                after: Counts::read(control),
                pending,
                charged,
                cleared: receipt.is_cleared(),
            });
        }
    }
    Ok(Report {
        observations,
        original_uncertain,
        final_cleared: receipt.is_cleared() && !has_outbox(store),
        no_replay: f.factory.prepared.load(Ordering::Acquire) == prepared
            && f.factory.provider.requests().len() == requests,
        residents: f.manager.progress().residents,
    })
}

pub(super) fn run() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let clock = Arc::new(Clock {
        now: Mutex::new(Instant::now()),
        wake: AtomicWaker::new(),
    });
    let mut recovery = ManagedRecovery::new(clock.clone());
    let (mut f, store, control) = fixture();
    control.error_clear.store(true, Ordering::Release);
    send(&mut f);
    let result = runtime.block_on(exercise(&mut f, &store, &control, &clock, &mut recovery));
    if let Err(error) = &result {
        // An unsuccessful regression must fail visibly rather than park inside
        // Fixture::drop. Disable only fixture faults and try exact-owner cleanup.
        control.error_clear.store(false, Ordering::Release);
        control.error_next.store(false, Ordering::Release);
        control.release();
        f.factory.cleanup.store(true, Ordering::Release);
        f.factory.close_error.store(false, Ordering::Release);
        f.factory.reconcile.store(true, Ordering::Release);
        let cleanup = runtime.block_on(async {
            tokio::time::timeout(DRIVE_LIMIT, async {
                loop {
                    clock.advance(Duration::from_secs(1));
                    let result = poll_fn(|cx| {
                        let result = f.manager.poll_shutdown(cx, 101);
                        recovery.poll(&f.manager, cx);
                        Poll::Ready(result)
                    })
                    .await;
                    if let Poll::Ready(result) = result {
                        return result;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
        });
        if !matches!(cleanup, Ok(Ok(()))) {
            let status = f.manager.progress();
            let counts = Counts::read(&control);
            // Failure-only abandonment of this fake-runtime fixture. This is
            // explicitly not a cleanup receipt or a successful test result.
            std::mem::forget(f);
            panic!("{error}; original cleanup did not settle: {status:?}, {counts:?}");
        }
    }
    // Normal execution settles the original resources before ANY stage assertion.
    drop(f);
    let report = result.unwrap();
    assert!(report.original_uncertain);
    assert!(report.final_cleared);
    assert!(report.no_replay);
    assert_eq!(report.residents, 0);
    assert_eq!(report.observations.len(), 8);
    for (index, observation) in report.observations.iter().enumerate() {
        assert!(
            observation.pending && observation.charged,
            "tick {index}: {observation:?}"
        );
        assert_eq!(
            observation.before_deadline, observation.before,
            "tick {index}: no early I/O"
        );
        let readback = index % 2 == 0;
        assert_eq!(
            observation.after.loads,
            observation.before.loads + usize::from(readback),
            "tick {index}: {observation:?}"
        );
        assert_eq!(
            observation.after.saves,
            observation.before.saves + usize::from(!readback),
            "tick {index}: {observation:?}"
        );
        assert_eq!(
            observation.cleared,
            index == 7,
            "tick {index}: {observation:?}"
        );
    }
}
