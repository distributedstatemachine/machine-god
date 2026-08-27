use std::collections::VecDeque;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolAuthorization, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS, ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS,
    ASK_USER_QUESTION_TOOL_NAME, AskUserQuestionConfigError, AskUserQuestionTool,
    MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION, MAX_ASK_USER_QUESTION_QUESTIONS,
    MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES, MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES,
    MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES, MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES, MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES,
    MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
    MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES, MAX_ASK_USER_QUESTION_TOTAL_OPTIONS,
    MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, QuestionPromptError, QuestionPromptOutcome,
    QuestionPromptRequest, QuestionPrompter,
};
use serde_json::{Value, json};

type PromptResult = Result<QuestionPromptOutcome, QuestionPromptError>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedOption {
    label: String,
    description: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedQuestion {
    question: String,
    options: Vec<RecordedOption>,
}

#[derive(Default)]
struct ScriptedState {
    calls: usize,
    requests: Vec<Vec<RecordedQuestion>>,
    outcomes: VecDeque<PromptResult>,
}

#[derive(Clone, Default)]
struct ScriptedPrompter {
    state: Arc<Mutex<ScriptedState>>,
}

impl ScriptedPrompter {
    fn new(outcomes: impl IntoIterator<Item = PromptResult>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedState {
                outcomes: outcomes.into_iter().collect(),
                ..ScriptedState::default()
            })),
        }
    }

    fn calls(&self) -> usize {
        self.state.lock().unwrap().calls
    }

    fn requests(&self) -> Vec<Vec<RecordedQuestion>> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl QuestionPrompter for ScriptedPrompter {
    fn prompt(&self, request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let recorded = request
            .questions()
            .iter()
            .map(|question| RecordedQuestion {
                question: question.question().to_owned(),
                options: question
                    .options()
                    .iter()
                    .map(|option| RecordedOption {
                        label: option.label().to_owned(),
                        description: option.description().map(str::to_owned),
                    })
                    .collect(),
            })
            .collect();
        let outcome = {
            let mut state = self.state.lock().unwrap();
            state.calls += 1;
            state.requests.push(recorded);
            state
                .outcomes
                .pop_front()
                .expect("scripted question prompter has another outcome")
        };
        Box::pin(async move { outcome })
    }
}

#[derive(Default)]
struct PendingProbe {
    calls: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
    live: AtomicUsize,
}

#[derive(Clone)]
struct PendingPrompter {
    probe: Arc<PendingProbe>,
}

impl QuestionPrompter for PendingPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        self.probe.calls.fetch_add(1, Ordering::SeqCst);
        self.probe.live.fetch_add(1, Ordering::SeqCst);
        Box::pin(PendingPrompt {
            probe: Arc::clone(&self.probe),
        })
    }
}

struct PendingPrompt {
    probe: Arc<PendingProbe>,
}

impl Future for PendingPrompt {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingPrompt {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
        self.probe.live.fetch_sub(1, Ordering::SeqCst);
    }
}

struct CancelThenReadyPrompter {
    cancellation: CancellationToken,
    outcome: Mutex<Option<PromptResult>>,
}

struct DebugCapturePrompter {
    debug: Arc<Mutex<Option<String>>>,
}

impl QuestionPrompter for DebugCapturePrompter {
    fn prompt(&self, request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        *self.debug.lock().unwrap() = Some(format!("{request:?}"));
        Box::pin(async { Ok(QuestionPromptOutcome::Unavailable) })
    }
}

struct DropCancelsReadyPrompter {
    cancellation: CancellationToken,
}

impl QuestionPrompter for DropCancelsReadyPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        Box::pin(DropCancelsReady {
            cancellation: self.cancellation.clone(),
        })
    }
}

struct DropCancelsReady {
    cancellation: CancellationToken,
}

impl Future for DropCancelsReady {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(QuestionPromptOutcome::Answered(vec![
            "must not publish".to_owned(),
        ])))
    }
}

impl Drop for DropCancelsReady {
    fn drop(&mut self) {
        assert!(self.cancellation.cancel());
    }
}

#[derive(Clone)]
struct PanicOncePrompter {
    calls: Arc<AtomicUsize>,
    panic_in_poll: bool,
    pending_probe: Arc<PendingProbe>,
}

