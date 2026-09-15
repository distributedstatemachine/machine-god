use super::*;

fn named_owner(name: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(name).unwrap(),
        owner().session_incarnation_id().clone(),
    )
}

fn owned_request(owner: &BackgroundOutputOwner) -> PermissionRequest {
    let mut request = request("same-request");
    request.session_id = owner.session_id().clone();
    request.session_incarnation_id = owner.session_incarnation_id().clone();
    request
}

fn pending<'a>(
    bridge: &'a NativeInteractivePromptBridge,
    owner: &BackgroundOutputOwner,
) -> BoxFuture<'a, Result<PermissionPromptDecision, PermissionPromptError>> {
    let mut future = PermissionPrompter::prompt(bridge, owned_request(owner));
    assert!(poll(&mut future).is_pending());
    future
}

#[test]
fn fixed_bridge_cannot_route_to_another_registered_owner_or_replacement() {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let mut first = inbox.register(owner()).unwrap();
    let second = inbox.register(named_owner("second")).unwrap();
    let bridge = first.bridge();
    assert!(
        block_on(PermissionPrompter::prompt(
            bridge.as_ref(),
            owned_request(second.owner())
        ))
        .is_err()
    );
    let never = permission(&bridge, "never");
    first.retire();
    let replacement = inbox.register(owner()).unwrap();
    assert_ne!(first.scope(), replacement.scope());
    assert!(block_on(never).is_err());
    assert!(block_on(permission(&bridge, "stale-bridge")).is_err());
    // Retiring the old lease again cannot remove its replacement.
    drop(first);
    let replacement_bridge = replacement.bridge();
    let mut future = pending(&replacement_bridge, replacement.owner());
    let prompt = view(&mut inbox);
    inbox.cancel(prompt.token()).unwrap();
    assert_eq!(
        poll(&mut future),
        Poll::Ready(Ok(PermissionPromptDecision::Deny))
    );
}

#[test]
fn lease_retirement_discards_only_its_queued_displayed_and_ready_responses() {
    for stage in 0..3 {
        let (router, mut inbox, mut parent) = bridge();
        let child = inbox.register(named_owner("child")).unwrap();
        let parent_future = pending(&router, parent.owner());
        let child_future = pending(&router, child.owner());
        let parent_token = if stage > 0 {
            let prompt = view(&mut inbox);
            if stage == 2 {
                respond(&mut inbox, &prompt, PermissionPromptDecision::AllowSession);
            }
            Some(prompt.token().clone())
        } else {
            None
        };
        parent.retire();
        assert!(block_on(parent_future).is_err());
        if let Some(token) = parent_token {
            assert_eq!(
                inbox.select_prompt(&token).unwrap_err(),
                NativeInteractivePromptError::Stale
            );
        }
        let prompt = view(&mut inbox);
        assert_eq!(prompt.token().owner(), child.owner());
        respond(&mut inbox, &prompt, PermissionPromptDecision::AllowOnce);
        assert_eq!(
            block_on(child_future),
            Ok(PermissionPromptDecision::AllowOnce)
        );
    }
}

#[test]
fn unrelated_registration_and_retirement_during_ready_cleanup_preserve_answer() {
    let (router, mut inbox, parent) = bridge();
    let child = inbox.register(named_owner("child")).unwrap();
    let mut future = pending(&router, parent.owner());
    let prompt = view(&mut inbox);
    respond(&mut inbox, &prompt, PermissionPromptDecision::AllowSession);
    let shared = router.shared.clone();
    let key = child.key.clone();
    let (wake, calls) = reentrant_waker(Callback::Drop, move || {
        shared.retire(&key);
    });
    assert_eq!(
        future.as_mut().poll(&mut Context::from_waker(&wake)),
        Poll::Ready(Ok(PermissionPromptDecision::AllowSession))
    );
    assert!(calls.calls() > 0);
}

