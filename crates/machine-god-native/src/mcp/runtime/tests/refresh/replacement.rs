use super::*;
use futures_util::StreamExt;
use std::{future::Future, sync::atomic::AtomicBool, task::Poll};

fn retained(runtime: &NativeMcpRuntime) -> usize {
    crate::mcp::runtime::refresh::retained_catalog_charge(&mut runtime.state.lock().unwrap())
        .unwrap()
}

fn drain(runtime: &NativeMcpRuntime) {
    block_on(runtime.drain_retired(
        Instant::now() + Duration::from_secs(1),
        CancellationToken::new(),
    ))
    .unwrap();
}

fn replacement(runtime: &NativeMcpRuntime) -> NativeMcpRuntimeCandidate {
    runtime
        .prepare_candidate(vec![addition::server("selected", &["same"])], &[])
        .unwrap()
}

#[test]
fn full_replacement_keeps_binding_charge_and_recovers_after_its_final_owner_drops() {
    let mut runtime = standalone();
    let initial = replacement(&runtime);
    let charge = initial.retained_byte_charge();
    runtime.limits.max_retained_bytes = 2 * charge;
    runtime.publish(initial).unwrap();
    let binding = active(&runtime)
        .tools
        .values()
        .next()
        .unwrap()
        .binding
        .clone();
    runtime.publish(replacement(&runtime)).unwrap();
    assert_eq!(
        retained(&runtime),
        0,
        "undrained peers already cover this charge"
    );
    drain(&runtime);
    assert_eq!(runtime.state.lock().unwrap().retired_byte_charge, 0);
    assert_eq!(retained(&runtime), charge);
    assert!(binding.live().is_err());

    let before = active(&runtime);
    let checkpoint = runtime.publication_checkpoint().unwrap();
    assert_eq!(
        runtime.publish_if(replacement(&runtime), &checkpoint),
        Err(NativeMcpRuntimeError::Limit)
    );
    assert!(Arc::ptr_eq(&before, &active(&runtime)));
    assert!(before.check().is_ok());
    assert!(
        before
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    assert!(runtime.state.lock().unwrap().retired.is_empty());
    assert_eq!(retained(&runtime), charge);

    drop(binding);
    assert_eq!(retained(&runtime), 0);
    runtime
        .publish_if(replacement(&runtime), &checkpoint)
        .unwrap();
}

#[test]
fn drained_old_charge_is_not_suppressed_by_a_new_pending_peer_retirement() {
    let runtime = standalone();
    let initial = replacement(&runtime);
    let charge = initial.retained_byte_charge();
    runtime.publish(initial).unwrap();
    let first_binding = active(&runtime)
        .tools
        .values()
        .next()
        .unwrap()
        .binding
        .clone();
    runtime.publish(replacement(&runtime)).unwrap();
    drain(&runtime);
    assert_eq!(retained(&runtime), charge);

    runtime.publish(replacement(&runtime)).unwrap();
    assert_eq!(runtime.state.lock().unwrap().retired_byte_charge, charge);
    assert_eq!(
        retained(&runtime),
        charge,
        "only the new record is peer-covered"
    );
    assert_eq!(runtime.state.lock().unwrap().retired_catalogs.len(), 2);
    drain(&runtime);
    assert_eq!(
        retained(&runtime),
        charge,
        "the unheld second generation was released"
    );
    assert_eq!(runtime.state.lock().unwrap().retired_catalogs.len(), 1);
    drop(first_binding);
    assert_eq!(retained(&runtime), 0);
}

#[test]
fn full_replacement_observer_count_is_bounded_before_visibility_changes() {
    let runtime = standalone_with_limits(NativeMcpRuntimeLimits {
        max_retired_servers: 1,
        ..Default::default()
    });
    runtime.publish(replacement(&runtime)).unwrap();
    let binding = active(&runtime)
        .tools
        .values()
        .next()
        .unwrap()
        .binding
        .clone();
    runtime.publish(replacement(&runtime)).unwrap();
    drain(&runtime);
    let before = active(&runtime);
    let checkpoint = runtime.publication_checkpoint().unwrap();
    assert_eq!(
        runtime.publish_if(replacement(&runtime), &checkpoint),
        Err(NativeMcpRuntimeError::Limit)
    );
    assert!(Arc::ptr_eq(&before, &active(&runtime)));
    assert!(before.check().is_ok());
    assert!(
        before
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    assert!(runtime.state.lock().unwrap().retired.is_empty());
    assert_eq!(runtime.state.lock().unwrap().retired_catalogs.len(), 1);
    drop(binding);
    runtime
        .publish_if(replacement(&runtime), &checkpoint)
        .unwrap();
}

#[test]
fn full_replacement_tracks_a_server_only_feature_owner_after_peer_drain() {
    let runtime = standalone();
    install(&runtime, &[]);
    let before = active(&runtime);
    let charge = before.retained_bytes;
    let server = before.servers[0].clone();
    assert!(before.tools.is_empty());
    drop(before);
    install(&runtime, &[]);
    drain(&runtime);
    assert!(server.cancellation.is_cancelled());
    assert_eq!(retained(&runtime), charge);
    drop(server);
    assert_eq!(retained(&runtime), 0);
}

struct PausedAfterExchange {
    exchanged: AtomicBool,
}

impl NativeMcpToolExecutor for PausedAfterExchange {
    fn execute(
        &self,
        mut call: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            let response = call.first_exchange().await?;
            drop(response);
            self.exchanged.store(true, Ordering::Release);
            // Same native call custody as awaiting human consent or durable
            // archive publication: the exchange released the peer lane, but
            // the executor future still owns its original call and generation.
            std::future::pending::<()>().await;
            call.revalidate()?;
            Ok(ToolExecution::output(ToolOutput::success(json!({}))))
        })
    }
}

