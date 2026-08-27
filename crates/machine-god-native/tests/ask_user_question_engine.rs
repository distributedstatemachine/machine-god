use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, ContentBlock, Engine, EngineEvent, ModelEvent, Role, SessionId,
    SessionIncarnationId, StopReason, ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{
    ASK_USER_QUESTION_TOOL_NAME, AskUserQuestionTool, QuestionPromptError, QuestionPromptOutcome,
    QuestionPromptRequest, QuestionPrompter,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use serde_json::json;

#[derive(Clone, Default)]
struct RecordingQuestionPrompter {
    calls: Arc<AtomicUsize>,
    questions: Arc<Mutex<Vec<Vec<String>>>>,
}

impl QuestionPrompter for RecordingQuestionPrompter {
    fn prompt(
        &self,
        request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.questions.lock().unwrap().push(
            request
                .questions()
                .iter()
                .map(|question| question.question().to_owned())
                .collect(),
        );
        Box::pin(async {
            Ok(QuestionPromptOutcome::Answered(vec![
                "bounded Other answer".to_owned(),
            ]))
        })
    }
}

fn provider() -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("ask-user-question-call").unwrap(),
        name: ToolName::new(ASK_USER_QUESTION_TOOL_NAME).unwrap(),
        arguments: json!({
            "questions": [{
                "question": "  Which path?  ",
                "options": [
                    {"label": "First", "description": "the first path"},
                    {"label": "Second"}
                ]
            }]
        }),
    };
    ScriptedModelProvider::new(
        "ask-user-question-provider",
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall { call },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}

#[test]
fn engine_skips_permission_events_and_policy_but_keeps_tool_lifecycle_and_persistence() {
    let provider = provider();
    let store = InMemorySessionStore::new();
    let permission = ScriptedPermissionHandler::new([]);
    let prompter = RecordingQuestionPrompter::default();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(permission.clone())
        .tool(AskUserQuestionTool::new(prompter.clone()))
        .build()
        .unwrap();
    let session_id = SessionId::new("ask-user-question-engine").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new("ask-user-question-engine-incarnation").unwrap(),
        )
        .unwrap();

    let events = futures_executor::block_on(async {
        session
            .prompt("ask the user")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<EngineEvent>, _>>()
            .unwrap()
    });

    assert!(permission.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. } | TurnEvent::PermissionResolved { .. }
    )));
    let started = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .unwrap();
    assert!(started < finished);
    assert_eq!(prompter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        prompter.questions.lock().unwrap().as_slice(),
        [vec!["Which path?".to_owned()]]
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].request.tools[0].name.as_str(),
        ASK_USER_QUESTION_TOOL_NAME
    );
    let message = &requests[1].request.messages[2];
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
        panic!("expected a durable ask_user_question result")
    };
    let expected = ToolOutput::success(json!([{
        "answer": "bounded Other answer",
        "question": "Which path?"
    }]));
    assert_eq!(output, &expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], *message);
}