#[test]
fn registry_is_bounded_reuses_retired_capacity_and_rejects_duplicates() {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let mut principals = Vec::new();
    for index in 0..MAX_NATIVE_INTERACTIVE_PROMPT_PRINCIPALS {
        principals.push(
            inbox
                .register(named_owner(&format!("owner-{index}")))
                .unwrap(),
        );
    }
    assert_eq!(
        inbox.register(named_owner("overflow")).unwrap_err(),
        NativeInteractivePromptError::Busy
    );
    assert_eq!(
        inbox.register(principals[0].owner().clone()).unwrap_err(),
        NativeInteractivePromptError::Busy
    );
    for index in 0..128 {
        drop(principals.pop());
        principals.push(
            inbox
                .register(named_owner(&format!("replacement-{index}")))
                .unwrap(),
        );
    }
    drop(principals);
    let _fresh = inbox.register(owner()).unwrap();
}

#[test]
fn one_count_budget_covers_all_principals_and_ready_answers() {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::new(2, 8192).unwrap())
            .unwrap();
    let router = inbox.router();
    let parent = inbox.register(owner()).unwrap();
    let mut child = inbox.register(named_owner("child")).unwrap();
    let first = pending(&router, parent.owner());
    let second = pending(&router, child.owner());
    let prompt = view(&mut inbox);
    respond(&mut inbox, &prompt, PermissionPromptDecision::AllowOnce);
    assert!(block_on(permission(&router, "overflow")).is_err());
    child.retire();
    assert!(block_on(second).is_err());
    let third = pending(&router, parent.owner());
    assert_eq!(block_on(first), Ok(PermissionPromptDecision::AllowOnce));
    drop(third);
}

#[test]
fn page_is_payload_free_non_authorizing_and_selection_is_exact() {
    let (router, mut inbox, parent) = bridge();
    let child = inbox.register(named_owner("child")).unwrap();
    let first = pending(&router, parent.owner());
    let second = pending(&router, child.owner());
    let mut cx = Context::from_waker(Waker::noop());
    let page = inbox.page(&mut cx, None, 1).unwrap();
    assert_eq!(page.entries().len(), 1);
    let first_token = page.entries()[0].token();
    assert_eq!(first_token.owner(), parent.owner());
    assert_eq!(
        page.entries()[0].kind(),
        NativeInteractivePromptKind::Permission
    );
    assert_eq!(
        inbox.cancel(first_token),
        Err(NativeInteractivePromptError::Stale)
    );
    let next = inbox.page(&mut cx, page.next(), 1).unwrap();
    assert!(next.next().is_none());
    let child_token = next.entries()[0].token();
    assert_eq!(child_token.owner(), child.owner());
    let selected = inbox.select_prompt(child_token).unwrap();
    respond(&mut inbox, &selected, PermissionPromptDecision::AllowOnce);
    assert_eq!(block_on(second), Ok(PermissionPromptDecision::AllowOnce));
    // A vanished cursor retains only its sequence, never renewed authority.
    drop(first);
    assert!(
        inbox
            .page(&mut cx, Some(first_token), 1)
            .unwrap()
            .entries()
            .is_empty()
    );
    assert_eq!(
        inbox.select_prompt(first_token).unwrap_err(),
        NativeInteractivePromptError::Stale
    );
    let mut foreign =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    assert_eq!(
        foreign.page(&mut cx, Some(first_token), 1).unwrap_err(),
        NativeInteractivePromptError::Stale
    );
    for limit in [0, MAX_NATIVE_INTERACTIVE_PROMPTS + 1, usize::MAX] {
        assert_eq!(
            inbox.page(&mut cx, None, limit).unwrap_err(),
            NativeInteractivePromptError::Limit
        );
    }
}

#[test]
fn page_and_retirement_wakers_reenter_without_holding_queue_lock() {
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let (router, mut inbox, mut principal) = bridge();
        let future = pending(&router, principal.owner());
        let shared = router.shared.clone();
        let (wake, calls) = reentrant_waker(callback, move || {
            assert!(shared.state.try_lock().is_ok());
        });
        for _ in 0..2 {
            inbox
                .page(&mut Context::from_waker(&wake), None, 64)
                .unwrap();
        }
        principal.retire();
        assert!(block_on(future).is_err());
        drop(wake);
        assert!(calls.calls() > 0);
    }
}

