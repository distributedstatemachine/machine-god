use super::{
    Action, Environment, JournalIntent, JournalMutation, JournalSnapshot, ManagedAgentState,
    ManagedFailureCode, ManagedLifecycle, ManagedLifecycleAction, ManagedMailboxJob,
    ManagedOutcome, ManagedQueueStatus, ManagedRuntimePreparationKind, ManagedRuntimeRequest,
    Outcome, durability, failure, origin, permitted, prepare, prepare_if_needed, publish, receipt,
};

#[allow(clippy::too_many_lines)] // Explicit lifecycle branches preserve original job custody.
pub(super) async fn execute(
    job: ManagedMailboxJob,
    env: Environment,
    mut snapshot: JournalSnapshot,
    request: ManagedLifecycle,
) -> Outcome {
    // A cleared journal intent is not completion of its original control job:
    // worker settlement may still be outstanding. Never replace that custody.
    if env.controls.contains(&request.id) {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
    }
    let busy = env
        .residents
        .iter()
        .any(|(id, _, busy)| id == &request.id && *busy);
    let resident = env.residents.iter().any(|(id, _, _)| id == &request.id);
    match request.action {
        ManagedLifecycleAction::Resume => {
            if busy || snapshot.head.intent.is_some() {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::InvalidState);
            }
            let Some(first) = snapshot.head.queue.first() else {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::NoActiveWork);
            };
            let mutation = JournalMutation::ResolveHead {
                work_id: first.id.clone(),
                retry: true,
            };
            let Ok(work) = env.journal.read_work(first.page.clone()).await else {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::StoreFailure);
            };
            if !permitted(job.lease(), work.configuration.permission_mode) {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
            }
            let prepared = match prepare_if_needed(&env, &snapshot, Some(origin(job.lease()))).await
            {
                Ok(value) => value,
                Err(code) => return Outcome::reject(job, &env.operation, code),
            };
            publish(
                job,
                env,
                snapshot,
                prepared,
                mutation,
                ManagedOutcome::LifecycleChanged,
            )
            .await
        }
        ManagedLifecycleAction::Reopen => {
            if busy || snapshot.head.status != ManagedAgentState::Archived {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::InvalidState);
            }
            if !permitted(job.lease(), snapshot.head.configuration.permission_mode) {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
            }
            let Some(generation) = snapshot.head.generation.checked_add(1) else {
                return Outcome::reject(
                    job,
                    &env.operation,
                    ManagedFailureCode::GenerationExhausted,
                );
            };
            if !env.capacity {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
            }
            if let Err(code) = settle_saved_lifetime(&env, &snapshot, origin(job.lease())).await {
                return Outcome::reject(job, &env.operation, code);
            }
            if !job.lease().is_live() {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::CallerUnavailable);
            }
            let prepared = match prepare(
                &env,
                ManagedRuntimeRequest {
                    kind: ManagedRuntimePreparationKind::Restore,
                    child_id: snapshot.head.id.clone(),
                    generation,
                    transcript: snapshot.head.transcript.clone(),
                    journal_owner: env.owner.clone(),
                    configuration: snapshot.head.configuration.clone(),
                    origin: Some(origin(job.lease())),
                    now_ms: env.now_ms,
                },
            )
            .await
            {
                Ok(value) => value,
                Err(code) => return Outcome::reject(job, &env.operation, code),
            };
            let transcript = snapshot.head.transcript.clone();
            publish(
                job,
                env,
                snapshot,
                Some(prepared),
                JournalMutation::Reopen(transcript),
                ManagedOutcome::LifecycleChanged,
            )
            .await
        }
        ManagedLifecycleAction::Cancel | ManagedLifecycleAction::Close => {
            let archive = request.action == ManagedLifecycleAction::Close;
            let intent = if archive {
                JournalIntent::Archive
            } else {
                JournalIntent::Cancel
            };
            if snapshot.head.intent.is_some_and(|old| old != intent) {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::InvalidState);
            }
            if busy && snapshot.head.intent == Some(intent) {
                let result = receipt(&env.operation, &snapshot, ManagedOutcome::LifecycleChanged);
                return Outcome {
                    job,
                    snapshot: Some(snapshot),
                    prepared: None,
                    action: Action::Reply(result),
                    replay_changed: false,
                };
            }
            if snapshot.head.intent.is_none() {
                snapshot = match durability::mutate_admitted(
                    &env.journal,
                    &env.gate,
                    snapshot,
                    JournalMutation::Intent(intent),
                    job.lease(),
                )
                .await
                {
                    Ok(value) => value,
                    Err(error) => return Outcome::reject(job, &env.operation, failure(error)),
                };
            }
            if busy || (archive && resident) {
                return Outcome {
                    job,
                    snapshot: Some(snapshot),
                    prepared: None,
                    action: if archive {
                        Action::Archive
                    } else {
                        Action::Cancel
                    },
                    replay_changed: true,
                };
            }
            if archive {
                loop {
                    if settle_saved_lifetime(&env, &snapshot, origin(job.lease()))
                        .await
                        .is_ok()
                    {
                        break;
                    }
                    // The close intent is already durable. Keep the original
                    // job until its exact old lifetime can be settled.
                    env.gate.blocked(super::ManagerBlock::Preparation).await;
                }
            }
            let mutation = if archive {
                JournalMutation::Archive
            } else if let Some(work) = snapshot.head.queue.first() {
                JournalMutation::HeadState {
                    work_id: work.id.clone(),
                    status: ManagedQueueStatus::Cancelled,
                    failure: None,
                }
            } else {
                JournalMutation::CancelIdle
            };
            // Intent is already accepted: retirement of the original caller cannot undo it.
            match durability::mutate(env.journal.clone(), env.gate.clone(), snapshot, mutation)
                .await
            {
                Ok(snapshot) => {
                    let result =
                        receipt(&env.operation, &snapshot, ManagedOutcome::LifecycleChanged);
                    Outcome {
                        job,
                        snapshot: Some(snapshot),
                        prepared: None,
                        action: Action::Reply(result),
                        replay_changed: true,
                    }
                }
                Err(error) => Outcome::reject(job, &env.operation, failure(error)),
            }
        }
    }
}

