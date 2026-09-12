//! Presentation state only. Native validates the exact queued request and owns effects.

use super::presentation::escaped;
use machine_god_native::NativeInteractivePromptResponse;
use machine_god_native::mcp::{
    interaction::{MAX_MCP_ELICITATION_ANSWER_BYTES as RESPONSE_BYTES, McpElicitationAnswerInput},
    mrtr::{
        McpElicitationMode, McpElicitationRequest, McpFormField, McpFormFieldKind, McpMrtrLimits,
    },
};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, fmt::Write};

#[cfg(test)]
mod tests;
pub(super) mod url;

// Each source page expands to at most 48 KiB after terminal-safe escaping.
const PAGE_BYTES: usize = 8 * 1024;
const STAGE_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub(super) struct ElicitationModal {
    // 0: request; 1..=fields: individual field; fields + 1: confirmation.
    stage: usize,
    page: usize,
    epoch: usize,
    answers: BTreeMap<String, Box<RawValue>>,
    answer_bytes: usize,
}

impl ElicitationModal {
    pub fn epoch(&self) -> usize {
        self.epoch
    }

    pub fn render(
        &self,
        request: &McpElicitationRequest,
        server: &str,
        tool: &str,
    ) -> Result<Vec<u8>, ()> {
        let source = self.stage_text(request)?;
        let (start, end) = page_span(&source, self.page).ok_or(())?;
        let mut output = super::bounded_output();
        write!(output, "\n[MCP input — page {}]\n", self.page + 1).map_err(|_| ())?;
        output.write_str("Server: ").map_err(|_| ())?;
        escaped(&mut output, server)?;
        output.write_str("\nTool: ").map_err(|_| ())?;
        escaped(&mut output, tool)?;
        output.write_char('\n').map_err(|_| ())?;
        escaped(&mut output, &source[start..end])?;
        output.write_char('\n').map_err(|_| ())?;
        if end < source.len() {
            output
                .write_str("/next to continue reading. ")
                .map_err(|_| ())?;
        } else {
            output
                .write_str(self.instructions(request)?)
                .map_err(|_| ())?;
        }
        if self.page > 0 {
            output.write_str(" /back to reread.").map_err(|_| ())?;
        }
        output
            .write_str(
                "\n/decline or /cancel-input dismisses this request; /cancel stops the turn.\n> ",
            )
            .map_err(|_| ())?;
        Ok(output.finish().into_bytes())
    }

    pub fn answer(
        &mut self,
        request: &McpElicitationRequest,
        line: &str,
    ) -> Result<Option<NativeInteractivePromptResponse>, ()> {
        let command = line.trim();
        if matches!(command, "/decline" | "/cancel-input") {
            return response(
                if command == "/decline" {
                    "decline"
                } else {
                    "cancel"
                },
                None,
            )
            .map(Some);
        }
        let source = self.stage_text(request)?;
        let (_, end) = page_span(&source, self.page).ok_or(())?;
        if command == "/back" && self.page > 0 {
            self.page -= 1;
            self.advance_epoch()?;
            return Ok(None);
        }
        if end < source.len() {
            if command != "/next" {
                return Err(());
            }
            self.page += 1;
            self.advance_epoch()?;
            return Ok(None);
        }
        if request.mode() == McpElicitationMode::Url {
            return if matches!(command, "y" | "yes") {
                response("accept", None).map(Some)
            } else {
                Err(())
            };
        }
        let form = request.form_schema().ok_or(())?;
        if self.stage == 0 {
            if command != "/next" {
                return Err(());
            }
        } else if let Some(field) = form.fields().get(self.stage - 1) {
            let value = field_answer(field, line)?;
            if let Some(value) = value {
                field
                    .validate_json(&value, McpMrtrLimits::default())
                    .map_err(|_| ())?;
                let key = serde_json::to_string(field.name()).map_err(|_| ())?;
                let charge = key
                    .len()
                    .checked_add(value.get().len())
                    .and_then(|n| n.checked_add(2))
                    .ok_or(())?;
                let total = self.answer_bytes.checked_add(charge).ok_or(())?;
                // Reserve the complete action/content wrapper before retaining a value.
                if total > RESPONSE_BYTES - 64 {
                    return Err(());
                }
                self.answers.insert(field.name().into(), value);
                self.answer_bytes = total;
            }
        } else {
            return if matches!(command, "y" | "yes") {
                response("accept", Some(&self.answers)).map(Some)
            } else {
                Err(())
            };
        }
        self.stage += 1;
        self.page = 0;
        self.advance_epoch()?;
        Ok(None)
    }

    fn advance_epoch(&mut self) -> Result<(), ()> {
        self.epoch = self.epoch.checked_add(1).ok_or(())?;
        Ok(())
    }

