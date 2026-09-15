use super::*;
use futures_executor::block_on;
use futures_util::{future::poll_fn, task::noop_waker_ref};
use machine_god_core::{Engine, Session, SessionId, SessionIncarnationId};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, Waker},
};

struct ActualTurn {
    _engine: Engine,
    _session: Session,
    turn: Option<Turn>,
}
impl ActualTurn {
    fn new(name: &str) -> Self {
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("scheduler", []))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .session_store(InMemorySessionStore::default())
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new(name).unwrap(),
                SessionIncarnationId::new("generation").unwrap(),
            )
            .unwrap();
        let turn = block_on(session.prompt("never polled by scheduler")).unwrap();
        Self {
            _engine: engine,
            _session: session,
            turn: Some(turn),
        }
    }
    fn turn(&self) -> &Turn {
        self.turn.as_ref().unwrap()
    }
}
struct Node {
    actual: ActualTurn,
    resident: ResidentLease,
    run: Option<RunLease>,
    settlement: Option<RunSettlement>,
}
impl Node {
    fn new(scheduler: &ManagedScheduler, name: &str) -> Self {
        let actual = ActualTurn::new(name);
        let resident = scheduler.reserve_resident().unwrap();
        let (run, settlement) = scheduler
            .register_run(&resident, NonZeroU64::new(1).unwrap(), actual.turn())
            .unwrap();
        Self {
            actual,
            resident,
            run: Some(run),
            settlement: Some(settlement),
        }
    }
    fn run(&self) -> &RunLease {
        self.run.as_ref().unwrap()
    }
    fn reference(&self) -> RunRef {
        self.run().reference()
    }
    fn finish(&mut self) {
        self.run.take().unwrap().finish();
    }
    fn complete(&mut self) {
        self.actual.turn.take();
        self.settlement.take().unwrap().complete().unwrap();
    }
}
fn scheduler(executions: usize, residents: usize, waiters: usize) -> ManagedScheduler {
    ManagedScheduler::new(SchedulerLimits::new(executions, residents, waiters).unwrap())
}
fn poll<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    std::pin::Pin::new(future).poll(&mut Context::from_waker(noop_waker_ref()))
}
#[derive(Default)]
struct Signal {
    ready: AtomicBool,
    waker: Mutex<Option<Waker>>,
}
impl Signal {
    fn set(&self) {
        self.ready.store(true, Ordering::Release);
        let waker = self.waker.lock().unwrap().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    fn poll(&self, cx: &mut Context<'_>) -> Poll<()> {
        let waker = cx.waker().clone();
        let old = self.waker.lock().unwrap().replace(waker);
        drop(old);
        if self.ready.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
    async fn observe(self: Arc<Self>) {
        poll_fn(|cx| self.poll(cx)).await;
    }
}

#[test]
fn limits_are_validated_and_residency_is_not_a_lifetime_cap() {
    assert_eq!(
        SchedulerLimits::default(),
        SchedulerLimits::new(4, 64, 64).unwrap()
    );
    for limits in [
        (0, 1, 1),
        (1, 0, 1),
        (1, 1, 0),
        (2, 1, 1),
        (257, 4096, 4096),
        (1, 4097, 1),
        (1, 1, 4097),
    ] {
        assert_eq!(
            SchedulerLimits::new(limits.0, limits.1, limits.2),
            Err(SchedulerError::InvalidLimits)
        );
    }
    let scheduler = scheduler(1, 1, 1);
    for _ in 0..128 {
        let resident = scheduler.reserve_resident().unwrap();
        assert!(matches!(
            scheduler.reserve_resident(),
            Err(SchedulerError::Capacity)
        ));
        drop(resident);
    }
    assert_eq!(scheduler.snapshot().residents, 0);
}
#[test]
fn acquisition_is_inert_fifo_and_ordinary_pending_retains_quota() {
    let scheduler = scheduler(1, 3, 3);
    let mut a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    let c = Node::new(&scheduler, "c");
    let mut a_acquire = a.run().acquire();
    assert_eq!(scheduler.snapshot().executing, 0);
    assert_eq!(poll(&mut a_acquire), Poll::Ready(Ok(())));
    let mut b_acquire = b.run().acquire();
    let mut c_acquire = c.run().acquire();
    assert_eq!(poll(&mut b_acquire), Poll::Pending);
    assert_eq!(poll(&mut c_acquire), Poll::Pending);
    for _ in 0..10 {
        assert_eq!(poll(&mut b_acquire), Poll::Pending);
    }
    // Nothing polls or recognizes generic provider Pending as quiescence.
    assert_eq!(scheduler.snapshot().executing, 1);
    assert_eq!(scheduler.snapshot().waiters, 2);
    a.finish();
    assert!(b.reference().is_executing()); // reserved grant, before observing it
    assert!(!c.reference().is_executing());
    assert_eq!(poll(&mut b_acquire), Poll::Ready(Ok(())));
    drop(c_acquire);
    assert!(c.actual.turn().handle().is_cancelled());
    assert_eq!(scheduler.snapshot().waiters, 0);
}
#[test]
fn one_slot_handoff_a_to_b_then_back_to_a() {
    let scheduler = scheduler(1, 2, 2);
    let a = Node::new(&scheduler, "a");
    let mut b = Node::new(&scheduler, "b");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let mut b_acquire = b.run().acquire();
    assert_eq!(poll(&mut b_acquire), Poll::Pending);
    let signal = Arc::new(Signal::default());
    let mut wait = a.reference().dependency_wait(
        b.reference(),
        signal.clone().observe(),
        CancellationToken::new(),
    );
    assert_eq!(scheduler.snapshot().dependencies, 0);
    assert_eq!(poll(&mut wait), Poll::Pending);
    assert!(!a.reference().is_executing());
    assert_eq!(poll(&mut b_acquire), Poll::Ready(Ok(())));
    assert_eq!(scheduler.snapshot().dependencies, 1);
    b.finish();
    b.complete();
    signal.set();
    assert_eq!(poll(&mut wait), Poll::Ready(Ok(())));
    assert!(a.reference().is_executing());
    assert_eq!(scheduler.snapshot().executing, 1);
    assert_eq!(scheduler.snapshot().waiters, 0);
    assert_eq!(scheduler.snapshot().dependencies, 0);
}
#[test]
fn cycles_self_and_unschedulable_targets_do_not_release_the_caller() {
    let scheduler = scheduler(3, 4, 4);
    let a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    let c = Node::new(&scheduler, "c");
    let never_queued = Node::new(&scheduler, "never");
    for node in [&a, &b, &c] {
        assert_eq!(poll(&mut node.run().acquire()), Poll::Ready(Ok(())));
    }
    let mut self_wait = a.reference().dependency_wait(
        a.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    assert_eq!(
        poll(&mut self_wait),
        Poll::Ready(Err(SchedulerError::DependencyCycle))
    );
    let mut never = a.reference().dependency_wait(
        never_queued.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    assert_eq!(
        poll(&mut never),
        Poll::Ready(Err(SchedulerError::Unschedulable))
    );
    assert_eq!(scheduler.snapshot().executing, 3);
    let mut ab = a.reference().dependency_wait(
        b.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    let mut bc = b.reference().dependency_wait(
        c.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    let mut ca = c.reference().dependency_wait(
        a.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    assert_eq!(poll(&mut ab), Poll::Pending);
    assert_eq!(poll(&mut bc), Poll::Pending);
    assert_eq!(
        poll(&mut ca),
        Poll::Ready(Err(SchedulerError::DependencyCycle))
    );
    assert!(c.reference().is_executing());
    assert_eq!(scheduler.snapshot().dependencies, 2);
    drop(ab);
    drop(bc);
    assert_eq!(scheduler.snapshot().waiters, 0);
}
#[test]
fn errors_and_timeouts_reacquire_fairly_before_publication() {
    for output in [Ok("settled"), Err("timeout"), Err("observer failed")] {
        let scheduler = scheduler(1, 3, 3);
        let a = Node::new(&scheduler, "a");
        let b = Node::new(&scheduler, "b");
        let c = Node::new(&scheduler, "c");
        assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
        let mut b_acquire = b.run().acquire();
        let mut c_acquire = c.run().acquire();
        assert_eq!(poll(&mut b_acquire), Poll::Pending);
        assert_eq!(poll(&mut c_acquire), Poll::Pending);
        let mut wait = a.reference().dependency_wait(
            b.reference(),
            std::future::ready(output),
            CancellationToken::new(),
        );
        assert_eq!(poll(&mut wait), Poll::Pending); // output is private, B owns grant
        assert_eq!(scheduler.snapshot().dependencies, 0);
        assert_eq!(scheduler.snapshot().queued, 2); // C before A reacquisition
        b.run().cancel();
        assert!(c.reference().is_executing());
        assert_eq!(poll(&mut wait), Poll::Pending);
        c.run().cancel();
        assert_eq!(poll(&mut wait), Poll::Ready(Ok(output)));
        assert!(a.reference().is_executing());
    }
}
#[test]
fn successive_dependency_tools_each_resume_under_execution_quota() {
    let scheduler = scheduler(1, 3, 3);
    let a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    let c = Node::new(&scheduler, "c");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    for target in [&b, &c] {
        let mut acquisition = target.run().acquire();
        assert_eq!(poll(&mut acquisition), Poll::Pending);
        let mut wait = a.reference().dependency_wait(
            target.reference(),
            std::future::ready(1),
            CancellationToken::new(),
        );
        assert_eq!(poll(&mut wait), Poll::Pending);
        assert_eq!(poll(&mut acquisition), Poll::Ready(Ok(())));
        target.run().cancel();
        assert_eq!(poll(&mut wait), Poll::Ready(Ok(1)));
        assert!(a.reference().is_executing());
        assert_eq!(scheduler.snapshot().executing, 1);
    }
}
#[test]
fn dropped_wait_and_same_poll_cancel_never_resume_quota_free() {
    for cancel_in_poll in [false, true] {
        let scheduler = scheduler(1, 2, 2);
        let a = Node::new(&scheduler, "a");
        let b = Node::new(&scheduler, "b");
        assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
        let mut b_acquire = b.run().acquire();
        assert_eq!(poll(&mut b_acquire), Poll::Pending);
        let cancellation = CancellationToken::new();
        let trigger = cancellation.clone();
        let observed = async move {
            if cancel_in_poll {
                trigger.cancel();
            }
            if !cancel_in_poll {
                std::future::pending::<()>().await;
            }
            "not publishable"
        };
        let mut wait = a
            .reference()
            .dependency_wait(b.reference(), observed, cancellation);
        if cancel_in_poll {
            assert_eq!(poll(&mut wait), Poll::Ready(Err(SchedulerError::Cancelled)));
        } else {
            assert_eq!(poll(&mut wait), Poll::Pending);
        }
        drop(wait);
        assert!(a.actual.turn().handle().is_cancelled());
        assert!(!a.reference().is_executing());
        assert_eq!(scheduler.snapshot().dependencies, 0);
        assert_eq!(scheduler.snapshot().waiters, 0);
        assert!(b.reference().is_executing());
        assert_eq!(scheduler.snapshot().settling, 1);
    }
}
#[test]
fn canceled_grant_drop_refunds_exact_slot_and_does_not_bypass_queue() {
    let scheduler = scheduler(1, 3, 3);
    let a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    let c = Node::new(&scheduler, "c");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let mut b_acquire = b.run().acquire();
    let mut c_acquire = c.run().acquire();
    assert_eq!(poll(&mut b_acquire), Poll::Pending);
    assert_eq!(poll(&mut c_acquire), Poll::Pending);
    a.run().cancel();
    assert!(b.reference().is_executing());
    drop(b_acquire); // grant was reserved but never observed
    assert!(b.actual.turn().handle().is_cancelled());
    assert!(c.reference().is_executing());
    assert_eq!(poll(&mut c_acquire), Poll::Ready(Ok(())));
    assert_eq!(scheduler.snapshot().executing, 1);
}
#[test]
fn waiter_capacity_is_reserved_before_releasing_execution() {
    let scheduler = scheduler(1, 2, 1);
    let a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let mut b_acquire = b.run().acquire();
    assert_eq!(poll(&mut b_acquire), Poll::Pending);
    let mut wait = a.reference().dependency_wait(
        b.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    assert_eq!(poll(&mut wait), Poll::Ready(Err(SchedulerError::Capacity)));
    assert!(a.reference().is_executing());
    assert_eq!(scheduler.snapshot().executing, 1);
}
#[test]
fn residency_waits_for_actual_settlement_and_abandonment_quarantines() {
    let scheduler = scheduler(1, 1, 1);
    let mut a = Node::new(&scheduler, "a");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let old = a.reference();
    a.run.take(); // cancels but does not release residency
    drop(a.resident);
    assert_eq!(scheduler.snapshot().executing, 0);
    assert_eq!(scheduler.snapshot().settling, 1);
    assert!(matches!(
        scheduler.reserve_resident(),
        Err(SchedulerError::Capacity)
    ));
    a.actual.turn.take();
    a.settlement.take().unwrap().complete().unwrap();
    assert_eq!(scheduler.snapshot().residents, 0);
    assert_eq!(
        poll(&mut old.acquire()),
        Poll::Ready(Err(SchedulerError::Stale))
    );
    let mut next = Node::new(&scheduler, "next");
    next.run.take();
    next.settlement.take(); // loses proof of actual completion: bounded quarantine
    drop(next.resident);
    assert_eq!(scheduler.snapshot().settling, 1);
    assert!(matches!(
        scheduler.reserve_resident(),
        Err(SchedulerError::Capacity)
    ));
}
#[test]
fn actual_work_generation_and_foreign_allocations_cannot_be_replaced() {
    let scheduler = scheduler(1, 2, 2);
    let other = super::ManagedScheduler::new(SchedulerLimits::default());
    let mut a = Node::new(&scheduler, "same-ids");
    let b = Node::new(&other, "same-ids");
    assert_eq!(a.reference().work_generation(), NonZeroU64::new(1));
    assert!(a.reference().matches_turn(&a.actual.turn().witness()));
    assert!(!a.reference().matches_turn(&b.actual.turn().witness()));
    assert!(matches!(
        other.register_run(&a.resident, NonZeroU64::new(2).unwrap(), b.actual.turn()),
        Err(SchedulerError::Foreign)
    ));
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    assert_eq!(poll(&mut b.run().acquire()), Poll::Ready(Ok(())));
    let mut foreign = a.reference().dependency_wait(
        b.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    assert_eq!(
        poll(&mut foreign),
        Poll::Ready(Err(SchedulerError::Foreign))
    );
    let old = a.reference();
    a.finish();
    a.complete();
    let next = ActualTurn::new("same-ids");
    assert!(matches!(
        scheduler.register_run(&a.resident, NonZeroU64::new(1).unwrap(), next.turn()),
        Err(SchedulerError::Stale)
    ));
    let (new_run, _settlement) = scheduler
        .register_run(&a.resident, NonZeroU64::new(2).unwrap(), next.turn())
        .unwrap();
    assert_eq!(
        poll(&mut old.acquire()),
        Poll::Ready(Err(SchedulerError::Stale))
    );
    assert_eq!(poll(&mut new_run.acquire()), Poll::Ready(Ok(())));
}
#[test]
fn waker_clone_drop_wake_and_cancellation_reenter_outside_registry_lock() {
    use machine_god_reentrant_waker_test::{Callback, new};
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let scheduler = scheduler(1, 2, 2);
        let a = Node::new(&scheduler, "a");
        let b = Node::new(&scheduler, "b");
        assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
        let observer = scheduler.clone();
        let (waker, handle) = new(callback, move || {
            let _ = observer.snapshot();
        });
        let mut acquisition = b.run().acquire();
        assert_eq!(
            std::pin::Pin::new(&mut acquisition).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        assert_eq!(poll(&mut acquisition), Poll::Pending); // discard old waker outside lock
        assert_eq!(
            std::pin::Pin::new(&mut acquisition).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        a.run().cancel();
        assert_eq!(poll(&mut acquisition), Poll::Ready(Ok(())));
        b.run().cancel();
        drop(waker);
        assert!(handle.calls() > 0);
    }
}

#[test]
fn actual_turn_cannot_be_registered_twice() {
    let scheduler = scheduler(1, 2, 2);
    let a = Node::new(&scheduler, "a");
    let resident = scheduler.reserve_resident().unwrap();
    assert!(matches!(
        scheduler.register_run(&resident, NonZeroU64::new(1).unwrap(), a.actual.turn()),
        Err(SchedulerError::Busy)
    ));
}

#[test]
fn settlement_progresses_with_all_execution_slots_occupied() {
    let scheduler = scheduler(1, 2, 2);
    let mut a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let mut acquisition = b.run().acquire();
    assert_eq!(poll(&mut acquisition), Poll::Pending);
    a.finish();
    assert_eq!(poll(&mut acquisition), Poll::Ready(Ok(())));
    assert_eq!(scheduler.snapshot().executing, 1);
    a.complete();
    assert_eq!(scheduler.snapshot().settling, 0);
    assert!(b.reference().is_executing());
}

#[test]
fn actual_turn_cancellation_refunds_wait_but_unpolled_drop_is_inert() {
    let scheduler = scheduler(1, 2, 2);
    let a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let mut acquisition = b.run().acquire();
    assert_eq!(poll(&mut acquisition), Poll::Pending);
    let make_wait = || {
        a.reference().dependency_wait(
            b.reference(),
            std::future::pending::<()>(),
            CancellationToken::new(),
        )
    };
    drop(make_wait());
    assert!(a.reference().is_executing());
    assert_eq!(scheduler.snapshot().dependencies, 0);
    let mut wait = make_wait();
    assert_eq!(poll(&mut wait), Poll::Pending);
    let _ = a.actual.turn().handle().cancel();
    assert_eq!(poll(&mut wait), Poll::Ready(Err(SchedulerError::Cancelled)));
    assert_eq!(scheduler.snapshot().waiters, 0);
    assert_eq!(scheduler.snapshot().dependencies, 0);
    assert!(b.reference().is_executing());
}

#[test]
fn stale_wait_drop_cannot_cancel_the_next_actual_generation() {
    let scheduler = scheduler(1, 2, 2);
    let mut a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let mut acquisition = b.run().acquire();
    assert_eq!(poll(&mut acquisition), Poll::Pending);
    let mut wait = a.reference().dependency_wait(
        b.reference(),
        std::future::pending::<()>(),
        CancellationToken::new(),
    );
    assert_eq!(poll(&mut wait), Poll::Pending);
    a.run.take();
    a.complete();
    b.run().cancel();
    let next = ActualTurn::new("a");
    let (run, _settlement) = scheduler
        .register_run(&a.resident, NonZeroU64::new(2).unwrap(), next.turn())
        .unwrap();
    assert_eq!(poll(&mut run.acquire()), Poll::Ready(Ok(())));
    drop(wait);
    assert!(!next.turn().handle().is_cancelled());
    assert!(run.reference().is_executing());
}

#[test]
fn granting_target_can_cancel_caller_before_observation_is_polled() {
    use machine_god_reentrant_waker_test::{Callback, new};
    let scheduler = scheduler(1, 2, 2);
    let a = Node::new(&scheduler, "a");
    let b = Node::new(&scheduler, "b");
    assert_eq!(poll(&mut a.run().acquire()), Poll::Ready(Ok(())));
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let (waker, _) = new(Callback::Wake, move || {
        trigger.cancel();
    });
    let mut acquisition = b.run().acquire();
    assert_eq!(
        std::pin::Pin::new(&mut acquisition).poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    );
    let observed = Arc::new(AtomicBool::new(false));
    let observation = observed.clone();
    let mut wait = a.reference().dependency_wait(
        b.reference(),
        async move {
            observation.store(true, Ordering::Release);
        },
        cancellation,
    );
    assert_eq!(poll(&mut wait), Poll::Ready(Err(SchedulerError::Cancelled)));
    assert!(!observed.load(Ordering::Acquire));
    assert!(a.actual.turn().handle().is_cancelled());
    assert!(b.reference().is_executing());
    assert_eq!(scheduler.snapshot().waiters, 0);
}
