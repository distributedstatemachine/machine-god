use super::*;
use crate::file_approval::tests::{Fixture, Policy, arguments};
use crate::{NativeFileApprovalKind as Kind, NativeHistoryFileSource};
use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionExecutionAdmission, ToolContext, ToolError,
};
use serde_json::Value;

const KINDS: [Kind; 5] = [
    Kind::Write,
    Kind::Edit,
    Kind::Delete,
    Kind::Copy,
    Kind::Rename,
];

fn catalog(
    fixture: &Fixture,
    kind: Kind,
) -> (
    ReferenceHostToolCatalog,
    Arc<crate::conversation_observations::ObservationSession>,
) {
    let observations = Arc::new(crate::NativeConversationObservations::new());
    let context = Fixture::context();
    let session = observations
        .register(
            context.session_id.clone(),
            context.session_incarnation_id.clone(),
        )
        .unwrap();
    session
        .begin_attempt(context.turn_id.clone(), 0, 1)
        .unwrap();
    let tool: Arc<dyn Tool> = Arc::from(fixture.tool(kind));
    bind_observation(&session, tool.as_ref(), 1);
    let mut catalog =
        ReferenceHostToolCatalog::new(Some(observations), EngineLimits::default(), true);
    register_mutation(&mut catalog, fixture, kind);
    assert!(catalog.workspace_contexts.is_none());
    assert_eq!(catalog.registrations.len(), 1);
    (catalog, session)
}

fn register_mutation(catalog: &mut ReferenceHostToolCatalog, fixture: &Fixture, kind: Kind) {
    macro_rules! register {
        ($tool:ty) => {
            catalog.mutation(
                <$tool>::open(&fixture.path)
                    .unwrap()
                    .with_file_approvals(fixture.registry.clone()),
                kind,
                Some(&fixture.registry),
                None,
            )
        };
    }
    match kind {
        Kind::Write => register!(crate::WriteFileTool),
        Kind::Edit => register!(crate::EditFileTool),
        Kind::Delete => register!(crate::DeleteFileTool),
        Kind::Copy => register!(crate::CopyFileTool),
        Kind::Rename => register!(crate::RenameFileTool),
    }
}

fn bind_observation(
    session: &crate::conversation_observations::ObservationSession,
    tool: &dyn Tool,
    message: usize,
) {
    let context = Fixture::context();
    session
        .bind_call(
            &context,
            NativeHistoryFileSource::new(message, 0, context.call_id.clone(), tool.spec().name)
                .unwrap(),
        )
        .unwrap();
}

fn execute(
    tool: &dyn Tool,
    context: ToolContext,
    args: Value,
    cancellation: CancellationToken,
    contextual: bool,
) -> BoxFuture<'_, Result<(), ToolError>> {
    if contextual {
        let execution = tool.execute_for_turn(context, args, cancellation);
        Box::pin(async move { execution.await.map(|_| ()) })
    } else {
        let execution = tool.execute(context, args, cancellation);
        Box::pin(async move { execution.await.map(|_| ()) })
    }
}

#[test]
fn nonworkspace_history_outer_future_cannot_capture_replacement_file_grant() {
    for kind in KINDS {
        for contextual in [false, true] {
            let fixture = Fixture::new();
            fixture.seed();
            let args = arguments(kind);
            fixture.admit(kind, &args, Policy::new(usize::MAX));
            let (catalog, observations) = catalog(&fixture, kind);
            let tool = &catalog.tools[0];
            let context = Fixture::context();
            let old = execute(
                tool.as_ref(),
                context.clone(),
                args.clone(),
                CancellationToken::new(),
                contextual,
            );
            assert!(observations.snapshot().entries().is_empty());
            fixture.registry.close_turn(
                &context.session_id,
                &context.session_incarnation_id,
                &context.turn_id,
            );
            Box::new(
                fixture
                    .prepare(kind, &args, "replacement")
                    .unwrap()
                    .admit(Policy::new(usize::MAX)),
            )
            .admit()
            .unwrap();
            assert_eq!(block_on(old).unwrap_err().code, "file_approval_failed");
            assert_eq!(std::fs::read(fixture.path.join("a")).unwrap(), b"before");
            assert!(!fixture.path.join("b").exists());
            bind_observation(&observations, tool.as_ref(), 2);
            block_on(execute(
                tool.as_ref(),
                context,
                args,
                CancellationToken::new(),
                contextual,
            ))
            .unwrap();
        }
    }
}

