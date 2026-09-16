use super::human::{close, command, create, open, response, submit};
use super::*;
use crate::{
    NativeManagedCatalogCursor, NativeManagedCatalogError as CatalogError,
    NativeManagedCatalogFilter as Filter, NativeManagedCatalogOutcome, NativeManagedCatalogPage,
    NativeManagedCatalogRequest,
};
use machine_god_core::{ManagedFailureCode, ManagedResultStatus, ManagedSubagentError};

async fn catalog_outcome(owner: &mut NativeInteractiveSession) -> NativeManagedCatalogOutcome {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 10);
        assert!(
            owner.managed_error().is_none(),
            "{:?}",
            owner.managed_error()
        );
        assert!(
            owner.shutdown_error().is_none(),
            "{:?}",
            owner.shutdown_error()
        );
        if let Some(outcome) = owner.take_managed_catalog_outcome() {
            return Poll::Ready(outcome);
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

pub(super) async fn page(
    owner: &mut NativeInteractiveSession,
    filter: Filter,
    cursor: Option<NativeManagedCatalogCursor>,
    limit: usize,
) -> NativeManagedCatalogPage {
    let request = owner
        .request_managed_catalog(filter, cursor, limit)
        .unwrap();
    let outcome = catalog_outcome(owner).await;
    assert_eq!(outcome.request, request);
    outcome.result.unwrap()
}

fn inspect(id: &str) -> machine_god_core::ManagedSubagentCommand {
    command(serde_json::json!({"inspect":{"id":id,"sections":["status"]}}))
}

#[test]
fn catalog_pages_current_and_archived_nonresident_heads_without_provider_work() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let first = submit(&mut owner, create()).await.child_id.unwrap();
        let second = submit(&mut owner, create()).await.child_id.unwrap();
        let archived = submit(
            &mut owner,
            command(serde_json::json!({
                "lifecycle":{"id":first,"action":"close"}
            })),
        )
        .await;
        assert!(archived.ok, "{archived:?}");
        assert!(owner.managed_agents().iter().all(|agent| agent.id != first));
        for filter in [Filter::All, Filter::Current, Filter::Archived] {
            let mut cursor = None;
            let mut rows = Vec::new();
            let mut scanned = 0;
            loop {
                let result = page(&mut owner, filter, cursor, 1).await;
                assert!(result.scanned <= 1);
                assert!(result.entries.len() <= result.scanned);
                scanned += result.scanned;
                rows.extend(result.entries);
                cursor = result.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(scanned, 2);
            assert!(rows.iter().all(|row| !row.recovery_required));
            match filter {
                Filter::All => assert_eq!(rows.len(), 2),
                Filter::Current => assert_eq!(rows[0].id, second),
                Filter::Archived => assert_eq!(rows[0].id, first),
            }
        }
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn catalog_bounds_requests_and_rejects_changed_filter_and_foreign_cursors() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let mut other = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let (mut foreign, foreign_completion) = open(&mut other).await;
        for limit in [0, 65, usize::MAX] {
            assert_eq!(
                owner.request_managed_catalog(Filter::All, None, limit),
                Err(CatalogError::InvalidLimit)
            );
        }
        submit(&mut owner, create()).await;
        submit(&mut owner, create()).await;
        let request = owner.request_managed_catalog(Filter::All, None, 1).unwrap();
        assert_eq!(
            owner.request_managed_catalog(Filter::All, None, 1),
            Err(CatalogError::Busy)
        );
        let result = catalog_outcome(&mut owner).await;
        assert_eq!(result.request, request);
        let cursor = result.result.unwrap().next.unwrap();
        assert_eq!(
            owner.request_managed_catalog(Filter::Current, Some(cursor.clone()), 1),
            Err(CatalogError::InvalidCursor)
        );
        assert_eq!(
            foreign.request_managed_catalog(Filter::All, Some(cursor.clone()), 1),
            Err(CatalogError::InvalidCursor)
        );
        assert_eq!(
            page(&mut owner, Filter::All, Some(cursor), 1)
                .await
                .entries
                .len(),
            1
        );
        close(owner, completion).await;
        close(foreign, foreign_completion).await;
    });
}

