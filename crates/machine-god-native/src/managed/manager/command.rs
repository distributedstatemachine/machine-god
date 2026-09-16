//! Serialized control operations. The original mailbox job remains owned here.
mod create;
mod lifecycle;
mod relationship;

use super::super::{
    actor::ManagedCommandActor,
    store::{
        JournalCreate, JournalError, JournalIntent, JournalMutation, JournalOwner, JournalSnapshot,
        JournalTranscript, JournalWork, ManagedJournal,
    },
};
use super::{
    Arc, ManagedMailboxJob, ManagerBlock, durability,
    factory::{
        ManagedPreparation, ManagedRelationshipProposal, ManagedRuntimeError,
        ManagedRuntimeFactory, ManagedRuntimeOrigin, ManagedRuntimePreparationKind,
        ManagedRuntimeRequest, PreparedManagedRuntime,
    },
};
use machine_god_core::{
    BoxFuture, CancellationToken, ManagedAgentMode, ManagedAgentState, ManagedConfiguration,
    ManagedCreate, ManagedFailureCode, ManagedInspect, ManagedLifecycle, ManagedLifecycleAction,
    ManagedMessage, ManagedOutcome, ManagedPermissionMode, ManagedQueueStatus, ManagedReceipt,
    ManagedRelationship, ManagedRelationshipAction, ManagedRequested, ManagedResultStatus,
    ManagedSubagentCommand, ManagedSubagentResult,
};

pub(super) struct Environment {
    pub journal: ManagedJournal,
    pub owner: JournalOwner,
    pub gate: Arc<durability::RetryGate>,
    pub cursors: Arc<std::sync::Mutex<super::projection::CursorBook>>,
    pub factory: Arc<dyn ManagedRuntimeFactory>,
    pub capacity: bool,
    pub residents: Vec<(String, JournalTranscript, bool)>,
    pub now_ms: i64,
    pub operation: String,
    pub wait_finished: Option<bool>,
    pub repaired_heads: Arc<std::sync::Mutex<Vec<JournalSnapshot>>>,
    pub notices: Arc<super::super::notices::ManagedNotices>,
}
pub(super) struct Outcome {
    pub job: ManagedMailboxJob,
    pub snapshot: Option<JournalSnapshot>,
    pub prepared: Option<PreparedManagedRuntime>,
    pub action: Action,
}
#[allow(clippy::large_enum_variant)] // One operation transfers its complete immutable proposal.
pub(super) enum Action {
    Reply(ManagedSubagentResult),
    Cancel,
    Archive,
    Wait(ManagedInspect),
    Approval {
        proposal: ManagedRelationshipProposal,
        mutation: JournalMutation,
    },
}
impl Outcome {
    fn reject(job: ManagedMailboxJob, operation: &str, code: ManagedFailureCode) -> Self {
        Self {
            job,
            snapshot: None,
            prepared: None,
            action: Action::Reply(rejected(operation, code)),
        }
    }
}