impl QuestionPrompter for PanicOncePrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            call != 0 || self.panic_in_poll,
            "intentional prompt-call panic"
        );
        if call == 0 {
            return Box::pin(PanicOnPoll);
        }
        self.pending_probe.live.fetch_add(1, Ordering::SeqCst);
        Box::pin(PendingPrompt {
            probe: Arc::clone(&self.pending_probe),
        })
    }
}

struct PanicOnPoll;

impl Future for PanicOnPoll {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        panic!("intentional prompt-poll panic")
    }
}

#[derive(Default)]
struct ReentrantState {
    calls: AtomicUsize,
    tool: Mutex<Option<Weak<AskUserQuestionTool>>>,
    arguments: Mutex<Option<Value>>,
    observed: Mutex<Option<Result<ToolOutput, ToolError>>>,
}

#[derive(Clone)]
struct ReentrantPrompter {
    state: Arc<ReentrantState>,
}

impl QuestionPrompter for ReentrantPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let call = self.state.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            Box::pin(ReentrantPending {
                state: Arc::clone(&self.state),
            })
        } else {
            Box::pin(std::future::pending())
        }
    }
}

struct ReentrantPending {
    state: Arc<ReentrantState>,
}

impl Future for ReentrantPending {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for ReentrantPending {
    fn drop(&mut self) {
        let tool = self
            .state
            .tool
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let arguments = self.state.arguments.lock().unwrap().clone().unwrap();
        let result = poll_ready(tool.execute(context(), arguments, CancellationToken::new()));
        *self.state.observed.lock().unwrap() = Some(result);
    }
}

impl QuestionPrompter for CancelThenReadyPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let cancellation = self.cancellation.clone();
        let outcome = self.outcome.lock().unwrap().take().unwrap();
        Box::pin(async move {
            assert!(cancellation.cancel());
            outcome
        })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("ask_user_question future unexpectedly remained pending"),
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("question-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("question-incarnation").unwrap(),
        turn_id: TurnId::new("question-turn").unwrap(),
        call_id: ToolCallId::new("question-call").unwrap(),
    }
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("question-call").unwrap(),
        name: ToolName::new(ASK_USER_QUESTION_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn basic_arguments() -> Value {
    json!({
        "questions": [{
            "question": "Which path?",
            "options": [
                {"label": "First", "description": "The first path"},
                {"label": "Second"}
            ]
        }]
    })
}

#[derive(Clone)]
struct BoundaryOption {
    label: String,
    description: String,
}

#[derive(Clone)]
struct BoundaryQuestion {
    question: String,
    options: Vec<BoundaryOption>,
}

fn boundary_value(questions: &[BoundaryQuestion]) -> Value {
    json!({
        "questions": questions.iter().map(|question| json!({
            "question": question.question,
            "options": question.options.iter().map(|option| json!({
                "label": option.label,
                "description": option.description,
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

fn serialized_argument_boundary(target: usize) -> Value {
    let mut questions = (0..4)
        .map(|question| BoundaryQuestion {
            question: format!("question-{question}"),
            options: (0..6)
                .map(|option| BoundaryOption {
                    label: format!("option-{question}-{option}"),
                    description: String::new(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let base = serde_json::to_vec(&boundary_value(&questions))
        .unwrap()
        .len();
    assert!(base <= target);
    let mut escaped = (target - base) / 2;
    for question in &mut questions {
        for option in &mut question.options {
            let count = escaped.min(MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES);
            option.description.extend(std::iter::repeat_n('\\', count));
            escaped -= count;
        }
    }
    for question in &mut questions {
        let capacity = MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES - question.question.len();
        let count = escaped.min(capacity);
        question.question.extend(std::iter::repeat_n('\\', count));
        escaped -= count;
    }
    assert_eq!(escaped, 0, "fixture lacks enough escaped string capacity");
    if (target - base) % 2 == 1 {
        let question = questions.last_mut().unwrap();
        assert!(question.question.len() < MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES);
        question.question.push('x');
    }
    let value = boundary_value(&questions);
    assert_eq!(serde_json::to_vec(&value).unwrap().len(), target);
    value
}

fn prepare(tool: &AskUserQuestionTool, arguments: Value) -> Value {
    let prepared = tool.prepare(call(arguments)).unwrap();
    assert_eq!(
        prepared.authorization(),
        &PreparedToolAuthorization::NoAuthorityRequired
    );
    prepared.arguments().clone()
}

fn execute(
    tool: &AskUserQuestionTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn assert_error(
    error: &ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
    assert_eq!(error.retryable, retryable);
}

fn assert_invalid(tool: &AskUserQuestionTool, arguments: Value) {
    let error = tool.prepare(call(arguments)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_invalid_arguments",
        "ask_user_question arguments are invalid",
        false,
    );
}

#[test]
fn public_limits_and_schema_are_exact_and_strict() {
    assert_eq!(ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS, 1);
    assert_eq!(ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS, 8);
    assert_eq!(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES, 32_768);
    assert_eq!(MAX_ASK_USER_QUESTION_QUESTIONS, 4);
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES, 1_024);
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES, 4_096);
    assert_eq!(MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION, 6);
    assert_eq!(MAX_ASK_USER_QUESTION_TOTAL_OPTIONS, 24);
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES, 128);
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES, 512);
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES, 512);
    assert_eq!(
        MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES,
        2_048
    );
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES, 32_768);
    assert_eq!(
        MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
        49_152
    );
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES, 4_096);
    assert_eq!(MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, 4_096);
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES, 16_384);
    assert_eq!(MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES, 49_152);

    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), ASK_USER_QUESTION_TOOL_NAME);
    assert_eq!(spec.input_schema["type"], "object");
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(spec.input_schema["required"], json!(["questions"]));
    assert_eq!(
        spec.input_schema["properties"].as_object().unwrap().len(),
        1
    );
    let questions = &spec.input_schema["properties"]["questions"];
    assert_eq!(questions["minItems"], 1);
    assert_eq!(questions["maxItems"], 4);
    assert!(
        !spec
            .input_schema
            .to_string()
            .contains("permission_request_id")
    );
}

#[test]
fn preparation_canonicalizes_in_order_and_success_allows_free_form_other() {
    let prompter = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Answered(vec![
        "  an unlisted Other answer  ".to_owned(),
        "custom\u{1b}answer".to_owned(),
    ]))]);
    let tool = AskUserQuestionTool::new(prompter.clone());
    let arguments = json!({
        "questions": [
            {
                "question": " \n\u{1b}[31mChoose\u{200b}\u{061c} now\r ",
                "options": [
                    {"label": "  Alpha  ", "description": "  visible\nline  "},
                    {"label": "Beta", "description": " \t\r\n "}
                ]
            },
            {
                "question": "Next?",
                "options": [{"label": "One"}, {"label": "Two"}]
            }
        ]
    });

    let prepared = prepare(&tool, arguments);
    assert_eq!(
        prepared,
        json!({
            "questions": [
                {
                    "question": "\\x1b[31mChoose\\u{200b}\\u{061c} now",
                    "options": [
                        {"label": "Alpha", "description": "visible\\x0aline"},
                        {"label": "Beta"}
                    ]
                },
                {
                    "question": "Next?",
                    "options": [{"label": "One"}, {"label": "Two"}]
                }
            ]
        })
    );

    let output = execute(&tool, prepared, CancellationToken::new()).unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!([
            {
                "answer": "an unlisted Other answer",
                "question": "\\x1b[31mChoose\\u{200b}\\u{061c} now"
            },
            {
                "answer": "custom\\x1banswer",
                "question": "Next?"
            }
        ]))
    );
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        prompter.requests(),
        vec![vec![
            RecordedQuestion {
                question: "\\x1b[31mChoose\\u{200b}\\u{061c} now".to_owned(),
                options: vec![
                    RecordedOption {
                        label: "Alpha".to_owned(),
                        description: Some("visible\\x0aline".to_owned()),
                    },
                    RecordedOption {
                        label: "Beta".to_owned(),
                        description: None,
                    },
                ],
            },
            RecordedQuestion {
                question: "Next?".to_owned(),
                options: vec![
                    RecordedOption {
                        label: "One".to_owned(),
                        description: None,
                    },
                    RecordedOption {
                        label: "Two".to_owned(),
                        description: None,
                    },
                ],
            },
        ]]
    );
}

#[test]
fn strict_shape_types_and_ascii_case_duplicate_labels_fail_before_prompting() {
    let prompter = ScriptedPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let invalid = [
        Value::Null,
        json!({}),
        json!({"questions": [], "unknown": true}),
        json!({"questions": "not-an-array"}),
        json!({"questions": []}),
        json!({"questions": [{}]}),
        json!({"questions": [{"question": 7, "options": [{"label":"a"},{"label":"b"}]}]}),
        json!({"questions": [{"question": " ", "options": [{"label":"a"},{"label":"b"}]}]}),
        json!({"questions": [{"question":"q","options":[]}]}),
        json!({"questions": [{"question":"q","options":[{"label":"a"}]}]}),
        json!({"questions": [{"question":"q","options":[{"label":"a"},{"label":"b","extra":0}]}]}),
        json!({"questions": [{"question":"q","options":[{"label":"a"},{"label":"b","description":7}]}]}),
        json!({"questions": [{"question":"q","options":[{"label":" Yes "},{"label":"yes"}]}]}),
    ];
    for arguments in invalid {
        assert_invalid(&tool, arguments);
    }
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn duplicate_labels_are_compared_after_terminal_rendering_and_trim_is_ascii_exact() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    assert_invalid(
        &tool,
        json!({
            "questions": [{
                "question": "q",
                "options": [{"label":"\u{1b}"}, {"label":"\\x1b"}]
            }]
        }),
    );

    prepare(
        &tool,
        json!({
            "questions": [{
                "question": "q",
                "options": [{"label":"Ä"}, {"label":"ä"}]
            }]
        }),
    );

    let prepared = prepare(
        &tool,
        json!({
            "questions": [{
                "question": "\u{000b}question\u{000c}",
                "options": [{"label":"\u{000b}yes\u{000c}"}, {"label":"no"}]
            }]
        }),
    );
    assert_eq!(prepared["questions"][0]["question"], "\\x0bquestion\\x0c");
    assert_eq!(
        prepared["questions"][0]["options"][0]["label"],
        "\\x0byes\\x0c"
    );
}

#[test]
fn permission_request_id_has_its_fixed_precedence_and_redacted_failure() {
    let marker = "PERMISSION_REQUEST_PRIVATE_MARKER";
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    for value in [Value::Null, json!(7), json!(marker)] {
        let error = tool
            .prepare(call(json!({
                "questions": [],
                "permission_request_id": value,
                "also_unknown": marker,
            })))
            .unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_permission_request_unsupported",
            "ask_user_question permission escalation is not supported",
            false,
        );
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains(marker));
    }
}

