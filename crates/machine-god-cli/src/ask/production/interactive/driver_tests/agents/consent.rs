use super::*;

/// Approve ordinary source permission if requested, but retain the second
/// prompt's actual flush acknowledgement. No structural consent is fabricated.
async fn hold_consent(harness: &mut Harness) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            if let Some(modal) = &harness.driver.modal
                && modal.view.permission().is_some()
                && modal.displayed
            {
                let binding = modal.binding();
                harness.driver.line("y", &binding, 101);
                cx.waker().wake_by_ref();
            }
            if let Ok(work) = harness.work.try_recv() {
                if matches!(work, OutputWork::Flush)
                    && harness.driver.modal.as_ref().is_some_and(|modal| {
                        modal.view.execution_consent().is_some()
                            && matches!(&harness.driver.in_flight,
                                Some(InFlight::Flush { confirm: Some(binding), .. })
                                    if binding == &modal.presentation_binding())
                    })
                {
                    return Poll::Ready(());
                }
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }),
    )
    .await
    .unwrap();
}

fn queue_relationship(fixture: &support::Fixture, harness: &Harness) {
    let child = harness.driver.owner.managed_agents()[0].id.clone();
    let parent = harness.driver.owner.runtime().id().to_string();
    fixture.transport.push(support::call(
        "subagent",
        &serde_json::json!({"command":{"relationship":{
            "id":child,"action":"reparent","parent_id":parent
        }}}),
    ));
    fixture.transport.push(support::answer());
}

fn assert_relationship_result(harness: &Harness, approved: bool) {
    let record = harness.driver.owner.runtime().record();
    let output = record
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            machine_god_core::ContentBlock::ToolResult { output, .. } => Some(output),
            _ => None,
        })
        .expect("the actual model call must publish its relationship result");
    assert_eq!(output.content["ok"], approved, "{output:?}");
}

#[test]
fn relationship_consent_requires_exact_flush_and_offers_no_reusable_grants() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    queue_relationship(&fixture, &harness);
    let result = runtime.block_on(async {
        harness
            .driver
            .owner
            .enqueue("request consent".into())
            .unwrap();
        hold_consent(&mut harness).await;
        let modal = harness.driver.modal.as_mut().unwrap();
        let binding = modal.presentation_binding();
        assert!(!modal.displayed);
        assert!(modal.answer("yes", &binding).is_err());
        assert!(!modal.view.can_save_rule());
        let output = String::from_utf8(modal.render().unwrap()).unwrap();
        assert!(output.contains("[consent]"));
        assert!(output.contains("approve this proposal"));
        assert!(output.contains("child_revision"));
        for forbidden in ["[t]", "[s]", "[a]", "[d]", "allow session", "allow turn"] {
            assert!(!output.contains(forbidden));
        }
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        pump_until(&mut harness, |driver| {
            driver.modal.as_ref().is_some_and(|modal| modal.displayed)
        })
        .await;
        let modal = harness.driver.modal.as_mut().unwrap();
        assert!(modal.answer("yes", &InputBinding::Command).is_err());
        for invalid in ["t", "s", "a", "d", "allow_always"] {
            assert!(modal.answer(invalid, &binding).is_err());
        }
        assert!(matches!(
            modal.answer("/cancel", &binding),
            Ok(Some(
                native::NativeInteractivePromptResponse::ExecutionConsent(false)
            ))
        ));
        harness.driver.line("yes", &binding, 102);
        pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.owner.runtime().status().queued_jobs == 0
        })
        .await;
        assert!(harness.driver.modal.is_none());
        assert_eq!(fixture.transport.requests().len(), 2);
        assert!(!harness.driver.native_failed);
        assert_relationship_result(&harness, true);
        queue_relationship(&fixture, &harness);
        harness
            .driver
            .owner
            .enqueue("approve another exact proposal".into())
            .unwrap();
        hold_consent(&mut harness).await;
        assert_eq!(fixture.transport.requests().len(), 3);
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn relationship_consent_rejection_settles_without_a_grant_or_driver_failure() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    queue_relationship(&fixture, &harness);
    let result = runtime.block_on(async {
        harness
            .driver
            .owner
            .enqueue("reject relationship".into())
            .unwrap();
        hold_consent(&mut harness).await;
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        pump_until(&mut harness, |driver| {
            driver.modal.as_ref().is_some_and(|modal| modal.displayed)
        })
        .await;
        let binding = harness.driver.modal.as_ref().unwrap().binding();
        harness.driver.line("no", &binding, 102);
        pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.owner.runtime().status().queued_jobs == 0
        })
        .await;
        assert!(harness.driver.modal.is_none());
        assert!(!harness.driver.native_failed);
        assert_eq!(fixture.transport.requests().len(), 2);
        assert_relationship_result(&harness, false);
        // A new actual call must still request its own exact-proposal consent.
        queue_relationship(&fixture, &harness);
        harness
            .driver
            .owner
            .enqueue("another relationship".into())
            .unwrap();
        hold_consent(&mut harness).await;
        assert!(
            harness
                .driver
                .modal
                .as_ref()
                .unwrap()
                .view
                .execution_consent()
                .is_some()
        );
        // Shutdown while the second proposal is unacknowledged must settle it.
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}