#[allow(clippy::too_many_lines)] // Central exhaustiveness boundary; effects live in dedicated handlers.
pub(super) fn execute(job: ManagedMailboxJob, env: Environment) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if !job.lease().is_live() {
            return Outcome::reject(job, &env.operation, ManagedFailureCode::CallerUnavailable);
        }
        let command = job.command().clone();
        if let ManagedSubagentCommand::Create(request) = command {
            return create::execute(job, env, request).await;
        }
        let id = match &command {
            ManagedSubagentCommand::Inspect(value) => &value.id,
            ManagedSubagentCommand::Message(ManagedMessage::Send(value)) => &value.id,
            ManagedSubagentCommand::Relationship(value) => &value.id,
            ManagedSubagentCommand::Configure(value) => &value.id,
            ManagedSubagentCommand::Lifecycle(value) => &value.id,
            ManagedSubagentCommand::Message(ManagedMessage::Milestone(_)) => {
                return Outcome::reject(
                    job,
                    &env.operation,
                    ManagedFailureCode::MilestoneRequiresActiveWork,
                );
            }
            ManagedSubagentCommand::Create(_) => unreachable!(),
        };
        let mut snapshot = match load(&env, id).await {
            Ok(value) => value,
            Err(code) => return Outcome::reject(job, &env.operation, code),
        };
        if !authorized(job.lease(), &snapshot) {
            return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
        }
        // Recheck after queueing and loading, before even recovery publication.
        // A UI row cannot silently retarget a reopened or concurrently changed head.
        if !job.lease().matches_observation(&snapshot) {
            return Outcome::reject(job, &env.operation, ManagedFailureCode::StaleGeneration);
        }
        // Recovery never executes work or signals cancellation. It only records interruption.
        if snapshot.recovery_required() {
            snapshot = match durability::mutate(
                env.journal.clone(),
                env.gate.clone(),
                snapshot,
                JournalMutation::Recover,
            )
            .await
            {
                Ok(value) => value,
                Err(error) => return Outcome::reject(job, &env.operation, failure(error)),
            };
        }
        if !job.lease().is_live() {
            return Outcome::reject(job, &env.operation, ManagedFailureCode::CallerUnavailable);
        }
        match command {
            ManagedSubagentCommand::Inspect(request) => {
                if env.wait_finished.is_none()
                    && request.wait.as_ref().is_some_and(|wait| {
                        !wait.satisfied(snapshot.head.generation, snapshot.head.status)
                    })
                {
                    return Outcome {
                        job,
                        snapshot: Some(snapshot),
                        prepared: None,
                        action: Action::Wait(request),
                    };
                }
                let result = super::projection::inspect(
                    &env.journal,
                    &env.cursors,
                    &snapshot,
                    &request,
                    &env.operation,
                    env.wait_finished == Some(true),
                )
                .await;
                Outcome {
                    job,
                    snapshot: Some(snapshot),
                    prepared: None,
                    action: Action::Reply(result),
                }
            }
            ManagedSubagentCommand::Message(ManagedMessage::Send(request)) => {
                if snapshot.head.mode != ManagedAgentMode::Persistent {
                    return Outcome::reject(
                        job,
                        &env.operation,
                        ManagedFailureCode::OneOffNotMessageable,
                    );
                }
                if snapshot.head.status == ManagedAgentState::Archived
                    || snapshot.head.intent.is_some()
                {
                    return Outcome::reject(job, &env.operation, ManagedFailureCode::InvalidState);
                }
                if !permitted(job.lease(), snapshot.head.configuration.permission_mode) {
                    return Outcome::reject(
                        job,
                        &env.operation,
                        ManagedFailureCode::PermissionDenied,
                    );
                }
                let prepared =
                    match prepare_if_needed(&env, &snapshot, Some(origin(job.lease()))).await {
                        Ok(value) => value,
                        Err(code) => return Outcome::reject(job, &env.operation, code),
                    };
                let Some(sequence) = snapshot.head.revision.checked_add(1) else {
                    return with_rejected_preparation(
                        job,
                        prepared,
                        &env,
                        ManagedFailureCode::GenerationExhausted,
                    );
                };
                let work = JournalWork {
                    id: format!("work-{sequence}"),
                    source_id: job.lease().principal().owner().session_id().to_string(),
                    source_owner: principal_owner(job.lease()),
                    content: request.content,
                    skills: job.skill_references().to_vec(),
                    accepted_at_ms: env.now_ms,
                    configuration: snapshot.head.configuration.clone(),
                };
                publish(
                    job,
                    env,
                    snapshot,
                    prepared,
                    JournalMutation::Enqueue(work),
                    ManagedOutcome::MessageQueued,
                )
                .await
            }
            ManagedSubagentCommand::Configure(request) => {
                let mut configuration = snapshot.head.configuration.clone();
                if let Some(value) = request.name {
                    configuration.name = value;
                }
                if let Some(value) = request.model {
                    configuration.model = Some(value);
                }
                if let Some(value) = request.effort {
                    configuration.effort = Some(value);
                }
                if let Some(value) = request.notifications {
                    configuration.notifications = value;
                }
                if let Some(value) = request.permission_mode {
                    configuration.permission_mode = value;
                }
                if !permitted(job.lease(), configuration.permission_mode) {
                    return Outcome::reject(
                        job,
                        &env.operation,
                        ManagedFailureCode::PermissionDenied,
                    );
                }
                publish(
                    job,
                    env,
                    snapshot,
                    None,
                    JournalMutation::Configure(configuration),
                    ManagedOutcome::Configured,
                )
                .await
            }
            ManagedSubagentCommand::Relationship(request) => {
                relationship::execute(job, env, snapshot, request).await
            }
            ManagedSubagentCommand::Lifecycle(request) => {
                lifecycle::execute(job, env, snapshot, request).await
            }
            ManagedSubagentCommand::Create(_)
            | ManagedSubagentCommand::Message(ManagedMessage::Milestone(_)) => unreachable!(),
        }
    })
}

