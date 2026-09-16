use super::*;

fn menu_displayed(driver: &Driver) -> bool {
    displayed(driver) && driver.owner.managed_navigation().unwrap().route == Route::Skills
}

#[test]
fn child_skill_menu_filters_inserts_and_restores_parent_without_inference() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared_with_extensions(None, true));
    let result = runtime.block_on(async {
        harness.driver.command("/skills create review", 100);
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        let snapshot = harness
            .driver
            .owner
            .skills_catalog()
            .unwrap()
            .discover(&CancellationToken::new())
            .unwrap();
        harness.driver.set_skills_snapshot(Some(Arc::new(snapshot)));
        pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.control_outcome.is_none()
        })
        .await;
        harness.input_writer.write_all(b"Help $re").unwrap();
        pump_until(&mut harness, |driver| {
            presentation_idle(driver)
                && matches!(
                    driver.skills_binding(),
                    Some(InputBinding::Skills { frame: Some(_), .. })
                )
        })
        .await;
        harness.input_writer.write_all(b"\t").unwrap();
        pump_until(&mut harness, |driver| {
            driver
                .input
                .raw_draft()
                .is_some_and(|(text, _)| text == "Help $review ")
        })
        .await;
        assert_eq!(harness.driver.bound_skill_count(), 1);
        enter_child(&mut harness).await;
        harness.input_writer.write_all(b"/skills\r").unwrap();
        pump_until(&mut harness, menu_displayed).await;
        let old = harness.driver.owner.managed_navigation().unwrap().frame;
        harness.input_writer.write_all(b"no-match").unwrap();
        pump_until(&mut harness, |driver| {
            menu_displayed(driver)
                && driver
                    .owner
                    .managed_navigation()
                    .unwrap()
                    .skills
                    .unwrap()
                    .total_matches
                    == 0
        })
        .await;
        assert_eq!(
            harness
                .driver
                .owner
                .act_on_managed_frame(&old, native::NativeManagedNavigationAction::Select),
            Err(native::NativeManagedNavigationError::StaleFrame)
        );
        harness.input_writer.write_all(b"\x03").unwrap();
        pump_until(&mut harness, |driver| {
            menu_displayed(driver)
                && driver
                    .owner
                    .managed_navigation()
                    .unwrap()
                    .skills
                    .unwrap()
                    .query
                    .is_empty()
        })
        .await;
        harness.input_writer.write_all(b"\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver.owner.managed_navigation().unwrap().route == Route::Conversation
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft().unwrap().0, "$review ");
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver.agents.is_none() && presentation_idle(driver)
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft().unwrap().0, "Help $review ");
        assert_eq!(harness.driver.bound_skill_count(), 1);
        enter_child(&mut harness).await;
        assert_eq!(harness.driver.input.raw_draft().unwrap().0, "$review ");
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}