#[test]
fn dropping_bridge_and_hidden_owner_does_not_retire_other_registration() {
    let (router, mut inbox, parent) = bridge();
    let child = inbox.register(named_owner("child")).unwrap();
    let child_bridge = child.bridge();
    drop(child_bridge);
    drop(parent);
    let future = pending(&router, child.owner());
    let prompt = view(&mut inbox);
    inbox.cancel(prompt.token()).unwrap();
    assert_eq!(block_on(future), Ok(PermissionPromptDecision::Deny));
}

#[test]
fn payload_and_response_budgets_are_shared_and_targeted_retirement_releases_charges() {
    let parent_owner = owner();
    let child_owner = named_owner("childxx"); // Same identity length for exact charging.
    let charge = Payload::Permission {
        request: owned_request(&parent_owner),
        rule: None,
    }
    .bytes(usize::MAX)
    .unwrap();
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::new(2, charge).unwrap())
            .unwrap();
    let router = inbox.router();
    let mut parent = inbox.register(parent_owner).unwrap();
    let child = inbox.register(child_owner).unwrap();
    let first = pending(&router, parent.owner());
    assert!(
        block_on(PermissionPrompter::prompt(
            router.as_ref(),
            owned_request(child.owner())
        ))
        .is_err()
    );
    let prompt = view(&mut inbox);
    respond(&mut inbox, &prompt, PermissionPromptDecision::AllowOnce);
    assert!(
        block_on(PermissionPrompter::prompt(
            router.as_ref(),
            owned_request(child.owner())
        ))
        .is_err()
    );
    parent.retire();
    assert!(block_on(first).is_err());
    let second = pending(&router, child.owner());
    drop(second);

    let limits = NativeInteractivePromptLimits::new(2, charge * 2)
        .unwrap()
        .with_response_bytes(64)
        .unwrap();
    let mut inbox = NativeInteractivePromptInbox::new(limits).unwrap();
    let router = inbox.router();
    let mut parent = inbox.register(owner()).unwrap();
    let child = inbox.register(named_owner("childxx")).unwrap();
    let first = pending(&router, parent.owner());
    let second = pending(&router, child.owner());
    let prompt = view(&mut inbox);
    respond(&mut inbox, &prompt, PermissionPromptDecision::AllowSession);
    let child_prompt = view(&mut inbox);
    assert_eq!(
        inbox.cancel(child_prompt.token()),
        Err(NativeInteractivePromptError::Limit)
    );
    parent.retire();
    assert!(block_on(first).is_err());
    inbox.cancel(child_prompt.token()).unwrap();
    assert_eq!(block_on(second), Ok(PermissionPromptDecision::Deny));
}

#[test]
fn full_domain_projects_with_bounded_rows_without_retaining_payloads() {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let router = inbox.router();
    let principals = (0..64)
        .map(|index| {
            inbox
                .register(named_owner(&format!("{index:0128}")))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let futures = principals
        .iter()
        .map(|principal| pending(&router, principal.owner()))
        .collect::<Vec<_>>();
    let page = inbox
        .page(&mut Context::from_waker(Waker::noop()), None, 64)
        .unwrap();
    assert_eq!(page.entries().len(), 64);
    assert!(page.next().is_none());
    assert!(
        block_on(PermissionPrompter::prompt(
            router.as_ref(),
            owned_request(principals[0].owner())
        ))
        .is_err()
    );
    drop(futures);
    assert!(
        inbox
            .page(&mut Context::from_waker(Waker::noop()), None, 64)
            .unwrap()
            .entries()
            .is_empty()
    );
    for row in page.entries() {
        assert_eq!(
            inbox.select_prompt(row.token()).unwrap_err(),
            NativeInteractivePromptError::Stale
        );
    }
}