#[test]
fn question_option_description_and_count_boundaries_are_inclusive() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let exact = json!({
        "questions": (0..4).map(|question| json!({
            "question": format!("{question}{}", "q".repeat(1_023)),
            "options": (0..6).map(|option| json!({
                "label": format!("{option}{}", "l".repeat(127)),
                "description": "d".repeat(512),
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    });
    prepare(&tool, exact);

    for arguments in [
        json!({"questions":[{"question":"q".repeat(1_025),"options":[{"label":"a"},{"label":"b"}]}]}),
        json!({"questions":[{"question":"q","options":[{"label":"a"},{"label":"l".repeat(129)}]}]}),
        json!({"questions":[{"question":"q","options":[{"label":"a"},{"label":"b","description":"d".repeat(513)}]}]}),
    ] {
        let error = tool.prepare(call(arguments)).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_resource_limit",
            "ask_user_question resource limit exceeded",
            false,
        );
    }
    for arguments in [
        json!({"questions": (0..5).map(|_| json!({"question":"q","options":[{"label":"a"},{"label":"b"}]})).collect::<Vec<_>>() }),
        json!({"questions":[{"question":"q","options":(0..7).map(|index| json!({"label":format!("o{index}")})).collect::<Vec<_>>()}]}),
    ] {
        assert_invalid(&tool, arguments);
    }
}

#[test]
fn terminal_encoding_reaches_each_exact_rendered_field_maximum() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let prepared = prepare(
        &tool,
        json!({
            "questions": [{
                "question": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES),
                "options": [
                    {
                        "label": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES),
                        "description": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES)
                    },
                    {"label":"visible"}
                ]
            }]
        }),
    );
    let question = prepared["questions"][0]["question"].as_str().unwrap();
    let option = &prepared["questions"][0]["options"][0];
    assert_eq!(
        question.len(),
        MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES
    );
    assert_eq!(
        option["label"].as_str().unwrap().len(),
        MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES
    );
    assert_eq!(
        option["description"].as_str().unwrap().len(),
        MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES
    );
}