#[test]
fn nonworkspace_history_captures_missing_approval_before_later_admission() {
    for kind in KINDS {
        for contextual in [false, true] {
            let fixture = Fixture::new();
            fixture.seed();
            let args = arguments(kind);
            let (catalog, observations) = catalog(&fixture, kind);
            let tool = catalog.tools[0].as_ref();
            let old = execute(
                tool,
                Fixture::context(),
                args.clone(),
                CancellationToken::new(),
                contextual,
            );
            assert!(observations.snapshot().entries().is_empty());
            fixture.admit(kind, &args, Policy::new(usize::MAX));
            assert_eq!(block_on(old).unwrap_err().code, "file_approval_failed");
            assert_eq!(std::fs::read(fixture.path.join("a")).unwrap(), b"before");
            assert!(!fixture.path.join("b").exists());
            bind_observation(&observations, tool, 2);
            block_on(execute(
                tool,
                Fixture::context(),
                args,
                CancellationToken::new(),
                contextual,
            ))
            .unwrap();
        }
    }
}

#[test]
fn nonworkspace_history_drop_and_denied_observation_do_not_claim_or_mutate() {
    for kind in KINDS {
        for contextual in [false, true] {
            for deny_history in [false, true] {
                let fixture = Fixture::new();
                fixture.seed();
                let args = arguments(kind);
                fixture.admit(kind, &args, Policy::new(usize::MAX));
                let (catalog, observations) = catalog(&fixture, kind);
                let old = execute(
                    catalog.tools[0].as_ref(),
                    Fixture::context(),
                    args.clone(),
                    CancellationToken::new(),
                    contextual,
                );
                assert!(observations.snapshot().entries().is_empty());
                if deny_history {
                    observations.finish_attempt(&Fixture::context().turn_id);
                    assert_eq!(
                        block_on(old).unwrap_err().code,
                        "native_history_observation_unavailable"
                    );
                } else {
                    drop(old);
                }
                assert!(observations.snapshot().entries().is_empty());
                assert_eq!(std::fs::read(fixture.path.join("a")).unwrap(), b"before");
                assert!(!fixture.path.join("b").exists());
                // The exact original grant remains usable after either path.
                block_on(fixture.tool(kind).execute(
                    Fixture::context(),
                    args,
                    CancellationToken::new(),
                ))
                .unwrap();
            }
        }
    }
}

#[test]
fn nonworkspace_history_precancellation_preserves_the_unclaimed_grant() {
    for kind in KINDS {
        for contextual in [false, true] {
            let fixture = Fixture::new();
            fixture.seed();
            let args = arguments(kind);
            fixture.admit(kind, &args, Policy::new(usize::MAX));
            let (catalog, observations) = catalog(&fixture, kind);
            let cancellation = CancellationToken::new();
            let old = execute(
                catalog.tools[0].as_ref(),
                Fixture::context(),
                args.clone(),
                cancellation.clone(),
                contextual,
            );
            cancellation.cancel();
            assert!(observations.snapshot().entries().is_empty());
            assert_eq!(
                block_on(old).unwrap_err().kind,
                machine_god_core::ToolErrorKind::Cancelled
            );
            assert_eq!(std::fs::read(fixture.path.join("a")).unwrap(), b"before");
            assert!(!fixture.path.join("b").exists());
            block_on(fixture.tool(kind).execute(
                Fixture::context(),
                args,
                CancellationToken::new(),
            ))
            .unwrap();
        }
    }
}
