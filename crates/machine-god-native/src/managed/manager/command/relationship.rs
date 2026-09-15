use super::{
    Action, Environment, JournalError, JournalMutation, JournalSnapshot, ManagedAgentState,
    ManagedFailureCode, ManagedMailboxJob, ManagedOutcome, ManagedRelationship,
    ManagedRelationshipAction, ManagedRelationshipProposal, Outcome, authorized, load, origin,
    principal_owner, publish,
};

pub(super) async fn execute(
    job: ManagedMailboxJob,
    env: Environment,
    snapshot: JournalSnapshot,
    request: ManagedRelationship,
) -> Outcome {
    let (parent_id, parent_owner) = if request.action == ManagedRelationshipAction::Detach {
        (None, None)
    } else {
        if request.action == ManagedRelationshipAction::Attach && snapshot.head.parent_id.is_some()
        {
            return Outcome::reject(
                job,
                &env.operation,
                ManagedFailureCode::RelationshipAlreadyParented,
            );
        }
        let actor = principal_owner(job.lease());
        let id = request
            .parent_id
            .unwrap_or_else(|| actor.session_id.to_string());
        let owner = if id == actor.session_id.as_str() {
            actor
        } else {
            let parent = match load(&env, &id).await {
                Ok(value) => value,
                Err(code) => return Outcome::reject(job, &env.operation, code),
            };
            if !authorized(job.lease(), &parent)
                || parent.head.status == ManagedAgentState::Archived
            {
                return Outcome::reject(job, &env.operation, ManagedFailureCode::PermissionDenied);
            }
            parent.head.transcript
        };
        if let Err(code) = check_graph(&env, &snapshot.head.id, &id).await {
            return Outcome::reject(job, &env.operation, code);
        }
        if job.lease().is_human() {
            return publish(
                job,
                env,
                snapshot,
                None,
                JournalMutation::Relationship {
                    parent_id: Some(id),
                    parent_owner: Some(owner),
                },
                ManagedOutcome::RelationshipChanged,
            )
            .await;
        }
        let Some(context) = job.context().cloned() else {
            return Outcome::reject(job, &env.operation, ManagedFailureCode::CallerUnavailable);
        };
        let proposal = ManagedRelationshipProposal {
            origin: origin(job.lease()),
            context,
            child_id: snapshot.head.id.clone(),
            generation: snapshot.head.generation,
            revision: snapshot.head.revision,
            action: request.action,
            previous_parent: snapshot.head.parent_owner.clone(),
            parent: owner.clone(),
        };
        return Outcome {
            job,
            snapshot: Some(snapshot),
            prepared: None,
            action: Action::Approval {
                proposal,
                mutation: JournalMutation::Relationship {
                    parent_id: Some(id),
                    parent_owner: Some(owner),
                },
            },
        };
    };
    publish(
        job,
        env,
        snapshot,
        None,
        JournalMutation::Relationship {
            parent_id,
            parent_owner,
        },
        ManagedOutcome::RelationshipChanged,
    )
    .await
}

pub(super) async fn check_graph(
    env: &Environment,
    child: &str,
    parent: &str,
) -> Result<(), ManagedFailureCode> {
    let mut cursor = Some(parent.to_owned());
    let mut visited = Vec::new();
    while let Some(next) = cursor.take() {
        if next == child || visited.contains(&next) {
            return Err(ManagedFailureCode::RelationshipCycle);
        }
        if visited.len() == 64 {
            return Err(ManagedFailureCode::GraphTooDeep);
        }
        visited.push(next.clone());
        match env.journal.inspect(next).await {
            Ok(value) => cursor = value.head.parent_id,
            Err(JournalError::Missing) => break,
            Err(_) => return Err(ManagedFailureCode::StoreFailure),
        }
    }
    Ok(())
}
