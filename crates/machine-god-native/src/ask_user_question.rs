//! Bounded, rootless user-question tool over an explicitly injected host prompt.

use std::fmt::{self, Write as _};
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, Cancelled, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Map, Value, json};

/// Model-visible tool name.
pub const ASK_USER_QUESTION_TOOL_NAME: &str = "ask_user_question";
/// Fixed successful content for an explicit user cancellation.
pub const ASK_USER_QUESTION_CANCELLED_SENTINEL: &str = "(user cancelled the question)";
/// Fixed successful content for a host without an interactive question surface.
pub const ASK_USER_QUESTION_UNAVAILABLE_SENTINEL: &str =
    "(ask_user_question is only available in the interactive shell; ask the user freeform instead)";

/// Maximum compact serialized bytes accepted from the model.
pub const MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES: usize = 32 * 1024;
/// Maximum number of questions in one prompt batch.
pub const MAX_ASK_USER_QUESTION_QUESTIONS: usize = 4;
/// Maximum raw UTF-8 bytes in one question.
pub const MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES: usize = 1024;
/// Maximum terminal-safe UTF-8 bytes in one question.
pub const MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES: usize = 4 * 1024;
/// Maximum number of options for one question.
pub const MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION: usize = 6;
/// Maximum number of options in one complete prompt batch.
pub const MAX_ASK_USER_QUESTION_TOTAL_OPTIONS: usize = 24;
/// Maximum raw UTF-8 bytes in one option label.
pub const MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES: usize = 128;
/// Maximum terminal-safe UTF-8 bytes in one option label.
pub const MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES: usize = 512;
/// Maximum raw UTF-8 bytes in one option description.
pub const MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES: usize = 512;
/// Maximum terminal-safe UTF-8 bytes in one option description.
pub const MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES: usize = 2 * 1024;
/// Maximum aggregate terminal-safe question, label, and description bytes.
pub const MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES: usize = 32 * 1024;
/// Maximum compact serialized bytes in normalized prepared arguments.
pub const MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES: usize = 48 * 1024;
/// Maximum raw UTF-8 bytes in one host answer.
pub const MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES: usize = 4 * 1024;
/// Maximum aggregate raw UTF-8 bytes in all host answers.
pub const MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES: usize = 4 * 1024;
/// Maximum aggregate terminal-safe UTF-8 bytes in all host answers.
pub const MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES: usize = 16 * 1024;
/// Maximum compact serialized bytes in the complete [`ToolOutput`].
pub const MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES: usize = 48 * 1024;
/// Default number of simultaneous prompts admitted by one tool instance.
pub const ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS: usize = 1;
/// Hard number of simultaneous prompts admitted by one tool instance.
pub const ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS: usize = 8;

const ASK_USER_QUESTION_DESCRIPTION: &str = "Ask the user one to four bounded questions with ordered options. Options guide the response but a bounded free-form answer is allowed. Use this only when progress requires an explicit user decision; it is not an approval or permission channel.";
const MAX_ACTIVITY_WAKE_CALLBACKS_PER_PROMPT: usize = 256;

/// One normalized option supplied to a [`QuestionPrompter`].
#[derive(Clone, Eq, PartialEq)]
pub struct QuestionPromptOption {
    label: String,
    description: Option<String>,
}

impl QuestionPromptOption {
    /// Returns the normalized terminal-safe label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the normalized terminal-safe description, when present.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

impl fmt::Debug for QuestionPromptOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionPromptOption")
            .field("has_description", &self.description.is_some())
            .finish_non_exhaustive()
    }
}

/// One normalized question supplied to a [`QuestionPrompter`].
#[derive(Clone, Eq, PartialEq)]
pub struct QuestionPrompt {
    question: String,
    options: Vec<QuestionPromptOption>,
}

impl QuestionPrompt {
    /// Returns the normalized terminal-safe question text.
    #[must_use]
    pub fn question(&self) -> &str {
        &self.question
    }

    /// Returns the normalized ordered options.
    #[must_use]
    pub fn options(&self) -> &[QuestionPromptOption] {
        &self.options
    }
}

impl fmt::Debug for QuestionPrompt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionPrompt")
            .field("option_count", &self.options.len())
            .finish_non_exhaustive()
    }
}

/// Complete owned normalized batch supplied to a [`QuestionPrompter`].
#[derive(Clone, Eq, PartialEq)]
pub struct QuestionPromptRequest {
    questions: Vec<QuestionPrompt>,
}

impl QuestionPromptRequest {
    /// Returns the normalized ordered questions.
    #[must_use]
    pub fn questions(&self) -> &[QuestionPrompt] {
        &self.questions
    }
}

impl fmt::Debug for QuestionPromptRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionPromptRequest")
            .field("question_count", &self.questions.len())
            .finish()
    }
}

/// Bounded ordered answers returned by a [`QuestionPrompter`].
///
/// The fixed private storage makes more than four answers unrepresentable at
/// the prompt-outcome boundary. Hosts add one owned answer at a time, so this
/// API never accepts an unbounded collection that execution would later need
/// to reject or destroy.
#[derive(Clone, Eq, PartialEq)]
pub struct QuestionPromptAnswers {
    answers: [Option<String>; MAX_ASK_USER_QUESTION_QUESTIONS],
    len: usize,
}

impl QuestionPromptAnswers {
    /// Creates an empty answer batch.
    ///
    /// Empty and otherwise short batches remain representable so the tool can
    /// reject a host response whose answer count does not match the request.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            answers: [None, None, None, None],
            len: 0,
        }
    }

    /// Adds one ordered answer without accepting an intermediate collection.
    ///
    /// # Errors
    ///
    /// Returns the supplied answer unchanged when all four slots are occupied.
    pub fn try_push(&mut self, answer: String) -> Result<(), String> {
        let Some(slot) = self.answers.get_mut(self.len) else {
            return Err(answer);
        };
        *slot = Some(answer);
        self.len += 1;
        Ok(())
    }

    /// Returns the number of ordered answers.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the batch contains no answers.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterates over the ordered answers.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.answers[..self.len].iter().filter_map(Option::as_deref)
    }

    fn into_values(self) -> impl ExactSizeIterator<Item = String> {
        self.answers
            .into_iter()
            .take(self.len)
            .map(|answer| answer.expect("occupied answer slots are present"))
    }
}