    fn instructions(&self, request: &McpElicitationRequest) -> Result<&'static str, ()> {
        if request.mode() == McpElicitationMode::Url {
            return Ok("[y] approve opening this URL. Approval is not browser completion.");
        }
        if self.stage == 0 {
            return Ok("/next to fill the form. Nothing is submitted until final confirmation.");
        }
        let fields = request.form_schema().ok_or(())?.fields();
        Ok(fields.get(self.stage - 1).map_or(
            "[y] send these answers to the MCP server.",
            |field| match field.kind() {
                McpFormFieldKind::String => "Enter text, 'text <literal>', or 'json <quoted string>'; /default uses the shown default, /skip omits an optional field.",
                McpFormFieldKind::Number | McpFormFieldKind::Integer => "Enter an exact JSON number; /default uses the shown default, /skip omits an optional field.",
                McpFormFieldKind::Boolean => "Enter true/false or y/n; /default uses the shown default, /skip omits an optional field.",
                McpFormFieldKind::SingleSelect => "Choose an option number; /default uses the shown default, /skip omits an optional field.",
                McpFormFieldKind::MultiSelect => "Choose comma-separated option numbers (empty means no selections); /default uses the shown default, /skip omits an optional field.",
            },
        ))
    }

    fn stage_text(&self, request: &McpElicitationRequest) -> Result<String, ()> {
        let mut text = crate::bounded_output::BoundedOutput::with_capacity(STAGE_BYTES, 1024);
        if self.stage == 0 {
            text.write_str(request.message()).map_err(|_| ())?;
            if request.mode() == McpElicitationMode::Url {
                text.write_str("\n\nURL: ").map_err(|_| ())?;
                text.write_str(request.url().ok_or(())?).map_err(|_| ())?;
                write!(
                    text,
                    "\nHost classification: {:?}\nOnly approve a destination you trust.",
                    request.host_classification()
                )
                .map_err(|_| ())?;
            }
        } else {
            let fields = request.form_schema().ok_or(())?.fields();
            if let Some(field) = fields.get(self.stage - 1) {
                writeln!(
                    text,
                    "Field {}/{}: {} ({})",
                    self.stage,
                    fields.len(),
                    field.name(),
                    if field.required() {
                        "required"
                    } else {
                        "optional"
                    }
                )
                .map_err(|_| ())?;
                // Exact schema includes title, description, constraints and defaults;
                // terminal escaping happens only after bounded source pagination.
                text.write_str(field.raw_json().get()).map_err(|_| ())?;
                for (index, choice) in field.choices().iter().enumerate() {
                    write!(text, "\n{}. {}", index + 1, choice.title()).map_err(|_| ())?;
                }
            } else {
                text.write_str("Review the complete answers before sending:\n")
                    .map_err(|_| ())?;
                let answers = serde_json::to_string(&self.answers).map_err(|_| ())?;
                text.write_str(&answers).map_err(|_| ())?;
            }
        }
        Ok(text.finish())
    }
}

fn page_span(text: &str, page: usize) -> Option<(usize, usize)> {
    let mut start = 0;
    for index in 0..=page {
        let mut end = text.len().min(start + PAGE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if index == page {
            return Some((start, end));
        }
        if end == text.len() {
            return None;
        }
        start = end;
    }
    None
}

fn field_answer(field: &McpFormField, line: &str) -> Result<Option<Box<RawValue>>, ()> {
    if line.len() > RESPONSE_BYTES {
        return Err(());
    }
    let command = line.trim();
    if command == "/skip" {
        return if field.required() { Err(()) } else { Ok(None) };
    }
    if command == "/default" {
        return field
            .default_json()
            .map(RawValue::to_owned)
            .map(Some)
            .ok_or(());
    }
    let raw = match field.kind() {
        McpFormFieldKind::String => {
            if let Some(raw) = line.strip_prefix("json ") {
                raw.to_owned()
            } else {
                serde_json::to_string(line.strip_prefix("text ").unwrap_or(line)).map_err(|_| ())?
            }
        }
        McpFormFieldKind::Number | McpFormFieldKind::Integer => command.to_owned(),
        McpFormFieldKind::Boolean => match command {
            "y" | "yes" | "true" => "true".into(),
            "n" | "no" | "false" => "false".into(),
            _ => return Err(()),
        },
        McpFormFieldKind::SingleSelect => {
            let index = command
                .parse::<usize>()
                .map_err(|_| ())?
                .checked_sub(1)
                .ok_or(())?;
            serde_json::to_string(field.choices().get(index).ok_or(())?.value()).map_err(|_| ())?
        }
        McpFormFieldKind::MultiSelect => {
            let mut selected = Vec::new();
            if !command.is_empty() {
                for token in command.split(',') {
                    if selected.len() >= field.choices().len() {
                        return Err(());
                    }
                    let index = token
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| ())?
                        .checked_sub(1)
                        .ok_or(())?;
                    let value = field.choices().get(index).ok_or(())?.value();
                    if selected.contains(&value) {
                        return Err(());
                    }
                    selected.push(value);
                }
            }
            serde_json::to_string(&selected).map_err(|_| ())?
        }
    };
    RawValue::from_string(raw).map(Some).map_err(|_| ())
}

fn response(
    action: &str,
    content: Option<&BTreeMap<String, Box<RawValue>>>,
) -> Result<NativeInteractivePromptResponse, ()> {
    let action = serde_json::to_string(action).map_err(|_| ())?;
    let raw = if let Some(content) = content {
        let content = serde_json::to_string(content).map_err(|_| ())?;
        format!("{{\"action\":{action},\"content\":{content}}}")
    } else {
        format!("{{\"action\":{action}}}")
    };
    if raw.len() > RESPONSE_BYTES {
        return Err(());
    }
    let raw = RawValue::from_string(raw).map_err(|_| ())?;
    McpElicitationAnswerInput::new(raw)
        .map(NativeInteractivePromptResponse::Elicitation)
        .map_err(|_| ())
}