#[test]
fn aggregate_rendered_presentation_limit_is_checked_without_truncation() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let rendered_bytes = 4 * (1 + 341 * 8) + 24 * ((3 + 41 * 8) + 170 * 8);
    assert!(rendered_bytes > MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES);
    let arguments = json!({
        "questions": (0..4).map(|question| json!({
            "question": format!("{question}{}", "\u{200b}".repeat(341)),
            "options": (0..6).map(|option| json!({
                "label": format!("{question}-{option}{}", "\u{200b}".repeat(41)),
                "description": "\u{200b}".repeat(170),
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    });
    assert!(
        serde_json::to_vec(&arguments).unwrap().len()
            < MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES
    );
    let error = tool.prepare(call(arguments)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
}

#[test]
fn incoming_serialized_argument_boundary_is_exact_and_checked_first() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let exact = serialized_argument_boundary(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES);
    assert_eq!(
        serde_json::to_vec(&exact).unwrap().len(),
        MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES
    );
    prepare(&tool, exact);

    let over = serialized_argument_boundary(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES + 1);
    let marker = "PRIVATE_OVERSIZED_INPUT_MARKER";
    let mut over = over.as_object().unwrap().clone();
    over.insert(marker.to_owned(), Value::Bool(true));
    let error = tool.prepare(call(Value::Object(over))).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(marker));
}

#[test]
fn direct_execute_accepts_only_canonical_prepared_arguments() {
    let prompter = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Unavailable)]);
    let tool = AskUserQuestionTool::new(prompter.clone());
    let error = execute(
        &tool,
        json!({
            "questions": [{
                "question": "  noncanonical question  ",
                "options": [{"label":"yes"}, {"label":"no"}]
            }]
        }),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_invalid_arguments",
        "ask_user_question arguments are invalid",
        false,
    );
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn deeply_nested_direct_input_is_rejected_without_invoking_the_prompt() {
    let prompter = ScriptedPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let mut nested = Value::Null;
    for _ in 0..2_048 {
        nested = Value::Array(vec![nested]);
    }
    let mut root = serde_json::Map::new();
    root.insert("questions".to_owned(), Value::Array(Vec::new()));
    root.insert("unknown".to_owned(), nested);
    let error = execute(&tool, Value::Object(root), CancellationToken::new()).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn cancellation_wins_before_prompt_and_over_every_same_poll_outcome() {
    let pre_cancelled = CancellationToken::new();
    assert!(pre_cancelled.cancel());
    let untouched = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Unavailable)]);
    let tool = AskUserQuestionTool::new(untouched.clone());
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, pre_cancelled).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
    assert_eq!(untouched.calls(), 0);

    for outcome in [
        Ok(QuestionPromptOutcome::Answered(vec!["answer".to_owned()])),
        Ok(QuestionPromptOutcome::Cancelled),
        Ok(QuestionPromptOutcome::Unavailable),
        Err(QuestionPromptError::new()),
    ] {
        let cancellation = CancellationToken::new();
        let tool = AskUserQuestionTool::new(CancelThenReadyPrompter {
            cancellation: cancellation.clone(),
            outcome: Mutex::new(Some(outcome)),
        });
        let prepared = prepare(&tool, basic_arguments());
        let error = execute(&tool, prepared, cancellation).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::Cancelled,
            "ask_user_question_cancelled",
            "ask_user_question was cancelled",
            false,
        );
    }

    let cancellation = CancellationToken::new();
    let tool = AskUserQuestionTool::new(DropCancelsReadyPrompter {
        cancellation: cancellation.clone(),
    });
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, cancellation).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
}

