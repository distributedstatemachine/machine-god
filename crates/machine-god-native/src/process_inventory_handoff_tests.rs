use super::*;
use crate::NativeOwnedWorkerScope;

#[test]
fn inventory_handoff_keeps_exact_child_host_and_independent_failed_cleanup() {
    let host = NativeOwnedWorkerScope::new();
    let run = host.begin_run().unwrap();
    let (child, failed) = futures_executor::block_on(run.with_poll(|| {
        host.run(|| {
            let child = InventoryChild::inert_for_handoff_test();
            let failed = NativeOwnedWorkerScope::retain_current_cleanup().unwrap();
            (child, failed)
        })
    }))
    .unwrap();
    let handoff = LeaseHandoff::new(child.service_handoff());
    run.close();
    handoff.query_completed(true, child.service_handoff());
    assert!(
        !run.completion().is_complete(),
        "unretained query cannot hand off startup"
    );
    handoff.promote();
    assert!(
        !run.completion().is_complete(),
        "independent failure still owns cleanup"
    );
    drop(failed);
    run.completion().wait_on_worker().unwrap();
    host.close();
    assert!(!host.completion().is_complete());
    drop(child);
    host.completion().wait_on_worker().unwrap();
}

#[test]
fn stale_inventory_receipt_never_promotes_replacement_or_failed_query() {
    let host = NativeOwnedWorkerScope::new();
    let original = host.begin_run().unwrap();
    let old_child = futures_executor::block_on(
        original.with_poll(|| host.run(InventoryChild::inert_for_handoff_test)),
    )
    .unwrap();
    let handoff = LeaseHandoff::new(old_child.service_handoff());
    original.close();
    drop(old_child);
    original.completion().wait_on_worker().unwrap();
    let replacement = host.begin_run().unwrap();
    let new_child = futures_executor::block_on(
        replacement.with_poll(|| host.run(InventoryChild::inert_for_handoff_test)),
    )
    .unwrap();
    replacement.close();
    handoff.promote();
    assert!(
        !replacement.completion().is_complete(),
        "old receipt cannot retarget a new child"
    );
    handoff.query_completed(false, new_child.service_handoff());
    assert!(
        !replacement.completion().is_complete(),
        "failed query keeps replacement cleanup"
    );
    handoff.query_completed(true, new_child.service_handoff());
    replacement.completion().wait_on_worker().unwrap();
    host.close();
    assert!(!host.completion().is_complete());
    drop(new_child);
    host.completion().wait_on_worker().unwrap();
}
