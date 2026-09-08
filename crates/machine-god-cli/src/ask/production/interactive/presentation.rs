use super::input_lines::InputBinding;
use machine_god_core::Capability;
use machine_god_native::{
    MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, NativeInteractivePromptResponse,
    NativeInteractivePromptView, PermissionPromptDecision, QuestionPromptAnswers,
    QuestionPromptOutcome,
};
use std::fmt::Write;

pub(super) struct Modal {
    pub view: NativeInteractivePromptView,
    pub displayed: bool,
    answers: QuestionPromptAnswers,
    answer_bytes: usize,
}

impl Modal {
    pub fn new(view: NativeInteractivePromptView) -> Self {
        Self {
            view,
            displayed: false,
            answers: QuestionPromptAnswers::new(),
            answer_bytes: 0,
        }
    }
    pub fn binding(&self) -> InputBinding {
        if self.displayed {
            self.presentation_binding()
        } else {
            InputBinding::AwaitingPrompt
        }
    }
    pub fn presentation_binding(&self) -> InputBinding {
        InputBinding::Prompt {
            token: self.view.token().clone(),
            question: self.answers.len(),
        }
    }
    pub fn render(&self) -> Result<Vec<u8>, ()> {
        let mut text = crate::BoundedModelsOutput::new();
        if let Some(request) = self.view.permission() {
            text.write_str("\n[permission] ").map_err(|_| ())?;
            escaped(&mut text, &request.reason)?;
            text.write_char('\n').map_err(|_| ())?;
            render_capability(&mut text, &request.capability)?;
            text.write_str("\n[y] allow once  [t] allow turn  [s] allow session  [n] deny\n> ")
                .map_err(|_| ())?;
        } else {
            let (_, request) = self.view.question().ok_or(())?;
            let question = request.questions().get(self.answers.len()).ok_or(())?;
            write!(
                text,
                "\n[question {}/{}] ",
                self.answers.len() + 1,
                request.questions().len()
            )
            .map_err(|_| ())?;
            escaped(&mut text, question.question())?;
            text.write_char('\n').map_err(|_| ())?;
            for (index, option) in question.options().iter().enumerate() {
                write!(text, "  {}. ", index + 1).map_err(|_| ())?;
                escaped(&mut text, option.label())?;
                if let Some(description) = option.description() {
                    text.write_str(" — ").map_err(|_| ())?;
                    escaped(&mut text, description)?;
                }
                text.write_char('\n').map_err(|_| ())?;
            }
            text.write_str("Choose a number, type 'other <answer>', or /cancel.\n> ")
                .map_err(|_| ())?;
        }
        Ok(text.finish().into_bytes())
    }

    /// Each question page is a separate presentation epoch even though the
    /// native question batch has one token. Pasted old-page lines cannot answer
    /// a newly displayed page.
    pub fn answer(
        &mut self,
        line: &str,
        binding: &InputBinding,
    ) -> Result<Option<NativeInteractivePromptResponse>, ()> {
        if !self.displayed || binding != &self.binding() {
            return Err(());
        }
        let line = line.trim();
        if self.view.permission().is_some() {
            let decision = match line {
                "y" | "yes" => PermissionPromptDecision::AllowOnce,
                "t" => PermissionPromptDecision::AllowTurn,
                "s" => PermissionPromptDecision::AllowSession,
                "n" | "no" | "/cancel" => PermissionPromptDecision::Deny,
                _ => return Err(()),
            };
            return Ok(Some(NativeInteractivePromptResponse::Permission(decision)));
        }
        if line == "/cancel" {
            return Ok(Some(NativeInteractivePromptResponse::Question(
                QuestionPromptOutcome::Cancelled,
            )));
        }
        let (_, request) = self.view.question().ok_or(())?;
        let question = request.questions().get(self.answers.len()).ok_or(())?;
        let answer = if let Some(answer) = line.strip_prefix("other ") {
            answer.trim()
        } else {
            let index = line
                .parse::<usize>()
                .map_err(|_| ())?
                .checked_sub(1)
                .ok_or(())?;
            question.options().get(index).ok_or(())?.label()
        };
        let bytes = self.answer_bytes.checked_add(answer.len()).ok_or(())?;
        if answer.is_empty() || bytes > MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES {
            return Err(());
        }
        self.answers.try_push(answer.to_owned()).map_err(|_| ())?;
        self.answer_bytes = bytes;
        self.displayed = false;
        if self.answers.len() == request.questions().len() {
            Ok(Some(NativeInteractivePromptResponse::Question(
                QuestionPromptOutcome::Answered(std::mem::take(&mut self.answers)),
            )))
        } else {
            Ok(None)
        }
    }
}