impl Default for QuestionPromptAnswers {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for QuestionPromptAnswers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionPromptAnswers")
            .field("answer_count", &self.len)
            .finish_non_exhaustive()
    }
}

/// Structured outcome returned by a [`QuestionPrompter`].
#[derive(Clone, Eq, PartialEq)]
pub enum QuestionPromptOutcome {
    /// One ordered answer for every question.
    Answered(QuestionPromptAnswers),
    /// The user explicitly cancelled the prompt.
    Cancelled,
    /// The host has no interactive question surface.
    Unavailable,
}

impl fmt::Debug for QuestionPromptOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Answered(answers) => formatter
                .debug_struct("Answered")
                .field("answer_count", &answers.len())
                .finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
            Self::Unavailable => formatter.write_str("Unavailable"),
        }
    }
}

/// Fixed, data-free failure returned by a [`QuestionPrompter`].
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct QuestionPromptError;

impl QuestionPromptError {
    /// Creates a redacted prompt failure.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl fmt::Debug for QuestionPromptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("QuestionPromptError")
    }
}

impl fmt::Display for QuestionPromptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ask_user_question prompt failed")
    }
}

impl std::error::Error for QuestionPromptError {}

/// Executor-neutral host boundary for one owned question batch.
///
/// Implementations must keep all interaction work owned by the returned
/// future. Dropping that future must not leave a task, thread, callback, or
/// other prompt activity detached.
pub trait QuestionPrompter: Send + Sync + 'static {
    /// Presents `request` and resolves with one structured outcome.
    fn prompt(
        &self,
        request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>>;
}

/// Fixed construction failure for an invalid active-prompt bound.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct AskUserQuestionConfigError;

impl fmt::Debug for AskUserQuestionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AskUserQuestionConfigError")
    }
}

impl fmt::Display for AskUserQuestionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid ask_user_question limits")
    }
}

impl std::error::Error for AskUserQuestionConfigError {}

/// Rootless bounded question tool over an explicitly injected prompter.
pub struct AskUserQuestionTool {
    prompter: Arc<dyn QuestionPrompter>,
    active_prompts: Arc<AtomicUsize>,
    max_active_prompts: usize,
}

impl AskUserQuestionTool {
    /// Constructs a tool with the default one-active-prompt bound.
    #[must_use]
    pub fn new(prompter: impl QuestionPrompter) -> Self {
        Self::shared_prompter(Arc::new(prompter))
    }

    /// Constructs a tool around a shared prompter with the default bound.
    #[must_use]
    pub fn shared_prompter(prompter: Arc<dyn QuestionPrompter>) -> Self {
        Self {
            prompter,
            active_prompts: Arc::new(AtomicUsize::new(0)),
            max_active_prompts: ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS,
        }
    }

    /// Constructs a tool with an explicit active-prompt bound.
    ///
    /// # Errors
    ///
    /// Returns a fixed error unless `max_active_prompts` is in `1..=8`.
    pub fn with_max_active_prompts(
        prompter: impl QuestionPrompter,
        max_active_prompts: usize,
    ) -> Result<Self, AskUserQuestionConfigError> {
        Self::with_shared_prompter_and_max_active_prompts(Arc::new(prompter), max_active_prompts)
    }

    /// Constructs a tool around a shared prompter with an explicit bound.
    ///
    /// # Errors
    ///
    /// Returns a fixed error unless `max_active_prompts` is in `1..=8`.
    pub fn with_shared_prompter_and_max_active_prompts(
        prompter: Arc<dyn QuestionPrompter>,
        max_active_prompts: usize,
    ) -> Result<Self, AskUserQuestionConfigError> {
        if !(1..=ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS).contains(&max_active_prompts) {
            return Err(AskUserQuestionConfigError);
        }
        Ok(Self {
            prompter,
            active_prompts: Arc::new(AtomicUsize::new(0)),
            max_active_prompts,
        })
    }

    fn try_acquire_prompt(&self) -> Option<Arc<ActivePromptPermit>> {
        self.active_prompts
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.max_active_prompts).then(|| active + 1)
            })
            .ok()
            .map(|_| {
                Arc::new(ActivePromptPermit {
                    active_prompts: Some(Arc::clone(&self.active_prompts)),
                })
            })
    }
}

impl fmt::Debug for AskUserQuestionTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AskUserQuestionTool")
            .field("max_active_prompts", &self.max_active_prompts)
            .finish_non_exhaustive()
    }
}

