use std::ffi::OsString;
use std::io;

pub(crate) const MAX_ASK_PROMPT_BYTES: usize = 256 * 1024;
pub(crate) const ASK_OPERATIONAL_FAILURE: &str = "machine-god ask: request failed\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AskCommandOutcome {
    Completed,
    OperationalFailure,
    OutputFailure,
    Interrupted,
    Terminated,
}

impl AskCommandOutcome {
    const fn exit_code(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::OperationalFailure | Self::OutputFailure => 1,
            Self::Interrupted => 130,
            Self::Terminated => 143,
        }
    }
}

pub(crate) trait AskCommandHost {
    fn execute(&self, prompt: String, output: &mut dyn io::Write) -> AskCommandOutcome;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProductionAskCommandHost;

pub(crate) fn parse_prompt_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<String, ()> {
    let mut parts = Vec::new();
    let mut joined_len = 0usize;
    let mut recognizing_options = true;
    let mut has_visible_byte = false;

    for argument in arguments {
        let argument = argument.into_string().map_err(|_| ())?;
        if recognizing_options && parts.is_empty() && argument == "--" {
            recognizing_options = false;
            continue;
        }
        if recognizing_options && argument.starts_with('-') {
            return Err(());
        }
        if argument.as_bytes().contains(&0) {
            return Err(());
        }
        has_visible_byte |= argument
            .bytes()
            .any(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'));
        joined_len = joined_len
            .checked_add(usize::from(!parts.is_empty()))
            .and_then(|length| length.checked_add(argument.len()))
            .filter(|length| *length <= MAX_ASK_PROMPT_BYTES)
            .ok_or(())?;
        parts.push(argument);
    }

    if parts.is_empty() || !has_visible_byte {
        return Err(());
    }

    let mut prompt = String::with_capacity(joined_len);
    for (index, part) in parts.into_iter().enumerate() {
        if index != 0 {
            prompt.push(' ');
        }
        prompt.push_str(&part);
    }
    debug_assert_eq!(prompt.len(), joined_len);
    Ok(prompt)
}

pub(crate) fn run_ask(
    host: &impl AskCommandHost,
    prompt: String,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &'static str,
) -> u8 {
    let outcome = host.execute(prompt, stdout);
    let diagnostic = match outcome {
        AskCommandOutcome::OperationalFailure => Some(ASK_OPERATIONAL_FAILURE),
        AskCommandOutcome::OutputFailure => Some(output_failure),
        AskCommandOutcome::Completed
        | AskCommandOutcome::Interrupted
        | AskCommandOutcome::Terminated => None,
    };
    if let Some(diagnostic) = diagnostic {
        let _ = stderr.write_all(diagnostic.as_bytes());
    }
    outcome.exit_code()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod production {
    use std::future::poll_fn;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::Poll;

    use futures_core::Stream;
    use machine_god_core::{ModelEvent, Turn, TurnEvent};
    use machine_god_native::{
        AiGatewayCredentialEnvironment, NativeEnvironment, NativeReferenceHost,
        NativeRootSelection, PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
        PreparedNativeRoots, QuestionPromptError, QuestionPromptOutcome, QuestionPromptRequest,
        QuestionPrompter, TokioWebSearchDeadline, load_native_config,
    };

    use super::{AskCommandHost, AskCommandOutcome, ProductionAskCommandHost};

    #[derive(Clone, Copy, Debug, Default)]
    struct DenyPermissionPrompter;

    impl PermissionPrompter for DenyPermissionPrompter {
        fn prompt(
            &self,
            _request: machine_god_core::PermissionRequest,
        ) -> machine_god_core::BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>>
        {
            Box::pin(async { Ok(PermissionPromptDecision::Deny) })
        }
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct UnavailableQuestionPrompter;

    impl QuestionPrompter for UnavailableQuestionPrompter {
        fn prompt(
            &self,
            _request: QuestionPromptRequest,
        ) -> machine_god_core::BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>>
        {
            Box::pin(async { Ok(QuestionPromptOutcome::Unavailable) })
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AskSignal {
        Interrupt,
        Terminate,
    }

    struct AskSignals {
        interrupt: tokio::signal::unix::Signal,
        terminate: tokio::signal::unix::Signal,
    }

    impl AskSignals {
        fn register() -> Result<Self, ()> {
            let interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                    .map_err(|_| ())?;
            let terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .map_err(|_| ())?;
            Ok(Self {
                interrupt,
                terminate,
            })
        }

        fn poll_signal(&mut self, context: &mut std::task::Context<'_>) -> Poll<AskSignal> {
            if self.interrupt.poll_recv(context).is_ready() {
                return Poll::Ready(AskSignal::Interrupt);
            }
            if self.terminate.poll_recv(context).is_ready() {
                return Poll::Ready(AskSignal::Terminate);
            }
            Poll::Pending
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TurnEventDisposition {
        Continue,
        Completed,
        Failed,
    }

    fn present_turn_event(
        event: TurnEvent,
        output: &mut dyn std::io::Write,
    ) -> Result<TurnEventDisposition, ()> {
        match event {
            TurnEvent::Model {
                event: ModelEvent::TextDelta { text },
            } => output
                .write_all(text.as_bytes())
                .map(|()| TurnEventDisposition::Continue)
                .map_err(|_| ()),
            TurnEvent::Completed { .. } => Ok(TurnEventDisposition::Completed),
            TurnEvent::Failed { .. } => Ok(TurnEventDisposition::Failed),
            _ => Ok(TurnEventDisposition::Continue),
        }
    }

    impl AskCommandHost for ProductionAskCommandHost {
        fn execute(&self, prompt: String, output: &mut dyn std::io::Write) -> AskCommandOutcome {
            execute_production(prompt, output).unwrap_or(AskCommandOutcome::OperationalFailure)
        }
    }

    fn execute_production(
        prompt: String,
        output: &mut dyn std::io::Write,
    ) -> Result<AskCommandOutcome, ()> {
        let environment = NativeEnvironment::from_process();
        let loaded_config = load_native_config(&environment).map_err(|_| ())?;
        let selection = NativeRootSelection::from_current_process(&environment).map_err(|_| ())?;
        let prepared_roots = PreparedNativeRoots::prepare(selection).map_err(|_| ())?;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| ())?;
        runtime.block_on(async move {
            let mut signals = AskSignals::register()?;
            let host = NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots(
                loaded_config,
                AiGatewayCredentialEnvironment::from_process(),
                prepared_roots,
                Arc::new(DenyPermissionPrompter),
                Arc::new(UnavailableQuestionPrompter),
                Arc::new(TokioWebSearchDeadline::new()),
            )
            .map_err(|_| ())?;
            execute_turn(&host, prompt, output, &mut signals).await
        })
    }

    async fn execute_turn(
        host: &NativeReferenceHost,
        prompt: String,
        output: &mut dyn std::io::Write,
        signals: &mut AskSignals,
    ) -> Result<AskCommandOutcome, ()> {
        let session = host
            .session_lifecycle()
            .create_generated()
            .await
            .map_err(|_| ())?;
        let turn = session.prompt(prompt).await.map_err(|_| ())?;
        Ok(drive_turn(turn, signals, output).await)
    }

    async fn drive_turn(
        mut turn: Turn,
        signals: &mut AskSignals,
        output: &mut dyn std::io::Write,
    ) -> AskCommandOutcome {
        let mut requested_signal = None;
        let mut output_failed = false;
        let mut turn_failed = false;
        let handle = turn.handle();

        loop {
            let event = if requested_signal.is_none() && !output_failed {
                let mut next_event = None;
                let signal = poll_fn(|context| {
                    if let Poll::Ready(signal) = signals.poll_signal(context) {
                        return Poll::Ready(Some(signal));
                    }
                    match Pin::new(&mut turn).poll_next(context) {
                        Poll::Ready(event) => {
                            next_event = Some(event);
                            Poll::Ready(None)
                        }
                        Poll::Pending => Poll::Pending,
                    }
                })
                .await;
                if let Some(signal) = signal {
                    requested_signal = Some(signal);
                    let _ = handle.cancel();
                    continue;
                }
                next_event.expect("turn polling completed with an event")
            } else {
                poll_fn(|context| Pin::new(&mut turn).poll_next(context)).await
            };

            match event {
                Some(Ok(event)) => {
                    if output_failed || requested_signal.is_some() {
                        match event.payload {
                            TurnEvent::Completed { .. } => break,
                            TurnEvent::Failed { .. } => {
                                turn_failed = true;
                                break;
                            }
                            _ => {}
                        }
                    } else {
                        match present_turn_event(event.payload, output) {
                            Ok(TurnEventDisposition::Continue) => {}
                            Ok(TurnEventDisposition::Completed) => break,
                            Ok(TurnEventDisposition::Failed) => {
                                turn_failed = true;
                                break;
                            }
                            Err(()) => {
                                output_failed = true;
                                let _ = handle.cancel();
                            }
                        }
                    }
                }
                Some(Err(_)) => {
                    turn_failed = true;
                    let _ = handle.cancel();
                }
                None => {
                    turn_failed = true;
                    break;
                }
            }
        }

        final_outcome(output_failed, requested_signal, turn_failed)
    }

    const fn final_outcome(
        output_failed: bool,
        requested_signal: Option<AskSignal>,
        turn_failed: bool,
    ) -> AskCommandOutcome {
        if let Some(signal) = requested_signal {
            match signal {
                AskSignal::Interrupt => AskCommandOutcome::Interrupted,
                AskSignal::Terminate => AskCommandOutcome::Terminated,
            }
        } else if output_failed {
            AskCommandOutcome::OutputFailure
        } else if turn_failed {
            AskCommandOutcome::OperationalFailure
        } else {
            AskCommandOutcome::Completed
        }
    }

    #[cfg(test)]
    mod tests {
        use std::io;

        use machine_god_core::{ModelEvent, StopReason, TokenUsage, TurnEvent};

        use super::{
            AskCommandOutcome, AskSignal, TurnEventDisposition, final_outcome, present_turn_event,
        };

        #[test]
        fn presentation_writes_only_text_delta_bytes_unchanged() {
            let mut output = Vec::new();
            assert_eq!(
                present_turn_event(
                    TurnEvent::Model {
                        event: ModelEvent::ReasoningDelta {
                            text: "hidden-reasoning".to_owned(),
                        },
                    },
                    &mut output,
                ),
                Ok(TurnEventDisposition::Continue)
            );
            assert_eq!(
                present_turn_event(
                    TurnEvent::Model {
                        event: ModelEvent::TextDelta {
                            text: "a\0β".to_owned(),
                        },
                    },
                    &mut output,
                ),
                Ok(TurnEventDisposition::Continue)
            );
            assert_eq!(
                present_turn_event(TurnEvent::Started, &mut output),
                Ok(TurnEventDisposition::Continue)
            );
            assert_eq!(output, "a\0β".as_bytes());
        }

        #[test]
        fn presentation_classifies_terminal_events_without_rendering_them() {
            let mut output = Vec::new();
            assert_eq!(
                present_turn_event(
                    TurnEvent::Completed {
                        reason: StopReason::Completed,
                        usage: TokenUsage::default(),
                    },
                    &mut output,
                ),
                Ok(TurnEventDisposition::Completed)
            );
            assert_eq!(
                present_turn_event(
                    TurnEvent::Failed {
                        component: "provider-secret".to_owned(),
                        code: "secret-code".to_owned(),
                        message: "secret-message".to_owned(),
                        retryable: false,
                    },
                    &mut output,
                ),
                Ok(TurnEventDisposition::Failed)
            );
            assert!(output.is_empty());
        }

        struct FailingOutput;

        impl io::Write for FailingOutput {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        #[test]
        fn presentation_reports_output_failure_without_retaining_payload() {
            assert_eq!(
                present_turn_event(
                    TurnEvent::Model {
                        event: ModelEvent::TextDelta {
                            text: "provider-secret".to_owned(),
                        },
                    },
                    &mut FailingOutput,
                ),
                Err(())
            );
        }

        #[test]
        fn final_outcome_has_fixed_output_signal_and_failure_precedence() {
            assert_eq!(
                final_outcome(true, Some(AskSignal::Interrupt), true),
                AskCommandOutcome::Interrupted
            );
            assert_eq!(
                final_outcome(false, Some(AskSignal::Interrupt), true),
                AskCommandOutcome::Interrupted
            );
            assert_eq!(
                final_outcome(false, Some(AskSignal::Terminate), false),
                AskCommandOutcome::Terminated
            );
            assert_eq!(
                final_outcome(false, None, true),
                AskCommandOutcome::OperationalFailure
            );
            assert_eq!(
                final_outcome(false, None, false),
                AskCommandOutcome::Completed
            );
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl AskCommandHost for ProductionAskCommandHost {
    fn execute(&self, _prompt: String, _output: &mut dyn io::Write) -> AskCommandOutcome {
        AskCommandOutcome::OperationalFailure
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::io;

    use super::{
        ASK_OPERATIONAL_FAILURE, AskCommandHost, AskCommandOutcome, MAX_ASK_PROMPT_BYTES,
        parse_prompt_arguments, run_ask,
    };

    struct FakeAskHost {
        outcome: AskCommandOutcome,
        calls: Cell<usize>,
        prompts: RefCell<Vec<String>>,
        bytes: &'static [u8],
    }

    impl FakeAskHost {
        fn new(outcome: AskCommandOutcome, bytes: &'static [u8]) -> Self {
            Self {
                outcome,
                calls: Cell::new(0),
                prompts: RefCell::new(Vec::new()),
                bytes,
            }
        }
    }

    impl AskCommandHost for FakeAskHost {
        fn execute(&self, prompt: String, output: &mut dyn io::Write) -> AskCommandOutcome {
            self.calls.set(self.calls.get() + 1);
            self.prompts.borrow_mut().push(prompt);
            if output.write_all(self.bytes).is_err() {
                AskCommandOutcome::OutputFailure
            } else {
                self.outcome
            }
        }
    }

    #[test]
    fn prompt_parser_joins_unicode_parts_and_honors_delimiter() {
        assert_eq!(
            parse_prompt_arguments(["hello", "世界"].map(OsString::from)),
            Ok("hello 世界".to_owned())
        );
        assert_eq!(
            parse_prompt_arguments(["--", "before", "-after"].map(OsString::from)),
            Ok("before -after".to_owned())
        );
        assert_eq!(
            parse_prompt_arguments(["--", "--flag", "--"].map(OsString::from)),
            Ok("--flag --".to_owned())
        );
    }

    #[test]
    fn prompt_parser_rejects_missing_whitespace_nul_and_unsupported_flags() {
        for arguments in [
            vec![],
            vec![OsString::from("--")],
            vec![OsString::from(" \t\r\n")],
            vec![OsString::from("hello\0world")],
            vec![OsString::from("--json")],
            vec![OsString::from("hello"), OsString::from("-later")],
            vec![OsString::from("hello"), OsString::from("--")],
        ] {
            assert!(parse_prompt_arguments(arguments).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn prompt_parser_rejects_non_unicode() {
        use std::os::unix::ffi::OsStringExt as _;

        assert!(parse_prompt_arguments([OsString::from_vec(vec![0xff])]).is_err());
    }

    #[test]
    fn prompt_parser_checks_complete_join_bound_before_allocation() {
        let exact = "x".repeat(MAX_ASK_PROMPT_BYTES);
        assert_eq!(parse_prompt_arguments([exact.clone().into()]), Ok(exact));
        assert!(
            parse_prompt_arguments(["x".repeat(MAX_ASK_PROMPT_BYTES).into(), "y".into()]).is_err()
        );
    }

    #[test]
    fn runner_forwards_prompt_and_stream_bytes_unchanged() {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, "a\0β".as_bytes());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_ask(
                &host,
                "prompt".to_owned(),
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            0
        );
        assert_eq!(host.calls.get(), 1);
        assert_eq!(*host.prompts.borrow(), ["prompt"]);
        assert_eq!(stdout, "a\0β".as_bytes());
        assert!(stderr.is_empty());
    }

    #[test]
    fn runner_uses_fixed_operational_output_and_signal_exits() {
        for (outcome, exit, expected_stderr) in [
            (
                AskCommandOutcome::OperationalFailure,
                1,
                ASK_OPERATIONAL_FAILURE,
            ),
            (AskCommandOutcome::OutputFailure, 1, "output failed\n"),
            (AskCommandOutcome::Interrupted, 130, ""),
            (AskCommandOutcome::Terminated, 143, ""),
        ] {
            let host = FakeAskHost::new(outcome, b"");
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_ask(
                    &host,
                    "prompt".to_owned(),
                    &mut stdout,
                    &mut stderr,
                    "output failed\n",
                ),
                exit
            );
            assert_eq!(stderr, expected_stderr.as_bytes());
        }
    }

    #[derive(Default)]
    struct FailingOutput;

    impl io::Write for FailingOutput {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("injected output failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn output_failure_is_fixed_and_redacted() {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"provider-secret");
        let mut stdout = FailingOutput;
        let mut stderr = Vec::new();
        assert_eq!(
            run_ask(
                &host,
                "prompt-secret".to_owned(),
                &mut stdout,
                &mut stderr,
                "output failed\n",
            ),
            1
        );
        assert_eq!(stderr, b"output failed\n");
    }
}
