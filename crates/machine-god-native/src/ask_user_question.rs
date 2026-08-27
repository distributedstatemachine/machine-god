//! Bounded, rootless user-question tool over an explicitly injected host prompt.

use std::fmt::{self, Write as _};
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput, ToolSpec,
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

/// Structured outcome returned by a [`QuestionPrompter`].
#[derive(Clone, Eq, PartialEq)]
pub enum QuestionPromptOutcome {
    /// One ordered answer for every question.
    Answered(Vec<String>),
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

    fn try_acquire_prompt(&self) -> Option<ActivePromptPermit> {
        self.active_prompts
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.max_active_prompts).then(|| active + 1)
            })
            .ok()
            .map(|_| ActivePromptPermit {
                active_prompts: Some(Arc::clone(&self.active_prompts)),
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
            let prompt = match catch_unwind(AssertUnwindSafe(|| {
                self.prompter.prompt(normalized.request)
            })) {
                Ok(prompt) => prompt,
                Err(payload) => {
                    drop(permit);
                    resume_unwind(payload);
                }
            };
            let mut activity = Some(PromptActivity::new(prompt, permit));
            let mut cancellation_wait = cancellation.cancelled();
            let prompted = poll_fn(|context| {
                if cancellation.is_cancelled() {
                    return Poll::Ready(Err(cancelled()));
                }
                if Pin::new(&mut cancellation_wait).poll(context).is_ready() {
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
                        let _ = catch_unwind(AssertUnwindSafe(|| drop(doomed)));
                        resume_unwind(payload);
                    }
                }
            })
            .await;

            let prompt_result = match prompted {
                Ok(result) => result,
                Err(error) => {
                    drop(activity.take());
                    return Err(error);
                }
            };
            check_cancellation(&cancellation)?;
            let outcome = prompt_result.map_err(|_| prompt_failed())?;
            let output = render_outcome(outcome, &output_questions, &cancellation)?;
            check_cancellation(&cancellation)?;
            drop(activity.take());
            Ok(output)
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
        add_presentation_bytes(&mut rendered_presentation, question.len())?;

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
                .any(|existing| existing.label.eq_ignore_ascii_case(&label))
            {
                return Err(invalid_arguments());
            }
            add_presentation_bytes(&mut rendered_presentation, label.len())?;

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
                add_presentation_bytes(&mut rendered_presentation, description.len())?;
            }
            options.push(QuestionPromptOption { label, description });
        }
        questions.push(QuestionPrompt { question, options });
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
    Ok(NormalizedArguments {
        request,
        arguments: normalized_arguments,
    })
}

fn normalize_required_text(
    text: &str,
    phase: ArgumentPhase,
    raw_limit: usize,
    rendered_limit: usize,
) -> Result<String, ToolError> {
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
            Ok(rendered)
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
            Ok(rendered)
        }
    }
}

fn normalize_optional_text(
    text: &str,
    phase: ArgumentPhase,
    raw_limit: usize,
    rendered_limit: usize,
) -> Result<Option<String>, ToolError> {
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
            for (question, answer) in questions.iter().zip(answers) {
                check_cancellation(cancellation)?;
                let answer = trim_ascii_edges(&answer);
                if answer.is_empty() {
                    return Err(invalid_response());
                }
                if answer.len() > MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES {
                    return Err(resource_limit());
                }
                total_raw = total_raw
                    .checked_add(answer.len())
                    .filter(|total| *total <= MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES)
                    .ok_or_else(resource_limit)?;
                let answer = encode_terminal_safe(answer);
                total_rendered = total_rendered
                    .checked_add(answer.len())
                    .filter(|total| *total <= MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES)
                    .ok_or_else(resource_limit)?;
                let mut pair = Map::new();
                pair.insert("question".to_owned(), Value::String(question.clone()));
                pair.insert("answer".to_owned(), Value::String(answer));
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
    permit: Option<ActivePromptPermit>,
}

impl<'a> PromptActivity<'a> {
    fn new(
        prompt: BoxFuture<'a, Result<QuestionPromptOutcome, QuestionPromptError>>,
        permit: ActivePromptPermit,
    ) -> Self {
        Self {
            prompt: Some(prompt),
            permit: Some(permit),
        }
    }

    fn poll_prompt(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<QuestionPromptOutcome, QuestionPromptError>> {
        let polled = self
            .prompt
            .as_mut()
            .expect("prompt exists until it completes")
            .as_mut()
            .poll(context);
        if polled.is_ready() {
            let completed = self.prompt.take();
            drop(completed);
        }
        polled
    }
}

impl Drop for PromptActivity<'_> {
    fn drop(&mut self) {
        let prompt = self.prompt.take();
        let permit = self.permit.take();
        let prompt_drop = catch_unwind(AssertUnwindSafe(|| drop(prompt)));
        drop(permit);
        if let Err(payload) = prompt_drop
            && !std::thread::panicking()
        {
            resume_unwind(payload);
        }
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
                let Some(size) = serialized_json_string_size(string) else {
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
                    let Some(key_size) = serialized_json_string_size(key) else {
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

fn serialized_json_string_size(value: &str) -> Option<usize> {
    let mut size = 2_usize;
    for character in value.chars() {
        let additional = match character {
            '"' | '\\' | '\u{0008}' | '\t' | '\n' | '\u{000c}' | '\r' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            _ => character.len_utf8(),
        };
        size = size.checked_add(additional)?;
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