impl Tool for AskUserQuestionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ask_user_question_name(),
            description: ASK_USER_QUESTION_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_ASK_USER_QUESTION_QUESTIONS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": { "type": "string" },
                                "options": {
                                    "type": "array",
                                    "minItems": 2,
                                    "maxItems": MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION,
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": { "type": "string" },
                                            "description": { "type": "string" }
                                        },
                                        "required": ["label"],
                                        "additionalProperties": false
                                    }
                                }
                            },
                            "required": ["question", "options"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["questions"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name.as_str() != ASK_USER_QUESTION_TOOL_NAME {
            drop_json_value_iterative(call.arguments);
            return Err(invalid_arguments());
        }
        let arguments = JsonOwner::new(call.arguments);
        let normalized = normalize_arguments(&arguments, ArgumentPhase::Incoming)?;
        Ok(PreparedToolCall::without_authority(normalized.arguments))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let arguments = JsonOwner::new(arguments);
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let normalized = normalize_arguments(&arguments, ArgumentPhase::Prepared)?;
            check_cancellation(&cancellation)?;

            let Some(permit) = self.try_acquire_prompt() else {
                check_cancellation(&cancellation)?;
                return Err(prompt_busy());
            };
            if cancellation.is_cancelled() {
                drop(permit);
                return Err(cancelled());
            }

            let output_questions: Vec<String> = normalized
                .request
                .questions
                .iter()
                .map(|question| question.question.clone())
                .collect();
            check_cancellation(&cancellation)?;
            let prompt = match catch_unwind(AssertUnwindSafe(|| {
                self.prompter.prompt(normalized.request)
            })) {
                Ok(prompt) => prompt,
                Err(payload) => {
                    drop(permit);
                    resume_unwind(payload);
                }
            };
            let mut activity = Some(PromptActivity::new(
                prompt,
                cancellation.cancelled(),
                permit,
            ));
            let prompted = poll_fn(|context| {
                if cancellation.is_cancelled() {
                    return Poll::Ready(Err(cancelled()));
                }
                if activity
                    .as_ref()
                    .expect("prompt activity remains present while polling")
                    .wake_delivery_resource_exhausted()
                {
                    return Poll::Ready(Err(prompt_failed()));
                }
                if activity
                    .as_mut()
                    .expect("prompt activity remains present while polling")
                    .poll_cancellation(context)
                    .is_ready()
                {
                    return Poll::Ready(Err(cancelled()));
                }
                let polled = catch_unwind(AssertUnwindSafe(|| {
                    activity
                        .as_mut()
                        .expect("prompt activity remains present while polling")
                        .poll_prompt(context)
                }));
                match polled {
                    Ok(Poll::Ready(result)) => {
                        if cancellation.is_cancelled() {
                            Poll::Ready(Err(cancelled()))
                        } else {
                            Poll::Ready(Ok(result))
                        }
                    }
                    Ok(Poll::Pending) => {
                        if cancellation.is_cancelled() {
                            Poll::Ready(Err(cancelled()))
                        } else {
                            Poll::Pending
                        }
                    }
                    Err(payload) => {
                        let doomed = activity.take();
                        let cleanup = catch_unwind(AssertUnwindSafe(|| drop(doomed)));
                        settle_cleanup_panics(true, [cleanup]);
                        resume_unwind(payload);
                    }
                }
            })
            .await;

            let output = match prompted {
                Err(error) => Err(error),
                Ok(Err(_)) => Err(prompt_failed()),
                Ok(Ok(outcome)) => render_outcome(outcome, &output_questions, &cancellation),
            };
            drop(activity.take());
            match check_cancellation(&cancellation) {
                Ok(()) => output,
                Err(error) => Err(error),
            }
        })
    }
}

