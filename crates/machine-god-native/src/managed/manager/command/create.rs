use super::{
    Action, Environment, JournalCreate, JournalWork, ManagedConfiguration, ManagedCreate,
    ManagedFailureCode, ManagedMailboxJob, ManagedOutcome, ManagedRuntimePreparationKind,
    ManagedRuntimeRequest, Outcome, durability, failure, inherited_mode, origin, permitted,
    prepare, principal_owner, receipt, runtime_failure, with_rejected_preparation,
};

pub(super) async fn execute(
    job: ManagedMailboxJob,
    env: Environment,
    request: ManagedCreate,
) -> Outcome {
    if !env.capacity {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::ResourceLimit);
    }
    let mode = request
        .permission_mode
        .unwrap_or_else(|| inherited_mode(job.lease()));
    if !permitted(job.lease(), mode) {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
    }
    let configuration = ManagedConfiguration {
        name: request.name,
        model: Some(
            request
                .model
                .unwrap_or_else(|| job.lease().preferences().model().into()),
        ),
        effort: Some(
            request
                .effort
                .unwrap_or_else(|| job.lease().preferences().effort().label().into()),
        ),
        permission_mode: mode,
        notifications: request.notifications,
    };
    // One retained candidate, allocated only once. A preparation receipt owns ambiguity.
    let transcript = match env.factory.allocate_identity().await {
        Ok(value) => value,
        Err(error) => return Outcome::reject(job, &env.operation, runtime_failure(error)),
    };
    if !job.lease().is_live() {
        return Outcome::reject(job, &env.operation, ManagedFailureCode::CallerUnavailable);
    }
    let id = transcript.session_id.to_string();
    let prepared = match prepare(
        &env,
        ManagedRuntimeRequest {
            kind: ManagedRuntimePreparationKind::Create,
            child_id: id.clone(),
            generation: 1,
            transcript: transcript.clone(),
            journal_owner: env.owner.clone(),
            configuration: configuration.clone(),
            origin: Some(origin(job.lease())),
            now_ms: env.now_ms,
        },
    )
    .await
    {
        Ok(value) => value,
        Err(code) => return Outcome::reject(job, &env.operation, code),
    };
    if !job.lease().is_live() {
        return with_rejected_preparation(
            job,
            Some(prepared),
            &env,
            ManagedFailureCode::CallerUnavailable,
        );
    }
    let parent = principal_owner(job.lease());
    let initial_work = request.prompt.map(|content| JournalWork {
        id: "work-1".into(),
        source_id: parent.session_id.to_string(),
        source_owner: parent.clone(),
        content,
        skills: Vec::new(),
        accepted_at_ms: env.now_ms,
        configuration: configuration.clone(),
    });
    let record = JournalCreate {
        id,
        mode: request.mode,
        configuration,
        transcript,
        controller: parent.clone(),
        parent_id: Some(parent.session_id.to_string()),
        parent_owner: Some(parent),
        parent_generation: Some(job.lease().principal().generation()),
        initial_work,
    };
    match durability::create(&env.journal, &env.gate, record, job.lease()).await {
        Ok(snapshot) => {
            let result = receipt(&env.operation, &snapshot, ManagedOutcome::Created);
            Outcome {
                job,
                snapshot: Some(snapshot),
                prepared: Some(prepared),
                action: Action::Reply(result),
                replay_changed: true,
            }
        }
        Err(error) => with_rejected_preparation(job, Some(prepared), &env, failure(error)),
    }
}