pub(super) fn escaped(text: &mut crate::BoundedModelsOutput, value: &str) -> Result<(), ()> {
    crate::write_json_string_content(text, value).map_err(|_| ())
}

fn json(text: &mut crate::BoundedModelsOutput, value: &serde_json::Value) -> Result<(), ()> {
    struct BoundedJson<'a>(&'a mut crate::BoundedModelsOutput);
    impl std::io::Write for BoundedJson<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let value = std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
            self.0.write_str(value).map_err(std::io::Error::other)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // The bridge has already bounded depth/nodes. A second escaping pass keeps
    // Unicode formatting controls inert, including those inside JSON strings.
    let mut raw = crate::BoundedModelsOutput::new();
    serde_json::to_writer(BoundedJson(&mut raw), value).map_err(|_| ())?;
    escaped(text, &raw.finish())
}

fn render_capability(
    text: &mut crate::BoundedModelsOutput,
    capability: &Capability,
) -> Result<(), ()> {
    match capability {
        Capability::Tool {
            name, arguments, ..
        } => {
            escaped(text, name.as_str())?;
            text.write_char(' ').map_err(|_| ())?;
            json(text, arguments)
        }
        Capability::Filesystem { access, path } => {
            write!(text, "{access:?}: ").map_err(|_| ())?;
            escaped(text, path)
        }
        Capability::FilesystemRename { old_path, new_path } => {
            text.write_str("rename ").map_err(|_| ())?;
            escaped(text, old_path)?;
            text.write_str(" -> ").map_err(|_| ())?;
            escaped(text, new_path)
        }
        Capability::FilesystemCopy {
            source,
            destination,
        } => {
            text.write_str("copy ").map_err(|_| ())?;
            escaped(text, source)?;
            text.write_str(" -> ").map_err(|_| ())?;
            escaped(text, destination)
        }
        Capability::OpenFile { path } => escaped(text, path),
        Capability::Process {
            program,
            arguments,
            working_directory,
            ..
        } => {
            text.write_char('"').map_err(|_| ())?;
            escaped(text, program)?;
            text.write_char('"').map_err(|_| ())?;
            for argument in arguments {
                text.write_str(" \"").map_err(|_| ())?;
                escaped(text, argument)?;
                text.write_char('"').map_err(|_| ())?;
            }
            text.write_str("\ncwd: ").map_err(|_| ())?;
            escaped(text, working_directory)?;
            text.write_str("\nInherited environment values are not displayed.")
                .map_err(|_| ())
        }
        Capability::Network { target } => {
            escaped(text, &target.scheme)?;
            text.write_str("://").map_err(|_| ())?;
            escaped(text, &target.host)?;
            if let Some(port) = target.port {
                write!(text, ":{port}").map_err(|_| ())?;
            }
            Ok(())
        }
        Capability::Vision { paths, target } => {
            for path in paths {
                escaped(text, path)?;
                text.write_char('\n').map_err(|_| ())?;
            }
            escaped(text, &target.host)
        }
        Capability::Custom { name, details } => {
            escaped(text, name)?;
            text.write_char(' ').map_err(|_| ())?;
            json(text, details)
        }
        _ => Err(()),
    }
}