struct NormalizedArguments {
    request: QuestionPromptRequest,
    arguments: Value,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ArgumentPhase {
    Incoming,
    Prepared,
}

#[allow(clippy::too_many_lines)]
fn normalize_arguments(
    owner: &JsonOwner,
    phase: ArgumentPhase,
) -> Result<NormalizedArguments, ToolError> {
    let serialized_limit = match phase {
        ArgumentPhase::Incoming => MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES,
        ArgumentPhase::Prepared => MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
    };
    if !serialized_json_value_fits(owner.value(), serialized_limit) {
        return Err(resource_limit());
    }
    let Value::Object(root) = owner.value() else {
        return Err(invalid_arguments());
    };
    if root.contains_key("permission_request_id") {
        return Err(permission_request_unsupported());
    }
    if root.len() != 1 || !root.contains_key("questions") {
        return Err(invalid_arguments());
    }
    let Some(Value::Array(question_values)) = root.get("questions") else {
        return Err(invalid_arguments());
    };
    if !(1..=MAX_ASK_USER_QUESTION_QUESTIONS).contains(&question_values.len()) {
        return Err(invalid_arguments());
    }

    let mut rendered_presentation = 0_usize;
    let mut total_options = 0_usize;
    let mut questions = Vec::with_capacity(question_values.len());
    let mut preimage_questions =
        (phase == ArgumentPhase::Prepared).then(|| Vec::with_capacity(question_values.len()));
    for question_value in question_values {
        let Value::Object(question_object) = question_value else {
            return Err(invalid_arguments());
        };
        if question_object.len() != 2
            || !question_object.contains_key("question")
            || !question_object.contains_key("options")
        {
            return Err(invalid_arguments());
        }
        let Some(Value::String(question)) = question_object.get("question") else {
            return Err(invalid_arguments());
        };
        let question = normalize_required_text(
            question,
            phase,
            MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES,
            MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES,
        )?;
        add_presentation_bytes(&mut rendered_presentation, question.rendered.len())?;

        let Some(Value::Array(option_values)) = question_object.get("options") else {
            return Err(invalid_arguments());
        };
        if !(2..=MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION).contains(&option_values.len()) {
            return Err(invalid_arguments());
        }
        total_options = total_options
            .checked_add(option_values.len())
            .filter(|total| *total <= MAX_ASK_USER_QUESTION_TOTAL_OPTIONS)
            .ok_or_else(resource_limit)?;

        let mut options: Vec<QuestionPromptOption> = Vec::with_capacity(option_values.len());
        let mut preimage_options = preimage_questions
            .as_ref()
            .map(|_| Vec::with_capacity(option_values.len()));
        for option_value in option_values {
            let Value::Object(option_object) = option_value else {
                return Err(invalid_arguments());
            };
            if !(option_object.len() == 1 || option_object.len() == 2)
                || !option_object.contains_key("label")
                || option_object
                    .keys()
                    .any(|key| key != "label" && key != "description")
            {
                return Err(invalid_arguments());
            }
            let Some(Value::String(label)) = option_object.get("label") else {
                return Err(invalid_arguments());
            };
            let label = normalize_required_text(
                label,
                phase,
                MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES,
                MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES,
            )?;
            if options
                .iter()
                .any(|existing| existing.label.eq_ignore_ascii_case(&label.rendered))
            {
                return Err(invalid_arguments());
            }
            add_presentation_bytes(&mut rendered_presentation, label.rendered.len())?;

            let description = match option_object.get("description") {
                None => None,
                Some(Value::String(description)) => normalize_optional_text(
                    description,
                    phase,
                    MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES,
                    MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES,
                )?,
                Some(_) => return Err(invalid_arguments()),
            };
            if let Some(description) = &description {
                add_presentation_bytes(&mut rendered_presentation, description.rendered.len())?;
            }
            let (rendered_description, preimage_description) = match description {
                Some(description) => (Some(description.rendered), description.incoming_preimage),
                None => (None, None),
            };
            options.push(QuestionPromptOption {
                label: label.rendered,
                description: rendered_description,
            });
            if let Some(preimage_options) = &mut preimage_options {
                preimage_options.push(QuestionPromptOption {
                    label: label
                        .incoming_preimage
                        .expect("prepared labels retain an incoming preimage"),
                    description: preimage_description,
                });
            }
        }
        questions.push(QuestionPrompt {
            question: question.rendered,
            options,
        });
        if let Some(preimage_questions) = &mut preimage_questions {
            preimage_questions.push(QuestionPrompt {
                question: question
                    .incoming_preimage
                    .expect("prepared questions retain an incoming preimage"),
                options: preimage_options.expect("prepared options retain incoming preimages"),
            });
        }
    }

    let request = QuestionPromptRequest { questions };
    let normalized_arguments = request_to_arguments(&request);
    if !serialized_json_value_fits(
        &normalized_arguments,
        MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
    ) {
        return Err(resource_limit());
    }
    if phase == ArgumentPhase::Prepared && owner.value() != &normalized_arguments {
        return Err(invalid_arguments());
    }
    if let Some(questions) = preimage_questions {
        let incoming_preimage = QuestionPromptRequest { questions };
        if !serialized_json_value_fits(
            &request_to_arguments(&incoming_preimage),
            MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES,
        ) {
            return Err(resource_limit());
        }
    }
    Ok(NormalizedArguments {
        request,
        arguments: normalized_arguments,
    })
}

struct NormalizedText {
    rendered: String,
    incoming_preimage: Option<String>,
}

fn normalize_required_text(
    text: &str,
    phase: ArgumentPhase,
    raw_limit: usize,
    rendered_limit: usize,
) -> Result<NormalizedText, ToolError> {
    let trimmed = trim_ascii_edges(text);
    if trimmed.is_empty() {
        return Err(invalid_arguments());
    }
    match phase {
        ArgumentPhase::Incoming => {
            if trimmed.len() > raw_limit {
                return Err(resource_limit());
            }
            let rendered = encode_terminal_safe(trimmed);
            if rendered.len() > rendered_limit {
                return Err(resource_limit());
            }
            Ok(NormalizedText {
                rendered,
                incoming_preimage: None,
            })
        }
        ArgumentPhase::Prepared => {
            if trimmed != text {
                return Err(invalid_arguments());
            }
            if text.len() > rendered_limit {
                return Err(resource_limit());
            }
            let rendered = encode_terminal_safe(text);
            if rendered != text {
                return Err(invalid_arguments());
            }
            let incoming_preimage = terminal_safe_preimage(text, raw_limit)?;
            Ok(NormalizedText {
                rendered,
                incoming_preimage: Some(incoming_preimage),
            })
        }
    }
}

fn normalize_optional_text(
    text: &str,
    phase: ArgumentPhase,
    raw_limit: usize,
    rendered_limit: usize,
) -> Result<Option<NormalizedText>, ToolError> {
    let trimmed = trim_ascii_edges(text);
    if trimmed.is_empty() {
        return match phase {
            ArgumentPhase::Incoming => Ok(None),
            ArgumentPhase::Prepared => Err(invalid_arguments()),
        };
    }
    normalize_required_text(text, phase, raw_limit, rendered_limit).map(Some)
}

fn add_presentation_bytes(total: &mut usize, bytes: usize) -> Result<(), ToolError> {
    *total = total
        .checked_add(bytes)
        .filter(|total| *total <= MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES)
        .ok_or_else(resource_limit)?;
    Ok(())
}

fn request_to_arguments(request: &QuestionPromptRequest) -> Value {
    Value::Object(Map::from_iter([(
        "questions".to_owned(),
        Value::Array(
            request
                .questions
                .iter()
                .map(|question| {
                    Value::Object(Map::from_iter([
                        (
                            "question".to_owned(),
                            Value::String(question.question.clone()),
                        ),
                        (
                            "options".to_owned(),
                            Value::Array(
                                question
                                    .options
                                    .iter()
                                    .map(|option| {
                                        let mut object = Map::new();
                                        object.insert(
                                            "label".to_owned(),
                                            Value::String(option.label.clone()),
                                        );
                                        if let Some(description) = &option.description {
                                            object.insert(
                                                "description".to_owned(),
                                                Value::String(description.clone()),
                                            );
                                        }
                                        Value::Object(object)
                                    })
                                    .collect(),
                            ),
                        ),
                    ]))
                })
                .collect(),
        ),
    )]))
}

fn render_outcome(
    outcome: QuestionPromptOutcome,
    questions: &[String],
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    match outcome {
        QuestionPromptOutcome::Cancelled => bounded_output(Value::String(
            ASK_USER_QUESTION_CANCELLED_SENTINEL.to_owned(),
        )),
        QuestionPromptOutcome::Unavailable => bounded_output(Value::String(
            ASK_USER_QUESTION_UNAVAILABLE_SENTINEL.to_owned(),
        )),
        QuestionPromptOutcome::Answered(answers) => {
            if answers.len() != questions.len() {
                return Err(invalid_response());
            }
            let mut total_raw = 0_usize;
            let mut total_rendered = 0_usize;
            let mut content = Vec::with_capacity(answers.len());
            for (question, answer) in questions.iter().zip(answers.into_values()) {
                check_cancellation(cancellation)?;
                if answer.len() > MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES {
                    return Err(resource_limit());
                }
                total_raw = total_raw
                    .checked_add(answer.len())
                    .filter(|total| *total <= MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES)
                    .ok_or_else(resource_limit)?;
                let answer = trim_ascii_edges(&answer);
                if answer.is_empty() {
                    return Err(invalid_response());
                }
                let answer = encode_terminal_safe(answer);
                total_rendered = total_rendered
                    .checked_add(answer.len())
                    .filter(|total| *total <= MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES)
                    .ok_or_else(resource_limit)?;
                let mut pair = Map::new();
                pair.insert("answer".to_owned(), Value::String(answer));
                pair.insert("question".to_owned(), Value::String(question.clone()));
                content.push(Value::Object(pair));
            }
            check_cancellation(cancellation)?;
            bounded_output(Value::Array(content))
        }
    }
}

fn bounded_output(content: Value) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(content);
    let value = serde_json::to_value(&output).map_err(|_| resource_limit())?;
    if serialized_json_value_fits(&value, MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES) {
        Ok(output)
    } else {
        Err(resource_limit())
    }
}

