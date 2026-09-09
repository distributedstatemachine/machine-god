use super::{NativeInteractivePromptError as Error, NativeInteractivePromptResponse as Response};
use crate::{
    MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES, MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES,
    QuestionPromptOutcome, QuestionPromptRequest,
};
use machine_god_core::{BackgroundOutputOwner, Capability, PermissionRequest, ToolContext};
use serde_json::Value;
use std::io::{self, Write};

pub(super) enum Payload {
    Permission {
        request: PermissionRequest,
        rule: Option<Box<crate::NativePermissionRulePrompt>>,
    },
    Question {
        context: ToolContext,
        request: QuestionPromptRequest,
    },
}

impl Payload {
    pub fn belongs_to(&self, owner: &BackgroundOutputOwner) -> bool {
        let (session, incarnation) = match self {
            Self::Permission { request, .. } => {
                (&request.session_id, &request.session_incarnation_id)
            }
            Self::Question { context, .. } => {
                (&context.session_id, &context.session_incarnation_id)
            }
        };
        session == owner.session_id() && incarnation == owner.session_incarnation_id()
    }

    pub fn bytes(&self, limit: usize) -> Result<usize, Error> {
        let mut budget = Budget { bytes: 0, limit };
        match self {
            Self::Permission { request, rule } => {
                // Canonical identity plus fixed digest/weak-owner bookkeeping.
                if rule.is_some() {
                    budget.add(crate::MAX_NATIVE_PERMISSION_IDENTITY_BYTES + 256)?;
                }
                if let Some(value) = capability_value(&request.capability) {
                    check_json(value)?;
                }
                serde_json::to_writer(&mut budget, request).map_err(|_| Error::Limit)?;
            }
            Self::Question { context, request } => {
                for text in [
                    context.session_id.as_str(),
                    context.session_incarnation_id.as_str(),
                    context.turn_id.as_str(),
                    context.call_id.as_str(),
                ] {
                    budget.add(text.len())?;
                }
                for question in request.questions() {
                    budget.add(question.question().len())?;
                    for option in question.options() {
                        budget.add(option.label().len())?;
                        if let Some(text) = option.description() {
                            budget.add(text.len())?;
                        }
                    }
                }
            }
        }
        Ok(budget.bytes)
    }

    pub fn validate_response(&self, response: &Response) -> Result<(), Error> {
        match (self, response) {
            (
                Self::Question { request, .. },
                Response::Question(QuestionPromptOutcome::Answered(answers)),
            ) => {
                if answers.len() != request.questions().len() {
                    return Err(Error::InvalidResponse);
                }
                let mut bytes = 0_usize;
                for answer in answers.iter() {
                    if answer.len() > MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES {
                        return Err(Error::Limit);
                    }
                    bytes = bytes.checked_add(answer.len()).ok_or(Error::Limit)?;
                    if bytes > MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES {
                        return Err(Error::Limit);
                    }
                    if answer.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
                        return Err(Error::InvalidResponse);
                    }
                }
                Ok(())
            }
            (Self::Permission { .. }, Response::Permission(_))
            | (
                Self::Question { .. },
                Response::Question(
                    QuestionPromptOutcome::Cancelled | QuestionPromptOutcome::Unavailable,
                ),
            ) => Ok(()),
            _ => Err(Error::InvalidResponse),
        }
    }
}

fn capability_value(capability: &Capability) -> Option<&Value> {
    match capability {
        Capability::Tool { arguments, .. } => Some(arguments),
        Capability::Custom { details, .. } => Some(details),
        _ => None,
    }
}

struct Budget {
    bytes: usize,
    limit: usize,
}
impl Budget {
    fn add(&mut self, bytes: usize) -> Result<(), Error> {
        let total = self.bytes.checked_add(bytes).ok_or(Error::Limit)?;
        if total > self.limit {
            return Err(Error::Limit);
        }
        self.bytes = total;
        Ok(())
    }
}
impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.add(bytes.len())
            .map_err(|_| io::Error::other("prompt limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_json(root: &Value) -> Result<(), Error> {
    let mut stack = vec![(root, 0_usize)];
    let mut nodes = 0_usize;
    while let Some((value, depth)) = stack.pop() {
        nodes = nodes.checked_add(1).ok_or(Error::Limit)?;
        if nodes > 65_536 || depth > 64 {
            return Err(Error::Limit);
        }
        match value {
            Value::Array(values) => {
                if depth >= 64 {
                    return Err(Error::Limit);
                }
                if values.len() > 65_536 - nodes || stack.len() > 65_536 - nodes - values.len() {
                    return Err(Error::Limit);
                }
                stack.extend(values.iter().map(|child| (child, depth + 1)));
            }
            Value::Object(values) => {
                if depth >= 64 {
                    return Err(Error::Limit);
                }
                if values.len() > 65_536 - nodes || stack.len() > 65_536 - nodes - values.len() {
                    return Err(Error::Limit);
                }
                stack.extend(values.values().map(|child| (child, depth + 1)));
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    Ok(())
}

impl Drop for Payload {
    fn drop(&mut self) {
        let Self::Permission { request, .. } = self else {
            return;
        };
        let value = match &mut request.capability {
            Capability::Tool { arguments, .. } => std::mem::take(arguments),
            Capability::Custom { details, .. } => std::mem::take(details),
            _ => return,
        };
        // Even unpolled or rejected direct host inputs have iterative cleanup.
        drop_json(value);
    }
}

enum Children {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoValues),
}
impl Children {
    fn next(&mut self) -> Option<Value> {
        match self {
            Self::Array(values) => values.next(),
            Self::Object(values) => values.next(),
        }
    }
}
fn drop_json(root: Value) {
    let mut frames = Vec::<Children>::new();
    let mut current = Some(root);
    loop {
        if let Some(value) = current.take() {
            match value {
                Value::Array(values) => frames.push(Children::Array(values.into_iter())),
                Value::Object(values) => frames.push(Children::Object(values.into_values())),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        loop {
            let Some(frame) = frames.last_mut() else {
                return;
            };
            if let Some(value) = frame.next() {
                current = Some(value);
                break;
            }
            frames.pop();
        }
    }
}
