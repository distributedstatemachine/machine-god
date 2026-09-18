use super::*;
use crate::PermissionMode;
use machine_god_core::{ManagedConfiguration, ManagedPermissionMode};
use serde_json::json;

fn interrupted(f: &mut Fixture, mode: PermissionMode) -> ManagedConfiguration {
    assert!(
        f.command_with_mode(
            json!({"create": {"name": "worker", "mode": "persistent",
                "prompt": "first", "model": "frozen-model", "effort": "low"}}),
            mode,
        )
        .ok
    );
    f.drive(|f| f.factory.provider.requests().len() == 1);
    assert!(
        f.command_with_mode(
            json!({"message": {"send": {"id": "child-1", "content": "frozen"}}}),
            mode,
        )
        .ok
    );
    assert!(
        f.command(json!({"lifecycle": {"id": "child-1", "action": "cancel"}}))
            .ok
    );
    f.drive(|f| f.manager.active.is_none());
    let head = &f.manager.children[0].snapshot.head;
    assert_eq!(head.queue[0].status, ManagedQueueStatus::Interrupted);
    block_on(f.journal.read_work(head.queue[0].page.clone()))
        .unwrap()
        .configuration
}

#[test]
fn resume_uses_frozen_configuration_with_resident_nonresident_and_restarted_owner() {
    for restart in 0..3 {
        let mut f = Fixture::new(vec![ModelProviderStep::pending(), completed(), completed()]);
        let frozen = interrupted(&mut f, PermissionMode::Ask);
        assert!(
            f.command_with_mode(
                json!({"configure": {"id": "child-1",
            "name": "future-worker", "model": "future-model", "effort": "high",
            "permission_mode": "yolo"}}),
                PermissionMode::Yolo
            )
            .ok
        );
        f.drive(|f| f.manager.active.is_none());
        let current = block_on(f.journal.inspect("child-1".into()))
            .unwrap()
            .head
            .configuration;
        match restart {
            1 => f.restart_manager(),
            2 => f.restart_journal_owner(),
            _ => {}
        }
        f.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
        assert_eq!(
            f.factory.provider.requests().len(),
            1,
            "restart must not replay work"
        );
        let preparations = f.factory.prepared.load(Ordering::Relaxed);
        let result = f.command(json!({"lifecycle": {"id": "child-1", "action": "resume"}}));
        assert!(result.ok, "{result:?}");
        if restart != 0 {
            assert_eq!(f.factory.prepared.load(Ordering::Relaxed), preparations + 1);
            assert_eq!(
                f.factory.prepared_configurations.lock().unwrap().last(),
                Some(&(frozen.clone(), Some(PermissionMode::Ask)))
            );
        } else {
            assert_eq!(f.factory.prepared.load(Ordering::Relaxed), preparations);
        }
        f.drive(|f| f.factory.provider.requests().len() == 2 && !f.manager.children[0].busy());
        f.drive(|f| f.manager.active.is_none());
        assert_eq!(
            block_on(f.journal.inspect("child-1".into()))
                .unwrap()
                .head
                .configuration,
            current
        );
        let requests = f.factory.provider.requests();
        assert_eq!(
            requests[1].request.options.model.as_deref(),
            frozen.model.as_deref()
        );
        assert_eq!(frozen.permission_mode, ManagedPermissionMode::Ask);
        f.restart_manager();
        let future_message = json!({"message": {"send": {"id": "child-1", "content": "future"}}});
        assert_eq!(
            f.command(future_message.clone()).error_code,
            Some(ManagedFailureCode::PermissionDenied)
        );
        assert!(
            f.command_with_mode(future_message, PermissionMode::Yolo,)
                .ok
        );
        f.drive(|f| f.factory.provider.requests().len() == 3 && !f.manager.children[0].busy());
        assert_eq!(
            f.factory.prepared_configurations.lock().unwrap().last(),
            Some(&(current.clone(), Some(PermissionMode::Yolo)))
        );
        assert_eq!(
            f.factory.provider.requests()[2]
                .request
                .options
                .model
                .as_deref(),
            current.model.as_deref()
        );
    }
}

#[test]
fn resume_rejects_frozen_policy_escalation_even_after_current_policy_is_restricted() {
    for restart in [false, true] {
        let mut f = Fixture::new(vec![ModelProviderStep::pending()]);
        let frozen = interrupted(&mut f, PermissionMode::Yolo);
        assert_eq!(frozen.permission_mode, ManagedPermissionMode::Yolo);
        assert!(
            f.command(json!({"configure": {"id": "child-1", "permission_mode": "ask"}}))
                .ok
        );
        if restart {
            f.restart_manager();
        }
        let preparations = f.factory.prepared.load(Ordering::Relaxed);
        let result = f.command(json!({"lifecycle": {"id": "child-1", "action": "resume"}}));
        assert_eq!(
            result.error_code,
            Some(ManagedFailureCode::PermissionDenied)
        );
        assert_eq!(f.factory.prepared.load(Ordering::Relaxed), preparations);
        assert_eq!(f.factory.provider.requests().len(), 1);
        f.drive(|f| f.manager.active.is_none());
        let snapshot = block_on(f.journal.inspect("child-1".into())).unwrap();
        assert_eq!(
            snapshot.head.queue[0].status,
            ManagedQueueStatus::Interrupted
        );
        assert_eq!(
            snapshot.head.configuration.permission_mode,
            ManagedPermissionMode::Ask
        );
    }
}