fn trim_ascii_edges(text: &str) -> &str {
    text.trim_matches(|character| matches!(character, ' ' | '\t' | '\r' | '\n'))
}

fn encode_terminal_safe(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    for character in text.chars() {
        let code = u32::from(character);
        if code <= 0x1f || code == 0x7f {
            write!(&mut rendered, "\\x{code:02x}").expect("writing to a String cannot fail");
        } else if (0x80..=0x9f).contains(&code)
            || code == 0x061c
            || (0x200b..=0x200f).contains(&code)
            || (0x2028..=0x202e).contains(&code)
            || (0x2060..=0x206f).contains(&code)
            || code == 0xfeff
        {
            write!(&mut rendered, "\\u{{{code:04x}}}").expect("writing to a String cannot fail");
        } else {
            rendered.push(character);
        }
    }
    rendered
}

fn terminal_safe_preimage(rendered: &str, raw_limit: usize) -> Result<String, ToolError> {
    let mut raw_bytes = rendered.len();
    let mut costly_escapes = 0_usize;
    let mut index = 0_usize;
    while index < rendered.len() {
        if let Some((decoded, escape_bytes)) = terminal_escape_at(rendered, index) {
            if terminal_escape_can_be_decoded(rendered.len(), index, escape_bytes, decoded) {
                let raw_savings = escape_bytes
                    .checked_sub(decoded.len_utf8())
                    .ok_or_else(invalid_arguments)?;
                if serialized_json_character_size(decoded) <= escape_bytes + 1 {
                    raw_bytes = raw_bytes
                        .checked_sub(raw_savings)
                        .ok_or_else(invalid_arguments)?;
                } else {
                    costly_escapes = costly_escapes.checked_add(1).ok_or_else(resource_limit)?;
                }
            }
            index += escape_bytes;
        } else {
            index += rendered[index..]
                .chars()
                .next()
                .expect("a nonempty string suffix has one scalar")
                .len_utf8();
        }
    }

    let costly_needed = raw_bytes.saturating_sub(raw_limit).div_ceil(3);
    if costly_needed > costly_escapes {
        return Err(resource_limit());
    }

    let mut preimage = String::with_capacity(raw_bytes);
    let mut costly_decoded = 0_usize;
    index = 0;
    while index < rendered.len() {
        if let Some((decoded, escape_bytes)) = terminal_escape_at(rendered, index) {
            let decodable =
                terminal_escape_can_be_decoded(rendered.len(), index, escape_bytes, decoded);
            let beneficial = serialized_json_character_size(decoded) <= escape_bytes + 1;
            if decodable && (beneficial || costly_decoded < costly_needed) {
                preimage.push(decoded);
                if !beneficial {
                    costly_decoded += 1;
                }
            } else {
                preimage.push_str(&rendered[index..index + escape_bytes]);
            }
            index += escape_bytes;
        } else {
            let character = rendered[index..]
                .chars()
                .next()
                .expect("a nonempty string suffix has one scalar");
            preimage.push(character);
            index += character.len_utf8();
        }
    }

    if preimage.len() > raw_limit {
        return Err(resource_limit());
    }
    if trim_ascii_edges(&preimage) != preimage || encode_terminal_safe(&preimage) != rendered {
        return Err(invalid_arguments());
    }
    Ok(preimage)
}

fn terminal_escape_can_be_decoded(
    rendered_bytes: usize,
    index: usize,
    escape_bytes: usize,
    decoded: char,
) -> bool {
    !matches!(decoded, '\t' | '\r' | '\n') || (index != 0 && index + escape_bytes != rendered_bytes)
}

fn terminal_escape_at(rendered: &str, index: usize) -> Option<(char, usize)> {
    let bytes = rendered.as_bytes();
    if bytes.get(index..index + 2) == Some(b"\\x") {
        let high = lowercase_hex_digit(*bytes.get(index + 2)?)?;
        let low = lowercase_hex_digit(*bytes.get(index + 3)?)?;
        let code = u32::from(high) * 16 + u32::from(low);
        if code <= 0x1f || code == 0x7f {
            return char::from_u32(code).map(|character| (character, 4));
        }
    }
    if bytes.get(index..index + 3) == Some(b"\\u{") && bytes.get(index + 7) == Some(&b'}') {
        let mut code = 0_u32;
        for offset in 3..7 {
            code = code
                .checked_mul(16)?
                .checked_add(u32::from(lowercase_hex_digit(*bytes.get(index + offset)?)?))?;
        }
        if (0x80..=0x9f).contains(&code)
            || code == 0x061c
            || (0x200b..=0x200f).contains(&code)
            || (0x2028..=0x202e).contains(&code)
            || (0x2060..=0x206f).contains(&code)
            || code == 0xfeff
        {
            return char::from_u32(code).map(|character| (character, 8));
        }
    }
    None
}

fn lowercase_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn serialized_json_character_size(character: char) -> usize {
    match character {
        '"' | '\\' | '\u{0008}' | '\t' | '\n' | '\u{000c}' | '\r' => 2,
        '\u{0000}'..='\u{001f}' => 6,
        _ => character.len_utf8(),
    }
}

struct ActivePromptPermit {
    active_prompts: Option<Arc<AtomicUsize>>,
}

impl Drop for ActivePromptPermit {
    fn drop(&mut self) {
        if let Some(active_prompts) = self.active_prompts.take() {
            let previous = active_prompts.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0);
        }
    }
}

struct PromptActivity<'a> {
    prompt: Option<BoxFuture<'a, Result<QuestionPromptOutcome, QuestionPromptError>>>,
    cancellation_wait: Option<Cancelled>,
    // One registration is shared by the cancellation and prompt futures.
    // Equivalent outer Wakers retain the cached target without clone/drop
    // churn, while every supplied-Waker clone shares one callback lane.
    registered_waker: Option<ActivityWakerRegistration>,
    activity: Option<Arc<ActivePromptPermit>>,
}