#[test]
fn unpolled_pending_drop_and_fail_fast_capacity_are_owned_and_recoverable() {
    let probe = Arc::new(PendingProbe::default());
    let tool = AskUserQuestionTool::with_max_active_prompts(
        PendingPrompter {
            probe: Arc::clone(&probe),
        },
        1,
    )
    .unwrap();
    let prepared = prepare(&tool, basic_arguments());

    let unpolled = tool.execute(context(), prepared.clone(), CancellationToken::new());
    drop(unpolled);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
    assert!(poll_once(first.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.polls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 1);

    let busy = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
        .unwrap_err();
    assert_error(
        &busy,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    let cancelled = CancellationToken::new();
    assert!(cancelled.cancel());
    let error = poll_ready(tool.execute(context(), prepared.clone(), cancelled)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    drop(first);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.live.load(Ordering::SeqCst), 1);
    drop(recovered);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);
}

#[test]
fn pending_drop_destroys_the_prompt_before_releasing_its_active_permit() {
    let state = Arc::new(ReentrantState::default());
    let tool = Arc::new(AskUserQuestionTool::new(ReentrantPrompter {
        state: Arc::clone(&state),
    }));
    *state.tool.lock().unwrap() = Some(Arc::downgrade(&tool));
    let prepared = prepare(&tool, basic_arguments());
    *state.arguments.lock().unwrap() = Some(prepared.clone());
    let mut pending = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(pending.as_mut()).is_pending());
    drop(pending);

    let result = state.observed.lock().unwrap().take().unwrap();
    let error = result.expect_err("reentrant prompt admission must remain busy during child drop");
    assert_error(
        &error,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cancelling_a_pending_prompt_drops_it_and_releases_capacity() {
    let probe = Arc::new(PendingProbe::default());
    let tool = AskUserQuestionTool::new(PendingPrompter {
        probe: Arc::clone(&probe),
    });
    let prepared = prepare(&tool, basic_arguments());
    let cancellation = CancellationToken::new();
    let mut pending = Box::pin(tool.execute(context(), prepared.clone(), cancellation.clone()));
    assert!(poll_once(pending.as_mut()).is_pending());
    assert!(cancellation.cancel());
    let error = match poll_once(pending.as_mut()) {
        Poll::Ready(Err(error)) => error,
        other => panic!("cancelled prompt must finish immediately, got {other:?}"),
    };
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn prompt_call_and_poll_panics_release_capacity_during_unwind() {
    for panic_in_poll in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::new(PendingProbe::default());
        let tool = AskUserQuestionTool::new(PanicOncePrompter {
            calls: Arc::clone(&calls),
            panic_in_poll,
            pending_probe: Arc::clone(&probe),
        });
        let prepared = prepare(&tool, basic_arguments());
        let mut panicking =
            Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
        assert!(catch_unwind(AssertUnwindSafe(|| poll_once(panicking.as_mut()))).is_err());
        drop(panicking);

        let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
        assert!(poll_once(recovered.as_mut()).is_pending());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(probe.live.load(Ordering::SeqCst), 1);
        drop(recovered);
        assert_eq!(probe.live.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn sentinels_and_invalid_prompt_responses_are_exact_and_redacted() {
    for (outcome, expected) in [
        (
            QuestionPromptOutcome::Cancelled,
            "(user cancelled the question)",
        ),
        (
            QuestionPromptOutcome::Unavailable,
            "(ask_user_question is only available in the interactive shell; ask the user freeform instead)",
        ),
    ] {
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(outcome)]));
        let prepared = prepare(&tool, basic_arguments());
        assert_eq!(
            execute(&tool, prepared, CancellationToken::new()).unwrap(),
            ToolOutput::success(expected)
        );
    }

    let private = "PRIVATE_INVALID_ANSWER_MARKER";
    for outcome in [
        Ok(QuestionPromptOutcome::Answered(Vec::new())),
        Ok(QuestionPromptOutcome::Answered(vec![" ".to_owned()])),
    ] {
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([outcome]));
        let prepared = prepare(&tool, basic_arguments());
        let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::Execution,
            "ask_user_question_invalid_response",
            "ask_user_question prompt returned an invalid response",
            false,
        );
        assert!(!format!("{error:?} {error}").contains(private));
    }

    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(
        QuestionPromptOutcome::Answered(vec![format!("{private}{}", "x".repeat(4_096))]),
    )]));
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(private));

    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Err(QuestionPromptError::new())]));
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "ask_user_question_prompt_failed",
        "ask_user_question prompt failed",
        false,
    );
}

