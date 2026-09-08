use super::*;
use machine_god_core::{SessionId, SessionIncarnationId};

fn owner(name: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(name).unwrap(),
        SessionIncarnationId::new(name).unwrap(),
    )
}
fn id(name: &str) -> TerminalSessionId {
    TerminalSessionId::new(name).unwrap()
}

#[test]
fn access_generations_cannot_reactivate_an_old_operation_or_writer() {
    let mut routes = TerminalAccessRoutes::default();
    let (old, writer) = routes.acquire(owner("a")).unwrap();
    routes.retire(&owner("a"));
    assert!(routes.acquire(owner("a")).is_err());
    routes.activate(owner("a")).unwrap();
    let (new, next_writer) = routes.acquire(owner("a")).unwrap();
    assert!(old.is_cancelled());
    assert!(!new.is_cancelled());
    assert_ne!(writer, next_writer);
    routes.activate(owner("a")).unwrap();
    assert!(
        !new.is_cancelled(),
        "activation of an active owner is idempotent"
    );
}

#[test]
fn access_direct_transfers_and_tombstones_never_expose_an_entire_original_catalog() {
    let mut routes = TerminalAccessRoutes::default();
    let route = AccessRoute {
        storage: owner("a"),
        id: id("terminal"),
        current: Some(owner("a")),
    };
    routes.set(route.clone(), Some(owner("b")));
    routes.set(route.clone(), Some(owner("c")));
    assert_eq!(routes.routes.len(), 1);
    assert_eq!(
        routes.resolve(&owner("c"), &id("terminal")).unwrap(),
        owner("a")
    );
    assert!(routes.resolve(&owner("a"), &id("terminal")).is_err());
    assert_eq!(
        routes.resolve(&owner("b"), &id("terminal")).unwrap(),
        owner("b")
    );
    assert!(!routes.visible(&owner("c"), &owner("a"), &id("unrelated")));
    routes.set(route, None);
    assert!(!routes.visible(&owner("a"), &owner("a"), &id("terminal")));
    assert!(!routes.visible(&owner("c"), &owner("a"), &id("terminal")));
}

#[test]
fn access_capacity_and_generation_exhaustion_precede_mutation() {
    let mut routes = TerminalAccessRoutes::default();
    routes.principals.lock().generation = u64::MAX;
    assert_eq!(
        routes.activate(owner("a")),
        Err(NativeTerminalTransitionError::Capacity)
    );
    assert!(routes.principals.lock().principals.is_empty());
    routes.principals.lock().generation = 0;
    for index in 0..MAX_PROFILE_OWNERS {
        routes.activate(owner(&format!("p{index}"))).unwrap();
    }
    assert_eq!(
        routes.acquire(owner("overflow")).unwrap_err(),
        NativeTerminalTransitionError::Capacity
    );
    assert_eq!(
        routes.principals.lock().principals.len(),
        MAX_PROFILE_OWNERS
    );
    routes.routes = (0..MAX_PROFILE_SESSIONS)
        .map(|index| AccessRoute {
            storage: owner("a"),
            id: id(&format!("s{index}")),
            current: Some(owner("b")),
        })
        .collect();
    let selected = vec![AccessRoute {
        storage: owner("a"),
        id: id("overflow"),
        current: Some(owner("b")),
    }];
    assert_eq!(
        routes.preflight(&selected, &[]),
        Err(NativeTerminalTransitionError::Capacity)
    );
    assert_eq!(routes.routes.len(), MAX_PROFILE_SESSIONS);
}

#[test]
fn access_alias_collisions_fail_before_any_route_or_generation_changes() {
    let mut routes = TerminalAccessRoutes::default();
    let route = AccessRoute {
        storage: owner("a"),
        id: id("same"),
        current: Some(owner("b")),
    };
    routes.set(route.clone(), Some(owner("b")));
    let (access, _) = routes.acquire(owner("b")).unwrap();
    assert_eq!(
        routes.collisions(std::slice::from_ref(&route), &owner("c"), &[id("same")]),
        Err(NativeTerminalTransitionError::Conflict)
    );
    assert!(!access.is_cancelled());
    assert!(routes.visible(&owner("b"), &owner("a"), &id("same")));
    let duplicate = AccessRoute {
        storage: owner("other"),
        id: id("same"),
        current: Some(owner("b")),
    };
    assert_eq!(
        routes.collisions(&[route.clone(), duplicate], &owner("c"), &[]),
        Err(NativeTerminalTransitionError::Conflict)
    );
    assert!(
        routes
            .collisions(&[route], &owner("a"), &[id("same")])
            .is_ok(),
        "returning to the identical storage origin is not a collision"
    );
    assert_eq!(routes.routes.len(), 1);
}

#[test]
fn closing_unknown_recovered_history_does_not_prove_process_termination() {
    use crate::terminal_monitor::TerminalProcessOutcome;
    use machine_god_core::TerminalLifecycle;
    assert!(!confirmed_history_outcome(TerminalLifecycle::Closed, None));
    assert!(!confirmed_history_outcome(TerminalLifecycle::Exited, None));
    assert!(!confirmed_history_outcome(
        TerminalLifecycle::Lost,
        Some(TerminalProcessOutcome::Exited(0))
    ));
    assert!(confirmed_history_outcome(
        TerminalLifecycle::Closed,
        Some(TerminalProcessOutcome::Signaled(9))
    ));
    assert!(confirmed_history_outcome(
        TerminalLifecycle::Exited,
        Some(TerminalProcessOutcome::Exited(0))
    ));
}