#[test]
fn native_call_suspended_after_exchange_stays_charged_across_reload_and_drain() {
    let executor = Arc::new(PausedAfterExchange {
        exchanged: AtomicBool::new(false),
    });
    let fixture = Fixture::with_executor(
        &[json!({})],
        PermissionMode::Yolo,
        executor.clone(),
        policy(),
        false,
        |runtime, writes| candidate(runtime, "calendar", &["lookup"], writes),
    );
    let runtime = fixture.runtime.clone();
    let before = active(&runtime);
    let charge = before.retained_bytes;
    let server = Arc::downgrade(&before.servers[0]);
    let tool = Arc::downgrade(before.tools.values().next().unwrap());
    drop(before);
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut running = Box::pin(turn.collect::<Vec<_>>());
    block_on(std::future::poll_fn(|cx| {
        // The real proof-bearing writer yields after writing and wakes its
        // caller before the later flush poll. Drive those actual wakeups until
        // first_exchange has returned, not merely until the turn first yields.
        assert!(
            running.as_mut().poll(cx).is_pending(),
            "turn completed before the executor's post-exchange pause"
        );
        if executor.exchanged.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }));
    assert!(executor.exchanged.load(Ordering::Acquire));
    let request = machine_god_core::json::from_slice(&fixture.writes.lock().unwrap()).unwrap();
    assert_eq!(request["method"], "tools/call");
    assert_eq!(request["params"]["name"], "lookup");
    assert!(server.upgrade().unwrap().peer.try_lock().is_some());

    runtime
        .publish(candidate(&runtime, "calendar", &["lookup"], Arc::default()))
        .unwrap();
    drain(&runtime);
    assert!(runtime.state.lock().unwrap().retired.is_empty());
    assert!(server.strong_count() > 0);
    assert!(tool.strong_count() > 0);
    assert_eq!(retained(&runtime), charge);

    drop(running);
    drop(fixture);
    assert_eq!(server.strong_count(), 0);
    assert_eq!(tool.strong_count(), 0);
    assert_eq!(retained(&runtime), 0);
}