async fn load(env: &Environment, id: &str) -> Result<JournalSnapshot, ManagedFailureCode> {
    loop {
        match env.journal.inspect(id.to_owned()).await {
            Ok(snapshot) => return Ok(snapshot),
            Err(JournalError::Busy) => env.gate.blocked(ManagerBlock::Capacity).await,
            Err(JournalError::Missing) => return Err(ManagedFailureCode::ChildUnavailable),
            Err(_) => return Err(ManagedFailureCode::StoreFailure),
        }
    }
}
pub(super) fn principal_owner(lease: &ManagedCommandActor) -> JournalTranscript {
    let owner = lease.principal().owner();
    JournalTranscript {
        session_id: owner.session_id().clone(),
        incarnation: owner.session_incarnation_id().clone(),
    }
}
fn authorized(lease: &ManagedCommandActor, snapshot: &JournalSnapshot) -> bool {
    let caller = principal_owner(lease);
    lease.is_live()
        && (lease.is_human()
            || caller == snapshot.head.controller
            || caller == snapshot.head.transcript
            || snapshot.head.parent_owner.as_ref() == Some(&caller))
}
fn origin(lease: &ManagedCommandActor) -> ManagedRuntimeOrigin {
    ManagedRuntimeOrigin {
        principal: lease.principal().clone(),
        workspace: lease.workspace().clone(),
        policy: lease.policy().clone(),
        preferences: lease.preferences().clone(),
    }
}
fn inherited_mode(lease: &ManagedCommandActor) -> ManagedPermissionMode {
    match lease.policy().mode() {
        crate::PermissionMode::Ask => ManagedPermissionMode::Ask,
        crate::PermissionMode::Auto => ManagedPermissionMode::Auto,
        crate::PermissionMode::Yolo => ManagedPermissionMode::Yolo,
    }
}
fn permitted(lease: &ManagedCommandActor, requested: ManagedPermissionMode) -> bool {
    let rank = |mode| match mode {
        ManagedPermissionMode::Ask => 0,
        ManagedPermissionMode::Auto => 1,
        ManagedPermissionMode::Yolo => 2,
    };
    rank(requested) <= rank(inherited_mode(lease))
}
async fn prepare_if_needed(
    env: &Environment,
    snapshot: &JournalSnapshot,
    origin: Option<ManagedRuntimeOrigin>,
) -> Result<Option<PreparedManagedRuntime>, ManagedFailureCode> {
    if env
        .residents
        .iter()
        .any(|(id, owner, _)| id == &snapshot.head.id && owner == &snapshot.head.transcript)
    {
        return Ok(None);
    }
    if !env.capacity {
        return Err(ManagedFailureCode::ResourceLimit);
    }
    prepare(
        env,
        ManagedRuntimeRequest {
            kind: ManagedRuntimePreparationKind::Restore,
            child_id: snapshot.head.id.clone(),
            generation: snapshot.head.generation,
            transcript: snapshot.head.transcript.clone(),
            journal_owner: env.owner.clone(),
            configuration: snapshot.head.configuration.clone(),
            origin,
            now_ms: env.now_ms,
        },
    )
    .await
    .map(Some)
}
async fn prepare(
    env: &Environment,
    request: ManagedRuntimeRequest,
) -> Result<PreparedManagedRuntime, ManagedFailureCode> {
    let restore = request.kind == ManagedRuntimePreparationKind::Restore;
    let prepared = match env
        .factory
        .prepare(request, CancellationToken::new())
        .await
        .map_err(runtime_failure)?
    {
        ManagedPreparation::Ready(value) => value,
        ManagedPreparation::Ambiguous(mut receipt) => loop {
            match std::future::poll_fn(|cx| receipt.poll_reconcile(cx)).await {
                Ok(Some(value)) => break value,
                Ok(None) => return Err(ManagedFailureCode::StoreFailure),
                Err(_) => env.gate.blocked(ManagerBlock::Preparation).await,
            }
        },
    };
    if restore && prepared.notice_context.is_some() {
        // This is an explicit manager recovery operation, not factory
        // preparation or a new delivery occurrence. Keep the same runtime and
        // its original uncertainty fence across repair failures.
        loop {
            match prepared.runtime.recover_notice_delivery().await {
                Ok(_) => break,
                Err(_) => env.gate.blocked(ManagerBlock::Journal).await,
            }
        }
    }
    Ok(prepared)
}
fn runtime_failure(error: ManagedRuntimeError) -> ManagedFailureCode {
    match error {
        ManagedRuntimeError::Capacity => ManagedFailureCode::ResourceLimit,
        ManagedRuntimeError::Ambiguous => ManagedFailureCode::ControlCommitIndeterminate,
        _ => ManagedFailureCode::HostUnavailable,
    }
}
pub(super) fn failure(error: durability::Failure) -> ManagedFailureCode {
    match error {
        durability::Failure::CallerUnavailable => ManagedFailureCode::CallerUnavailable,
        durability::Failure::Rejected(JournalError::Conflict) => ManagedFailureCode::InvalidState,
        durability::Failure::Rejected(JournalError::Limit | JournalError::Busy) => {
            ManagedFailureCode::ResourceLimit
        }
        _ => ManagedFailureCode::StoreFailure,
    }
}
async fn publish(
    job: ManagedMailboxJob,
    env: Environment,
    snapshot: JournalSnapshot,
    prepared: Option<PreparedManagedRuntime>,
    mutation: JournalMutation,
    outcome: ManagedOutcome,
) -> Outcome {
    if !job.lease().is_live() {
        return with_rejected_preparation(
            job,
            prepared,
            &env,
            ManagedFailureCode::CallerUnavailable,
        );
    }
    match durability::mutate_admitted(&env.journal, &env.gate, snapshot, mutation, job.lease())
        .await
    {
        Ok(snapshot) => {
            let result = receipt(&env.operation, &snapshot, outcome);
            Outcome {
                job,
                snapshot: Some(snapshot),
                prepared,
                action: Action::Reply(result),
            }
        }
        Err(error) => with_rejected_preparation(job, prepared, &env, failure(error)),
    }
}
pub(super) fn approved_relationship(
    job: ManagedMailboxJob,
    env: Environment,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if let JournalMutation::Relationship {
            parent_id: Some(parent),
            ..
        } = &mutation
            && let Err(code) = relationship::check_graph(&env, &snapshot.head.id, parent).await
        {
            return Outcome::reject(job, &env.operation, code);
        }
        publish(
            job,
            env,
            snapshot,
            None,
            mutation,
            ManagedOutcome::RelationshipChanged,
        )
        .await
    })
}
fn with_rejected_preparation(
    job: ManagedMailboxJob,
    prepared: Option<PreparedManagedRuntime>,
    env: &Environment,
    code: ManagedFailureCode,
) -> Outcome {
    Outcome {
        job,
        snapshot: None,
        prepared,
        action: Action::Reply(rejected(&env.operation, code)),
    }
}
pub(super) fn rejected(operation: &str, code: ManagedFailureCode) -> ManagedSubagentResult {
    ManagedSubagentResult {
        ok: false,
        operation_id: operation.into(),
        child_id: None,
        status: ManagedResultStatus::Rejected,
        error_code: Some(code),
        retryable: matches!(code, ManagedFailureCode::ResourceLimit),
        requested: None,
        cursor: None,
    }
}
pub(super) fn receipt(
    operation: &str,
    snapshot: &JournalSnapshot,
    outcome: ManagedOutcome,
) -> ManagedSubagentResult {
    let status = match outcome {
        ManagedOutcome::Created => ManagedResultStatus::Created,
        ManagedOutcome::MessageQueued => ManagedResultStatus::MessageQueued,
        ManagedOutcome::Configured => ManagedResultStatus::Configured,
        ManagedOutcome::RelationshipChanged => ManagedResultStatus::RelationshipChanged,
        ManagedOutcome::LifecycleChanged => ManagedResultStatus::LifecycleChanged,
        ManagedOutcome::MilestoneEmitted => ManagedResultStatus::MilestoneEmitted,
    };
    ManagedSubagentResult {
        ok: true,
        operation_id: operation.into(),
        child_id: Some(snapshot.head.id.clone()),
        status,
        error_code: None,
        retryable: false,
        requested: Some(ManagedRequested::Receipt(ManagedReceipt {
            outcome,
            generation: snapshot.head.generation,
            event_sequence: snapshot.head.next_sequence - 1,
        })),
        cursor: None,
    }
}