#[test]
fn exact_answer_and_aggregate_answer_boundaries_are_enforced() {
    let exact = "a".repeat(4_096);
    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(
        QuestionPromptOutcome::Answered(vec![exact.clone()]),
    )]));
    let prepared = prepare(&tool, basic_arguments());
    assert_eq!(
        execute(&tool, prepared, CancellationToken::new())
            .unwrap()
            .content[0]["answer"],
        exact
    );

    let two_questions = json!({
        "questions": [
            {"question":"first?","options":[{"label":"a"},{"label":"b"}]},
            {"question":"second?","options":[{"label":"a"},{"label":"b"}]}
        ]
    });
    for (arguments, answers) in [
        (basic_arguments(), vec!["a".repeat(4_097)]),
        (two_questions, vec!["a".repeat(2_048), "b".repeat(2_049)]),
    ] {
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(
            QuestionPromptOutcome::Answered(answers),
        )]));
        let prepared = prepare(&tool, arguments);
        let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_resource_limit",
            "ask_user_question resource limit exceeded",
            false,
        );
    }
}

#[test]
fn worst_case_terminal_rendering_keeps_the_complete_tool_output_bounded() {
    let answer = "\0".repeat(MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES);
    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(
        QuestionPromptOutcome::Answered(vec![answer]),
    )]));
    let prepared = prepare(&tool, basic_arguments());
    let output = execute(&tool, prepared, CancellationToken::new()).unwrap();
    let serialized = serde_json::to_vec(&output).unwrap();
    assert!(serialized.len() <= MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES);
    assert_eq!(
        output.content[0]["answer"].as_str().unwrap().len(),
        MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES
    );
}