impl<'a> PromptActivity<'a> {
    fn new(
        prompt: BoxFuture<'a, Result<QuestionPromptOutcome, QuestionPromptError>>,
        cancellation_wait: Cancelled,
        activity: Arc<ActivePromptPermit>,
    ) -> Self {
        let registered_waker = ActivityWakerRegistration::new(Arc::clone(&activity));
        Self {
            prompt: Some(prompt),
            cancellation_wait: Some(cancellation_wait),
            registered_waker: Some(registered_waker),
            activity: Some(activity),
        }
    }

    fn register_waker(&self, downstream: &Waker) {
        self.registered_waker
            .as_ref()
            .expect("prompt activity retains its Waker registration")
            .bind(downstream);
    }

    fn wake_delivery_resource_exhausted(&self) -> bool {
        self.registered_waker
            .as_ref()
            .expect("prompt activity retains its Waker registration")
            .wake_delivery_resource_exhausted()
    }

    fn poll_cancellation(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.register_waker(context.waker());
        let waker = self
            .registered_waker
            .as_ref()
            .expect("polling registers an activity-backed Waker")
            .waker();
        let mut activity_context = Context::from_waker(waker);
        Pin::new(
            self.cancellation_wait
                .as_mut()
                .expect("cancellation wait exists until activity teardown"),
        )
        .poll(&mut activity_context)
    }

    fn poll_prompt(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<QuestionPromptOutcome, QuestionPromptError>> {
        self.register_waker(context.waker());
        let waker = self
            .registered_waker
            .as_ref()
            .expect("polling registers an activity-backed Waker")
            .waker();
        let mut activity_context = Context::from_waker(waker);
        let polled = self
            .prompt
            .as_mut()
            .expect("prompt exists until it completes")
            .as_mut()
            .poll(&mut activity_context);
        if polled.is_ready() {
            self.registered_waker
                .as_ref()
                .expect("prompt activity retains its Waker registration")
                .close();
            let completed = self.prompt.take();
            drop(completed);
        }
        polled
    }
}

impl Drop for PromptActivity<'_> {
    fn drop(&mut self) {
        let preserve_existing_primary = std::thread::panicking();
        // A future may wake its retained supplied Waker from Drop. Close the
        // stale outer target first while the notifier still owns the permit.
        let registered_waker_close = catch_unwind(AssertUnwindSafe(|| {
            if let Some(registered_waker) = self.registered_waker.as_ref() {
                registered_waker.close();
            }
        }));
        let prompt = self.prompt.take();
        let prompt_drop = catch_unwind(AssertUnwindSafe(|| drop(prompt)));
        let cancellation_wait = self.cancellation_wait.take();
        let cancellation_wait_drop = catch_unwind(AssertUnwindSafe(|| drop(cancellation_wait)));
        let registered_waker = self.registered_waker.take();
        let registered_waker_drop = catch_unwind(AssertUnwindSafe(|| drop(registered_waker)));
        let activity = self.activity.take();
        drop(activity);
        // Preserve the established cleanup precedence. Every nonselected
        // opaque payload is forgotten before resuming the selected panic so a
        // payload destructor cannot replace it or abort an ambient unwind.
        settle_cleanup_panics(
            preserve_existing_primary,
            [
                prompt_drop,
                cancellation_wait_drop,
                registered_waker_close,
                registered_waker_drop,
            ],
        );
    }
}

struct ActivityWakerRegistration {
    wake: Option<Arc<ActivityWake>>,
    waker: Option<Waker>,
}

impl ActivityWakerRegistration {
    fn new(activity: Arc<ActivePromptPermit>) -> Self {
        let wake = Arc::new(ActivityWake::new(activity));
        Self {
            waker: Some(Waker::from(Arc::clone(&wake))),
            wake: Some(wake),
        }
    }

    fn bind(&self, downstream: &Waker) {
        self.wake
            .as_ref()
            .expect("activity Waker registration retains its notifier")
            .bind(downstream, self.waker());
    }

    fn close(&self) {
        self.wake
            .as_ref()
            .expect("activity Waker registration retains its notifier")
            .close();
    }

    fn wake_delivery_resource_exhausted(&self) -> bool {
        self.wake
            .as_ref()
            .expect("activity Waker registration retains its notifier")
            .delivery_resource_exhausted()
    }

    fn waker(&self) -> &Waker {
        self.waker
            .as_ref()
            .expect("activity Waker registration retains its Waker")
    }
}

impl Drop for ActivityWakerRegistration {
    fn drop(&mut self) {
        let preserve_existing_primary = std::thread::panicking();
        // Retain the notifier and activity across arbitrary target destruction,
        // including unwind, before releasing the supplied Waker and owner Arc.
        let close = catch_unwind(AssertUnwindSafe(|| {
            if let Some(wake) = self.wake.as_ref() {
                wake.close();
            }
        }));
        let waker = self.waker.take();
        let waker_drop = catch_unwind(AssertUnwindSafe(|| drop(waker)));
        let wake = self.wake.take();
        drop(wake);
        // Close retains precedence over cached-Waker destruction.
        settle_cleanup_panics(preserve_existing_primary, [close, waker_drop]);
    }
}

struct ActivityWake {
    state: Mutex<ActivityWakeState>,
}

#[derive(Default)]
struct ActivityWakeState {
    // A closed notifier identity may be retained indefinitely in an opaque
    // panic payload. Keeping the permit detachable in state lets close make
    // those retained identities inert without releasing an active callback.
    target: Option<Arc<Waker>>,
    activity: Option<Arc<ActivePromptPermit>>,
    delivery_callbacks_started: usize,
    notifying: bool,
    observed_while_notifying: bool,
    pending_after_observation: bool,
    lifecycle: ActivityWakeLifecycle,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum ActivityWakeLifecycle {
    #[default]
    Open,
    DeliveryResourceExhausted,
    Closed,
}

struct ActivityWakeCallback {
    target: Arc<Waker>,
    activity: Arc<ActivePromptPermit>,
}

impl ActivityWakeState {
    fn begin_callback(&mut self) -> Option<ActivityWakeCallback> {
        if !matches!(self.lifecycle, ActivityWakeLifecycle::Open) {
            return None;
        }
        if self.delivery_callbacks_started >= MAX_ACTIVITY_WAKE_CALLBACKS_PER_PROMPT {
            self.lifecycle = ActivityWakeLifecycle::DeliveryResourceExhausted;
            return None;
        }
        let target = self.target.as_ref().map(Arc::clone)?;
        let activity = self.activity.as_ref().map(Arc::clone)?;
        self.delivery_callbacks_started += 1;
        if self.delivery_callbacks_started == MAX_ACTIVITY_WAKE_CALLBACKS_PER_PROMPT {
            // Callback 256 is admitted only after exhaustion becomes sticky,
            // so it schedules the terminal poll and every later wake/bind is
            // suppressed for the remainder of this prompt activity.
            self.lifecycle = ActivityWakeLifecycle::DeliveryResourceExhausted;
        }
        Some(ActivityWakeCallback { target, activity })
    }

