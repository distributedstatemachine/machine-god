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
    if matches!(
        request.action,
        ManagedLifecycleAction::Close | ManagedLifecycleAction::Reopen
    ) && env.retiring.contains(&snapshot.head.transcript)
    {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
    }
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
            if snapshot.head.status != ManagedAgentState::Archived {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::InvalidState);
            }
            if resident {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
            }
            if !permitted(job.lease(), snapshot.head.configuration.permission_mode) {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
            }
            if snapshot.head.generation.checked_add(1).is_none() {
                return Outcome::reject(
                    job,
                    &env.operation,
                    ManagedFailureCode::GenerationExhausted,
                );
            }
            if !env.capacity {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
            }
            saved_lifetime(job, env, snapshot, true)
        }
        ManagedLifecycleAction::Cancel | ManagedLifecycleAction::Close => {
            let archive = request.action == ManagedLifecycleAction::Close;
            if archive && !resident && !env.capacity {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
            }
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
                return saved_lifetime(job, env, snapshot, false);
            }
            let mutation = if let Some(work) = snapshot.head.queue.first() {
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

fn saved_lifetime(
    job: ManagedMailboxJob,
    env: Environment,
    snapshot: JournalSnapshot,
    reopen: bool,
) -> Outcome {
    // This runtime only repairs saved delivery evidence and retires. It never
    // admits work, so closing a saved child must not require its former execution
    // policy. Restrict the temporary owner to both the saved and caller policies;
    // leave the durable configuration untouched for a later explicit reopen.
    let mut configuration = snapshot.head.configuration.clone();
    let origin = origin(job.lease());
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
    let request = ManagedRuntimeRequest {
        kind: ManagedRuntimePreparationKind::Restore,
        child_id: snapshot.head.id.clone(),
        generation: snapshot.head.generation,
        transcript: snapshot.head.transcript.clone(),
        journal_owner: env.owner.clone(),
        configuration,
        origin: Some(origin),
        now_ms: env.now_ms,
    };
    let preparation = Box::pin(async move {
        loop {
            match prepare(&env, request.clone()).await {
                Ok(prepared) => return Ok(prepared),
                Err(code) if reopen => return Err(code),
                Err(_) => env.gate.blocked(super::ManagerBlock::Preparation).await,
            }
        }
    });
    Outcome {
        job,
        snapshot: Some(snapshot),
        prepared: None,
        action: Action::SavedLifetime {
            reopen,
            preparation,
        },
        replay_changed: !reopen,
    }
}

/// Resume only after the original saved owner, outbox and resources are gone.
pub(in crate::managed::manager) async fn resume_reopen(
    job: ManagedMailboxJob,
    env: Environment,
    expected: JournalSnapshot,
) -> Outcome {
    if !job.lease().is_live() {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::CallerUnavailable);
    }
    let Ok(snapshot) = env.journal.inspect(expected.head.id.clone()).await else {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::StoreFailure);
    };
    // execute checked the original observed row before its owned recovery.
    // This retained head follows only that admission's confirmed maintenance;
    // checking the historical row again would reject our own revision changes.
    if snapshot.head != expected.head {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::StaleGeneration);
    }
    if !super::authorized(job.lease(), &snapshot)
        || !permitted(job.lease(), snapshot.head.configuration.permission_mode)
    {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
    }
    if snapshot.head.status != ManagedAgentState::Archived || snapshot.head.intent.is_some() {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::InvalidState);
    }
    let Some(generation) = snapshot.head.generation.checked_add(1) else {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::GenerationExhausted);
    };
    if !env.capacity {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
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
        Ok(prepared) => prepared,
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