#[test]
fn retained_catalog_result_does_not_block_commands_or_actual_shutdown() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        submit(&mut owner, create()).await;
        submit(&mut owner, create()).await;
        let request = owner.request_managed_catalog(Filter::All, None, 1).unwrap();
        // The catalog is admitted ahead of this command. Driving the command
        // deliberately leaves the completed catalog result unconsumed.
        assert!(submit(&mut owner, create()).await.ok);
        assert_eq!(
            owner.request_managed_catalog(Filter::All, None, 1),
            Err(CatalogError::Busy)
        );
        let retained = owner.take_managed_catalog_outcome().unwrap();
        assert_eq!(retained.request, request);
        assert!(retained.result.as_ref().unwrap().next.is_some());
        close(owner, completion).await;
        // Keep the result, cursor and row observation alive through actual join.
        assert_eq!(retained.result.unwrap().entries.len(), 1);
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn observed_command_checks_revision_after_earlier_queued_mutation() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        let row = page(&mut owner, Filter::All, None, 1)
            .await
            .entries
            .remove(0);
        let accepted = owner
            .request_observed_managed_command(
                row.observation.clone(),
                inspect(&child),
                CancellationToken::new(),
            )
            .unwrap();
        assert!(response(&mut owner, accepted).await.unwrap().ok);
        let mutation = owner
            .request_managed_command(
                command(serde_json::json!({
                    "configure":{"id":child,"name":"renamed"}
                })),
                CancellationToken::new(),
            )
            .unwrap();
        let stale = owner
            .request_observed_managed_command(
                row.observation,
                command(serde_json::json!({
                    "message":{"send":{"id":child,"content":"must not execute"}}
                })),
                CancellationToken::new(),
            )
            .unwrap();
        assert!(response(&mut owner, mutation).await.unwrap().ok);
        let rejected = response(&mut owner, stale).await.unwrap();
        assert_eq!(rejected.status, ManagedResultStatus::Rejected);
        assert_eq!(
            rejected.error_code,
            Some(ManagedFailureCode::StaleGeneration)
        );
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn observed_command_rejects_foreign_manager_wrong_target_and_untargeted_create() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let mut other = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let (mut foreign, foreign_completion) = open(&mut other).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        let row = page(&mut owner, Filter::All, None, 1)
            .await
            .entries
            .remove(0);
        for command in [inspect("wrong-child"), create()] {
            assert!(matches!(
                owner.request_observed_managed_command(
                    row.observation.clone(),
                    command,
                    CancellationToken::new(),
                ),
                Err(ManagedSubagentError::Unavailable)
            ));
        }
        assert!(matches!(
            foreign.request_observed_managed_command(
                row.observation,
                inspect(&child),
                CancellationToken::new(),
            ),
            Err(ManagedSubagentError::Unavailable)
        ));
        close(owner, completion).await;
        close(foreign, foreign_completion).await;
    });
}

#[test]
fn parent_replacement_keeps_catalog_identity_but_uses_new_foreground_admission() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        submit(&mut owner, create()).await;
        submit(&mut owner, create()).await;
        let mut first = page(&mut owner, Filter::All, None, 1).await;
        let row = first.entries.remove(0);
        let previous = owner.runtime().clone();
        owner
            .request_transition(NativeInteractiveTransition::New, 11)
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_ne!(previous.id(), owner.runtime().id());
        assert!(previous.enqueue("retired".into()).is_err());
        let accepted = owner
            .request_observed_managed_command(
                row.observation,
                inspect(&row.id),
                CancellationToken::new(),
            )
            .unwrap();
        assert!(response(&mut owner, accepted).await.unwrap().ok);
        assert_eq!(
            page(&mut owner, Filter::All, first.next, 1)
                .await
                .entries
                .len(),
            1
        );
        drop(previous);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn reopening_a_child_never_retargets_an_old_observation_to_its_new_generation() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        let original = page(&mut owner, Filter::All, None, 1)
            .await
            .entries
            .remove(0);
        for action in ["close", "reopen"] {
            let result = submit(
                &mut owner,
                command(serde_json::json!({
                    "lifecycle":{"id":child,"action":action}
                })),
            )
            .await;
            assert!(result.ok, "{result:?}");
        }
        let reopened = page(&mut owner, Filter::All, None, 1)
            .await
            .entries
            .remove(0);
        assert_eq!(reopened.id, original.id);
        assert!(reopened.generation > original.generation);
        let stale = owner
            .request_observed_managed_command(
                original.observation,
                command(serde_json::json!({"lifecycle":{"id":child,"action":"close"}})),
                CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            response(&mut owner, stale).await.unwrap().error_code,
            Some(ManagedFailureCode::StaleGeneration)
        );
        let accepted = owner
            .request_observed_managed_command(
                reopened.observation,
                inspect(&child),
                CancellationToken::new(),
            )
            .unwrap();
        assert!(response(&mut owner, accepted).await.unwrap().ok);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn shutdown_rejects_unstarted_catalog_and_preserves_its_exact_request() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let request: NativeManagedCatalogRequest = owner
            .request_managed_catalog(Filter::All, None, 64)
            .unwrap();
        owner.request_shutdown();
        assert_eq!(
            owner.request_managed_catalog(Filter::All, None, 1),
            Err(CatalogError::Closed)
        );
        let rejected = catalog_outcome(&mut owner).await;
        assert_eq!(rejected.request, request);
        assert!(matches!(rejected.result, Err(CatalogError::Closed)));
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