async fn settle_saved_lifetime(
    env: &Environment,
    snapshot: &JournalSnapshot,
    origin: super::ManagedRuntimeOrigin,
) -> Result<(), ManagedFailureCode> {
    // This runtime only repairs saved delivery evidence and retires. It never
    // admits work, so closing a saved child must not require its former execution
    // policy. Restrict the temporary owner to both the saved and caller policies;
    // leave the durable configuration untouched for a later explicit reopen.
    let mut configuration = snapshot.head.configuration.clone();
    configuration.permission_mode = match (origin.policy.mode(), configuration.permission_mode) {
        (crate::PermissionMode::Ask, _) | (_, super::ManagedPermissionMode::Ask) => {
            super::ManagedPermissionMode::Ask
        }
        (crate::PermissionMode::Auto, _) | (_, super::ManagedPermissionMode::Auto) => {
            super::ManagedPermissionMode::Auto
        }
        (crate::PermissionMode::Yolo, super::ManagedPermissionMode::Yolo) => {
            super::ManagedPermissionMode::Yolo
        }
    };
    let mut original = prepare(
        env,
        ManagedRuntimeRequest {
            kind: ManagedRuntimePreparationKind::Restore,
            child_id: snapshot.head.id.clone(),
            generation: snapshot.head.generation,
            transcript: snapshot.head.transcript.clone(),
            journal_owner: env.owner.clone(),
            configuration,
            origin: Some(origin),
            now_ms: env.now_ms,
        },
    )
    .await?;
    if let Some(context) = &original.notice_context
        && let Some(delivery) = context.delivery()
    {
        loop {
            let mut snapshots = Vec::new();
            let result = super::super::delivery::reconcile_sources(
                &env.journal,
                &env.gate,
                &delivery,
                &mut snapshots,
            )
            .await;
            {
                let mut repaired = env.repaired_heads.lock().unwrap();
                for snapshot in snapshots {
                    if let Some(existing) = repaired
                        .iter_mut()
                        .find(|head| head.head.id == snapshot.head.id)
                    {
                        *existing = snapshot;
                    } else {
                        repaired.push(snapshot);
                    }
                }
            }
            if let Ok(identities) = result
                && context
                    .confirm_source_acknowledgements(&delivery, &identities)
                    .is_ok()
            {
                env.notices
                    .acknowledge_recovered(delivery.originals())
                    .map_err(|_| ManagedFailureCode::StoreFailure)?;
                break;
            }
            env.gate.blocked(super::ManagerBlock::Journal).await;
        }
        loop {
            match original.runtime.clear_notice_delivery(&delivery).await {
                Ok(_) => break,
                Err(_) => env.gate.blocked(super::ManagerBlock::Journal).await,
            }
        }
    }
    // No old-generation runtime/principal or cleanup obligation crosses the
    // subsequent generation publication. Preparation remains effects-free.
    original.resources.begin_close();
    loop {
        match std::future::poll_fn(|cx| original.resources.poll_closed(cx)).await {
            Ok(()) => break,
            Err(_) => env.gate.blocked(super::ManagerBlock::Cleanup).await,
        }
    }
    Ok(())
}
