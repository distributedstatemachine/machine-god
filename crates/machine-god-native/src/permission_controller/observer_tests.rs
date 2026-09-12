use super::*;

fn proof(
    f: &Fixture,
    registration: &NativePermissionTurn,
    bypass: bool,
) -> NativePermissionExecutionProof {
    NativePermissionExecutionProof {
        owner: Arc::downgrade(&f.owner),
        attempt: Arc::downgrade(&registration.attempt),
        epoch: (!bypass).then_some(1),
        rules_epoch: (!bypass).then_some(1),
        saved: None,
        saved_generation: None,
    }
}
struct Counter(AtomicUsize);
impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn reset_wakes_exact_proof_observer_without_changing_yolo_semantics() {
    for bypass in [false, true] {
        let f = Fixture::new();
        let turn = block_on(f.owner.session.prompt("observe")).unwrap();
        let registration = f
            .owner
            .begin_turn(&turn, f.owner.snapshot().unwrap())
            .unwrap();
        let proof = proof(&f, &registration, bypass);
        let mut observer = proof.invalidated();
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        assert!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        f.owner.reset().unwrap();
        assert!(counter.0.load(Ordering::SeqCst) > 0);
        assert_eq!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending(),
            bypass
        );
        if bypass {
            proof.revalidate().unwrap();
            drop(registration);
            assert!(
                observer
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_ready()
            );
        }
    }
}

#[test]
fn reset_before_first_poll_and_rule_publication_are_not_lost() {
    let f = Fixture::new();
    let turn = block_on(f.owner.session.prompt("observe")).unwrap();
    let registration = f
        .owner
        .begin_turn(&turn, f.owner.snapshot().unwrap())
        .unwrap();
    let proof = proof(&f, &registration, false);
    let mut observer = proof.invalidated();
    f.owner.reset().unwrap();
    assert!(
        observer
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );

    let f = Fixture::new();
    let turn = block_on(f.owner.session.prompt("observe")).unwrap();
    let registration = f
        .owner
        .begin_turn(&turn, f.owner.snapshot().unwrap())
        .unwrap();
    let proof = self::proof(&f, &registration, false);
    let mut observer = proof.invalidated();
    assert!(
        observer
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    f.store.pending.store(true, Ordering::SeqCst);
    let mut save = f.owner.confirm_rule_change(f.proposal());
    assert!(
        save.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(
        observer
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    drop(save);
    assert!(proof.revalidate().is_err());
}

#[test]
fn reset_notification_reenters_without_holding_permission_state_locks() {
    use machine_god_reentrant_waker_test::{Callback, new};
    for callback in [Callback::Clone, Callback::Wake, Callback::Drop] {
        let f = Fixture::new();
        let turn = block_on(f.owner.session.prompt("observe")).unwrap();
        let registration = f
            .owner
            .begin_turn(&turn, f.owner.snapshot().unwrap())
            .unwrap();
        let proof = proof(&f, &registration, false);
        let mut observer = proof.invalidated();
        let state = f.owner.state.clone();
        let (waker, calls) = new(callback, move || {
            assert!(state.try_lock().is_ok());
        });
        assert!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        f.owner.reset().unwrap();
        assert!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
        drop(observer);
        drop(waker);
        assert!(calls.calls() > 0);
    }
}

#[test]
fn panicking_rule_notification_does_not_leave_an_unarmed_operation_active() {
    use machine_god_reentrant_waker_test::{Callback, new};
    let f = Fixture::new();
    let turn = block_on(f.owner.session.prompt("observe")).unwrap();
    let registration = f
        .owner
        .begin_turn(&turn, f.owner.snapshot().unwrap())
        .unwrap();
    let proof = proof(&f, &registration, false);
    let mut observer = proof.invalidated();
    let (waker, _) = new(Callback::Wake, || panic!("fixture wake panic"));
    assert!(
        observer
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    let mut save = f.owner.confirm_rule_change(f.proposal());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = save.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        }))
        .is_err()
    );
    assert!(!lock(&f.owner.state).changing_rules);
    assert!(!lock(&f.owner.state).uncertain_rules);
    proof.revalidate().unwrap();
}