    fn recover_after_callback_failure(&mut self) {
        self.lifecycle =
            if self.delivery_callbacks_started >= MAX_ACTIVITY_WAKE_CALLBACKS_PER_PROMPT {
                ActivityWakeLifecycle::DeliveryResourceExhausted
            } else {
                ActivityWakeLifecycle::Open
            };
    }
}

impl ActivityWake {
    fn new(activity: Arc<ActivePromptPermit>) -> Self {
        Self {
            state: Mutex::new(ActivityWakeState {
                activity: Some(activity),
                ..ActivityWakeState::default()
            }),
        }
    }

    fn bind(&self, target: &Waker, notifier_waker: &Waker) {
        let notifier_target = target.will_wake(notifier_waker);
        {
            let mut state = lock_activity_wake(&self.state);
            if !matches!(state.lifecycle, ActivityWakeLifecycle::Open) {
                return;
            }
            if state.notifying {
                // This poll observes every notice that preceded the bind. A
                // later notice in the same callback window needs one replay.
                state.observed_while_notifying = true;
                state.pending_after_observation = false;
            }
            if notifier_target {
                // A prompt future may use the supplied Waker to re-poll the
                // outer task. Never replace the genuine target with this
                // notifier itself or create a cycle/recursive delivery.
                return;
            }
            if state
                .target
                .as_deref()
                .is_some_and(|existing| existing.will_wake(target))
            {
                return;
            }
        }

        // Arbitrary Waker cloning may execute foreign code. Do it outside the
        // lock, then recheck lifecycle and identity after reacquiring it.
        let incoming = Arc::new(target.clone());
        let (replaced, unused) = {
            let mut state = lock_activity_wake(&self.state);
            if matches!(state.lifecycle, ActivityWakeLifecycle::Open) {
                // Notification may have started while the arbitrary target
                // clone ran. This bind is still an observing outer poll.
                if state.notifying {
                    state.observed_while_notifying = true;
                    state.pending_after_observation = false;
                }
                if state
                    .target
                    .as_deref()
                    .is_some_and(|existing| existing.will_wake(target))
                {
                    (None, Some(incoming))
                } else {
                    (state.target.replace(incoming), None)
                }
            } else {
                (None, Some(incoming))
            }
        };
        // Arbitrary Waker destruction also remains outside the lock.
        drop(replaced);
        drop(unused);
    }

    fn close(&self) {
        let preserve_existing_primary = std::thread::panicking();
        let (target, activity) = {
            let mut state = lock_activity_wake(&self.state);
            state.lifecycle = ActivityWakeLifecycle::Closed;
            state.observed_while_notifying = false;
            state.pending_after_observation = false;
            (state.target.take(), state.activity.take())
        };
        // Arbitrary target destruction stays outside the lock. The detached
        // close guard retains capacity through it, but is released before any
        // selected panic resumes. Closed notifier identities retain no permit.
        let target_drop = catch_unwind(AssertUnwindSafe(|| drop(target)));
        drop(activity);
        settle_cleanup_panics(preserve_existing_primary, [target_drop]);
    }

    fn delivery_resource_exhausted(&self) -> bool {
        matches!(
            lock_activity_wake(&self.state).lifecycle,
            ActivityWakeLifecycle::DeliveryResourceExhausted
        )
    }

    fn notify(&self) {
        let mut callback = {
            let mut state = lock_activity_wake(&self.state);
            if !matches!(state.lifecycle, ActivityWakeLifecycle::Open) {
                return;
            }
            if state.notifying {
                // Notices before a re-poll are represented by the callback
                // already in flight. A notice after that poll must be replayed
                // once the callback returns or the wake could be lost.
                if state.observed_while_notifying {
                    state.pending_after_observation = true;
                }
                return;
            }
            let Some(callback) = state.begin_callback() else {
                return;
            };
            state.notifying = true;
            state.observed_while_notifying = false;
            state.pending_after_observation = false;
            callback
        };

        // This loop is an iterative trampoline within one activation. The
        // state-owned counter also bounds queue-only activations across the
        // complete prompt lifetime. Every ordinary replay is admitted only by
        // an outer bind/poll followed by a later source notification; the
        // final bounded callback makes the outer future observe a fixed
        // terminal delivery failure.
        loop {
            let ActivityWakeCallback { target, activity } = callback;
            let preserve_existing_primary = std::thread::panicking();
            let notified = catch_unwind(AssertUnwindSafe(|| target.wake_by_ref()));
            // Releasing a replaced callback target may run arbitrary Waker
            // destruction. Keep the callback lane occupied, retain the
            // activity, and let destructor-driven close or replacement settle
            // before deciding whether any replay is still authoritative.
            let target_drop = catch_unwind(AssertUnwindSafe(|| drop(target)));
            let callback_failed = notified.is_err();
            let target_drop_failed = target_drop.is_err();
            let next_callback = {
                let mut state = lock_activity_wake(&self.state);
                if matches!(state.lifecycle, ActivityWakeLifecycle::Closed) {
                    state.notifying = false;
                    state.observed_while_notifying = false;
                    state.pending_after_observation = false;
                    None
                } else if callback_failed || target_drop_failed {
                    state.recover_after_callback_failure();
                    state.notifying = false;
                    state.observed_while_notifying = false;
                    state.pending_after_observation = false;
                    None
                } else if matches!(
                    state.lifecycle,
                    ActivityWakeLifecycle::DeliveryResourceExhausted
                ) {
                    state.notifying = false;
                    state.observed_while_notifying = false;
                    state.pending_after_observation = false;
                    None
                } else if state.pending_after_observation {
                    if let Some(target) = state.begin_callback() {
                        state.observed_while_notifying = false;
                        state.pending_after_observation = false;
                        Some(target)
                    } else {
                        state.notifying = false;
                        state.observed_while_notifying = false;
                        state.pending_after_observation = false;
                        None
                    }
                } else {
                    state.notifying = false;
                    state.observed_while_notifying = false;
                    state.pending_after_observation = false;
                    None
                }
            };
            // A concurrent close may have detached every other permit guard.
            // Retain this callback's guard through callback return, arbitrary
            // target destruction, and lane settlement, then release it before
            // any selected panic resumes.
            drop(activity);
            // Callback failure precedes target destruction. During an ambient
            // unwind both are cleanup failures and neither may replace it.
            settle_cleanup_panics(preserve_existing_primary, [notified, target_drop]);
            let Some(next_callback) = next_callback else {
                return;
            };
            callback = next_callback;
        }
    }
}

impl Wake for ActivityWake {
    fn wake(self: Arc<Self>) {
        self.notify();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

fn lock_activity_wake(state: &Mutex<ActivityWakeState>) -> MutexGuard<'_, ActivityWakeState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn settle_cleanup_panics<const N: usize>(
    preserve_existing_primary: bool,
    cleanups: [std::thread::Result<()>; N],
) {
    let mut selected = None;
    for cleanup in cleanups {
        if let Err(payload) = cleanup {
            if preserve_existing_primary || selected.is_some() {
                // Opaque panic payload destruction is arbitrary foreign work.
                // On this exceptional suppression path, forgetting the
                // payload is the only total way to preserve panic precedence.
                std::mem::forget(payload);
            } else {
                selected = Some(payload);
            }
        }
    }
    if let Some(payload) = selected {
        resume_unwind(payload);
    }
}

struct JsonOwner(Option<Value>);

impl JsonOwner {
    fn new(value: Value) -> Self {
        Self(Some(value))
    }