#[test]
#[allow(clippy::default_constructed_unit_structs)]
fn construction_and_debug_contracts_are_data_free() {
    assert!(AskUserQuestionTool::with_max_active_prompts(ScriptedPrompter::default(), 1).is_ok());
    assert!(AskUserQuestionTool::with_max_active_prompts(ScriptedPrompter::default(), 8).is_ok());
    for limit in [0, 9, usize::MAX] {
        let error =
            AskUserQuestionTool::with_max_active_prompts(ScriptedPrompter::default(), limit)
                .unwrap_err();
        assert_eq!(error, AskUserQuestionConfigError::default());
        assert_eq!(error.to_string(), "invalid ask_user_question limits");
        assert_eq!(format!("{error:?}"), "AskUserQuestionConfigError");
    }
    assert_eq!(QuestionPromptError::new(), QuestionPromptError::default());
    assert_eq!(
        format!("{:?}", QuestionPromptError::new()),
        "QuestionPromptError"
    );

    let private = "PRIVATE_PROMPT_DEBUG_MARKER";
    let prompter = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Answered(vec![
        private.to_owned(),
    ]))]);
    let tool = AskUserQuestionTool::shared_prompter(Arc::new(prompter));
    let prepared = tool
        .prepare(call(json!({
            "questions":[{"question":private,"options":[{"label":"yes","description":private},{"label":"no"}]}]
        })))
        .unwrap();
    for diagnostic in [format!("{tool:?}"), format!("{prepared:?}")] {
        assert!(!diagnostic.contains(private));
    }
    assert!(
        !format!(
            "{:?}",
            QuestionPromptOutcome::Answered(vec![private.to_owned()])
        )
        .contains(private)
    );

    let debug = Arc::new(Mutex::new(None));
    let tool = AskUserQuestionTool::new(DebugCapturePrompter {
        debug: Arc::clone(&debug),
    });
    let prepared = prepare(
        &tool,
        json!({
            "questions":[{"question":private,"options":[{"label":"yes","description":private},{"label":"no"}]}]
        }),
    );
    execute(&tool, prepared, CancellationToken::new()).unwrap();
    assert!(!debug.lock().unwrap().as_ref().unwrap().contains(private));
}