    fn value(&self) -> &Value {
        self.0.as_ref().expect("JSON owner retains its value")
    }
}

impl Drop for JsonOwner {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            drop_json_value_iterative(value);
        }
    }
}

enum OwnedJsonChildren {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoValues),
}

impl Iterator for OwnedJsonChildren {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(values) => values.next(),
            Self::Object(values) => values.next(),
        }
    }
}

fn drop_json_value_iterative(root: Value) {
    let mut frames = Vec::<OwnedJsonChildren>::new();
    let mut current = Some(root);
    loop {
        if let Some(value) = current.take() {
            match value {
                Value::Array(values) => frames.push(OwnedJsonChildren::Array(values.into_iter())),
                Value::Object(values) => {
                    frames.push(OwnedJsonChildren::Object(values.into_values()));
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        loop {
            let Some(frame) = frames.last_mut() else {
                return;
            };
            if let Some(child) = frame.next() {
                current = Some(child);
                break;
            }
            frames.pop();
        }
    }
}

fn serialized_json_value_fits(root: &Value, limit: usize) -> bool {
    let mut total = 0_usize;
    let mut values = vec![root];
    while let Some(value) = values.pop() {
        match value {
            Value::Null | Value::Bool(true) => {
                if !add_serialized_size(&mut total, 4, limit) {
                    return false;
                }
            }
            Value::Bool(false) => {
                if !add_serialized_size(&mut total, 5, limit) {
                    return false;
                }
            }
            Value::Number(number) => {
                if !add_serialized_size(&mut total, number.to_string().len(), limit) {
                    return false;
                }
            }
            Value::String(string) => {
                let Some(remaining) = limit.checked_sub(total) else {
                    return false;
                };
                let Some(size) = serialized_json_string_size(string, remaining) else {
                    return false;
                };
                if !add_serialized_size(&mut total, size, limit) {
                    return false;
                }
            }
            Value::Array(items) => {
                let Some(structural) = 2_usize.checked_add(items.len().saturating_sub(1)) else {
                    return false;
                };
                if !add_serialized_size(&mut total, structural, limit) {
                    return false;
                }
                values.extend(items.iter());
            }
            Value::Object(object) => {
                let Some(structural) = 2_usize
                    .checked_add(object.len().saturating_sub(1))
                    .and_then(|size| size.checked_add(object.len()))
                else {
                    return false;
                };
                if !add_serialized_size(&mut total, structural, limit) {
                    return false;
                }
                for key in object.keys() {
                    let Some(remaining) = limit.checked_sub(total) else {
                        return false;
                    };
                    let Some(key_size) = serialized_json_string_size(key, remaining) else {
                        return false;
                    };
                    if !add_serialized_size(&mut total, key_size, limit) {
                        return false;
                    }
                }
                values.extend(object.values());
            }
        }
    }
    true
}

fn add_serialized_size(total: &mut usize, additional: usize, limit: usize) -> bool {
    let Some(next) = total.checked_add(additional) else {
        return false;
    };
    if next > limit {
        return false;
    }
    *total = next;
    true
}

fn serialized_json_string_size(value: &str, limit: usize) -> Option<usize> {
    if value.len() > limit.checked_sub(2)? {
        return None;
    }
    let mut size = 2_usize;
    for character in value.chars() {
        let additional = serialized_json_character_size(character);
        size = size.checked_add(additional)?;
        if size > limit {
            return None;
        }
    }
    Some(size)
}

fn ask_user_question_name() -> ToolName {
    ToolName::new(ASK_USER_QUESTION_TOOL_NAME)
        .expect("ask_user_question is a valid registered tool name")
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "ask_user_question_invalid_arguments",
        "ask_user_question arguments are invalid",
        false,
    )
}

fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    )
}

fn permission_request_unsupported() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "ask_user_question_permission_request_unsupported",
        "ask_user_question permission escalation is not supported",
        false,
    )
}

fn prompt_busy() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    )
}

fn prompt_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "ask_user_question_prompt_failed",
        "ask_user_question prompt failed",
        false,
    )
}

fn invalid_response() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "ask_user_question_invalid_response",
        "ask_user_question prompt returned an invalid response",
        false,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    )
}
