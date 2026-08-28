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
    use std::time::Duration;

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

    fn classify_turn_event(event: TurnEvent) -> (TurnEventDisposition, Option<Vec<u8>>) {
        match event {
            TurnEvent::Model {
                event: ModelEvent::TextDelta { text },
            } => (TurnEventDisposition::Continue, Some(text.into_bytes())),
            TurnEvent::Completed { .. } => (TurnEventDisposition::Completed, None),
            TurnEvent::Failed { .. } => (TurnEventDisposition::Failed, None),
            _ => (TurnEventDisposition::Continue, None),
        }
    }

    enum OutputWork {
        Write(Vec<u8>),
        Flush,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum OutputAcknowledgement {
        Succeeded,
        Failed,
    }

    const SIGNAL_OUTPUT_GRACE: Duration = Duration::from_millis(100);

    struct OutputBridge {
        work: tokio::sync::mpsc::Sender<OutputWork>,
        acknowledgements: tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TurnDriveResult {
        outcome: AskCommandOutcome,
        stalled_output_after_signal: bool,
    }

    #[derive(Default)]
    struct TurnDriveState {
        requested_signal: Option<AskSignal>,
        output_failed: bool,
        turn_failed: bool,
        stalled_output_after_signal: bool,
    }

    enum PollResult<T> {
        Signal(AskSignal),
        Value(T),
    }

    enum SignalGraceResult {
        Acknowledged(Option<OutputAcknowledgement>),
        TimedOut,
    }

    trait SignalSource {
        fn poll_signal(&mut self, context: &mut std::task::Context<'_>) -> Poll<AskSignal>;
    }

    impl SignalSource for AskSignals {
        fn poll_signal(&mut self, context: &mut std::task::Context<'_>) -> Poll<AskSignal> {
            Self::poll_signal(self, context)
        }
    }

    struct TurnEventStream<'a> {
        turn: &'a mut Turn,
    }

    impl Stream for TurnEventStream<'_> {
        type Item = Result<TurnEvent, ()>;

        fn poll_next(
            mut self: Pin<&mut Self>,
            context: &mut std::task::Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            Pin::new(&mut *self.turn)
                .poll_next(context)
                .map(|event| event.map(|event| event.map(|event| event.payload).map_err(|_| ())))
        }
    }

    fn serve_output(
        mut work: tokio::sync::mpsc::Receiver<OutputWork>,
        acknowledgements: &tokio::sync::mpsc::Sender<OutputAcknowledgement>,
        output: &mut dyn std::io::Write,
    ) {
        while let Some(work) = work.blocking_recv() {
            let succeeded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match work {
                OutputWork::Write(bytes) => output.write_all(&bytes),
                OutputWork::Flush => output.flush(),
            }))
            .is_ok_and(|result| result.is_ok());
            let acknowledgement = if succeeded {
                OutputAcknowledgement::Succeeded
            } else {
                OutputAcknowledgement::Failed
            };
            if acknowledgements.blocking_send(acknowledgement).is_err() {
                break;
            }
        }
    }

    async fn poll_next_or_signal<S, G>(
        stream: &mut S,
        signals: &mut G,
    ) -> PollResult<Option<Result<TurnEvent, ()>>>
    where
        S: Stream<Item = Result<TurnEvent, ()>> + Unpin,
        G: SignalSource,
    {
        poll_fn(|context| {
            if let Poll::Ready(signal) = signals.poll_signal(context) {
                return Poll::Ready(PollResult::Signal(signal));
            }
            Pin::new(&mut *stream)
                .poll_next(context)
                .map(PollResult::Value)
        })
        .await
    }

    async fn acknowledgement_finishes_within_signal_grace(
        acknowledgements: &mut tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
    ) -> SignalGraceResult {
        match tokio::time::timeout(SIGNAL_OUTPUT_GRACE, acknowledgements.recv()).await {
            Ok(acknowledgement) => SignalGraceResult::Acknowledged(acknowledgement),
            Err(_) => SignalGraceResult::TimedOut,
        }
    }

    async fn record_signal_grace(
        acknowledgements: &mut tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
        state: &mut TurnDriveState,
    ) {
        match acknowledgement_finishes_within_signal_grace(acknowledgements).await {
            SignalGraceResult::Acknowledged(Some(OutputAcknowledgement::Succeeded)) => {}
            SignalGraceResult::Acknowledged(Some(OutputAcknowledgement::Failed) | None) => {
                state.output_failed = true;
            }
            SignalGraceResult::TimedOut => state.stalled_output_after_signal = true,
        }
    }

    async fn poll_acknowledgement_or_signal<G: SignalSource>(
        acknowledgements: &mut tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
        signals: &mut G,
    ) -> PollResult<Option<OutputAcknowledgement>> {
        poll_fn(|context| {
            if let Poll::Ready(signal) = signals.poll_signal(context) {
                return Poll::Ready(PollResult::Signal(signal));
            }
            acknowledgements.poll_recv(context).map(PollResult::Value)
        })
        .await
    }

    async fn poll_signal_now<G: SignalSource>(signals: &mut G) -> Option<AskSignal> {
        poll_fn(|context| {
            Poll::Ready(match signals.poll_signal(context) {
                Poll::Ready(signal) => Some(signal),
                Poll::Pending => None,
            })
        })
        .await
    }

    async fn flush_terminal_output<G: SignalSource>(
        output: &mut OutputBridge,
        signals: &mut G,
        state: &mut TurnDriveState,
    ) {
        if state.stalled_output_after_signal || state.output_failed {
            return;
        }
        if output.work.try_send(OutputWork::Flush).is_err() {
            state.output_failed = true;
            return;
        }
        if state.requested_signal.is_some() {
            record_signal_grace(&mut output.acknowledgements, state).await;
            return;
        }
        match poll_acknowledgement_or_signal(&mut output.acknowledgements, signals).await {
            PollResult::Signal(signal) => {
                if state.requested_signal.is_none() {
                    state.requested_signal = Some(signal);
                }
                record_signal_grace(&mut output.acknowledgements, state).await;
            }
            PollResult::Value(Some(OutputAcknowledgement::Succeeded)) => {}
            PollResult::Value(Some(OutputAcknowledgement::Failed) | None) => {
                state.output_failed = true;
            }
        }
    }

    async fn drain_turn<S, G>(
        stream: &mut S,
        signals: &mut G,
        requested_signal: &mut Option<AskSignal>,
    ) where
        S: Stream<Item = Result<TurnEvent, ()>> + Unpin,
        G: SignalSource,
    {
        loop {
            match poll_next_or_signal(stream, signals).await {
                PollResult::Signal(signal) => {
                    if requested_signal.is_none() {
                        *requested_signal = Some(signal);
                    }
                }
                PollResult::Value(
                    Some(Ok(TurnEvent::Completed { .. } | TurnEvent::Failed { .. }) | Err(()))
                    | None,
                ) => break,
                PollResult::Value(Some(Ok(_))) => {}
            }
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

        std::thread::scope(|scope| {
            let (work_sender, work_receiver) = tokio::sync::mpsc::channel(1);
            let (acknowledgement_sender, acknowledgement_receiver) = tokio::sync::mpsc::channel(1);
            let worker = scope.spawn(move || {
                let (runtime, deadline) =
                    TokioWebSearchDeadline::build_runtime_pair().map_err(|_| ())?;
                runtime.block_on(async move {
                    let mut signals = AskSignals::register()?;
                    let host = NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots(
                        loaded_config,
                        AiGatewayCredentialEnvironment::from_process(),
                        prepared_roots,
                        Arc::new(DenyPermissionPrompter),
                        Arc::new(UnavailableQuestionPrompter),
                        Arc::new(deadline),
                    )
                    .map_err(|_| ())?;
                    let result = execute_turn(
                        &host,
                        prompt,
                        OutputBridge {
                            work: work_sender,
                            acknowledgements: acknowledgement_receiver,
                        },
                        &mut signals,
                    )
                    .await?;
                    if result.stalled_output_after_signal {
                        std::process::exit(i32::from(result.outcome.exit_code()));
                    }
                    Ok(result.outcome)
                })
            });

            serve_output(work_receiver, &acknowledgement_sender, output);
            worker.join().map_err(|_| ())?
        })
    }

    async fn execute_turn(
        host: &NativeReferenceHost,
        prompt: String,
        output: OutputBridge,
        signals: &mut AskSignals,
    ) -> Result<TurnDriveResult, ()> {
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
        output: OutputBridge,
    ) -> TurnDriveResult {
        let handle = turn.handle();
        let mut stream = TurnEventStream { turn: &mut turn };
        drive_turn_stream(
            &mut stream,
            || {
                let _ = handle.cancel();
            },
            signals,
            output,
        )
        .await
    }

    async fn drive_turn_stream<S, G, C>(
        stream: &mut S,
        cancel: C,
        signals: &mut G,
        mut output: OutputBridge,
    ) -> TurnDriveResult
    where
        S: Stream<Item = Result<TurnEvent, ()>> + Unpin,
        G: SignalSource,
        C: Fn(),
    {
        let mut state = TurnDriveState::default();

        loop {
            let event = match poll_next_or_signal(stream, signals).await {
                PollResult::Signal(signal) => {
                    state.requested_signal = Some(signal);
                    cancel();
                    drain_turn(stream, signals, &mut state.requested_signal).await;
                    break;
                }
                PollResult::Value(event) => event,
            };

            match event {
                Some(Ok(event)) => {
                    let (disposition, bytes) = classify_turn_event(event);
                    if let Some(bytes) = bytes {
                        if output.work.try_send(OutputWork::Write(bytes)).is_err() {
                            state.output_failed = true;
                            cancel();
                            drain_turn(stream, signals, &mut state.requested_signal).await;
                            break;
                        }
                        match poll_acknowledgement_or_signal(&mut output.acknowledgements, signals)
                            .await
                        {
                            PollResult::Signal(signal) => {
                                state.requested_signal = Some(signal);
                                cancel();
                                drain_turn(stream, signals, &mut state.requested_signal).await;
                                record_signal_grace(&mut output.acknowledgements, &mut state).await;
                                break;
                            }
                            PollResult::Value(Some(OutputAcknowledgement::Succeeded)) => {}
                            PollResult::Value(Some(OutputAcknowledgement::Failed) | None) => {
                                state.output_failed = true;
                                cancel();
                                drain_turn(stream, signals, &mut state.requested_signal).await;
                                break;
                            }
                        }
                    }
                    match disposition {
                        TurnEventDisposition::Continue => {}
                        TurnEventDisposition::Completed => {
                            break;
                        }
                        TurnEventDisposition::Failed => {
                            state.turn_failed = true;
                            break;
                        }
                    }
                }
                Some(Err(())) => {
                    state.turn_failed = true;
                    cancel();
                    drain_turn(stream, signals, &mut state.requested_signal).await;
                    break;
                }
                None => {
                    state.turn_failed = true;
                    break;
                }
            }
        }

        flush_terminal_output(&mut output, signals, &mut state).await;
        if state.requested_signal.is_none() {
            state.requested_signal = poll_signal_now(signals).await;
        }

        TurnDriveResult {
            outcome: final_outcome(
                state.output_failed,
                state.requested_signal,
                state.turn_failed,
            ),
            stalled_output_after_signal: state.stalled_output_after_signal,
        }
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
        use std::collections::VecDeque;
        use std::fs;
        use std::io;
        use std::path::{Path, PathBuf};
        use std::pin::Pin;
        use std::process::{Command, Stdio};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
        use std::task::{Context, Poll};
        use std::time::{Duration, Instant};

        use futures_core::Stream;
        use machine_god_core::{ModelEvent, StopReason, TokenUsage, TurnEvent};

        use super::{
            AskCommandOutcome, AskSignal, OutputBridge, SignalSource, TurnDriveResult,
            TurnEventDisposition, classify_turn_event, drive_turn_stream, final_outcome,
            serve_output,
        };

        const BLOCKED_OUTPUT_CHILD_MODE: &str = "MACHINE_GOD_ASK_BLOCKED_OUTPUT_CHILD";
        const BLOCKED_OUTPUT_READY_PATH: &str = "MACHINE_GOD_ASK_BLOCKED_OUTPUT_READY_PATH";
        const BLOCKED_OUTPUT_DRAINED_PATH: &str = "MACHINE_GOD_ASK_BLOCKED_OUTPUT_DRAINED_PATH";

        struct ScriptedTurn {
            events: VecDeque<Result<TurnEvent, ()>>,
            cancelled: Arc<AtomicBool>,
            drained: Arc<AtomicBool>,
            terminal_emitted: bool,
            signal_on_drain: Option<(Arc<AtomicU8>, AskSignal)>,
        }

        impl ScriptedTurn {
            fn new(events: impl IntoIterator<Item = TurnEvent>) -> Self {
                Self {
                    events: events.into_iter().map(Ok).collect(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                    drained: Arc::new(AtomicBool::new(false)),
                    terminal_emitted: false,
                    signal_on_drain: None,
                }
            }

            fn with_signal_on_drain(mut self, flag: Arc<AtomicU8>, signal: AskSignal) -> Self {
                self.signal_on_drain = Some((flag, signal));
                self
            }

            fn cancellation_flag(&self) -> Arc<AtomicBool> {
                Arc::clone(&self.cancelled)
            }

            fn drained_flag(&self) -> Arc<AtomicBool> {
                Arc::clone(&self.drained)
            }
        }

        impl Stream for ScriptedTurn {
            type Item = Result<TurnEvent, ()>;

            fn poll_next(
                mut self: Pin<&mut Self>,
                _context: &mut Context<'_>,
            ) -> Poll<Option<Self::Item>> {
                if self.cancelled.load(Ordering::Acquire) {
                    if self.terminal_emitted {
                        return Poll::Ready(None);
                    }
                    self.terminal_emitted = true;
                    if let Some((flag, signal)) = &self.signal_on_drain {
                        flag.store(signal_number(*signal), Ordering::Release);
                    }
                    self.drained.store(true, Ordering::Release);
                    return Poll::Ready(Some(Ok(completed_event(StopReason::Cancelled))));
                }
                self.events.pop_front().map_or(Poll::Pending, |event| {
                    if matches!(
                        &event,
                        Ok(TurnEvent::Completed { .. } | TurnEvent::Failed { .. })
                    ) {
                        self.drained.store(true, Ordering::Release);
                    }
                    Poll::Ready(Some(event))
                })
            }
        }

        #[derive(Default)]
        struct TestSignals {
            pending: Arc<AtomicU8>,
        }

        impl TestSignals {
            fn flag(&self) -> Arc<AtomicU8> {
                Arc::clone(&self.pending)
            }

            fn request(&self, signal: AskSignal) {
                self.pending.store(signal_number(signal), Ordering::Release);
            }
        }

        impl SignalSource for TestSignals {
            fn poll_signal(&mut self, _context: &mut Context<'_>) -> Poll<AskSignal> {
                match self.pending.swap(0, Ordering::AcqRel) {
                    1 => Poll::Ready(AskSignal::Interrupt),
                    2 => Poll::Ready(AskSignal::Terminate),
                    _ => Poll::Pending,
                }
            }
        }

        const fn signal_number(signal: AskSignal) -> u8 {
            match signal {
                AskSignal::Interrupt => 1,
                AskSignal::Terminate => 2,
            }
        }

        fn text_event(text: &str) -> TurnEvent {
            TurnEvent::Model {
                event: ModelEvent::TextDelta {
                    text: text.to_owned(),
                },
            }
        }

        fn completed_event(reason: StopReason) -> TurnEvent {
            TurnEvent::Completed {
                reason,
                usage: TokenUsage::default(),
            }
        }

        #[derive(Default)]
        struct RecordingOutput {
            bytes: Vec<u8>,
            flushes: usize,
            fail_write: bool,
            fail_flush: bool,
            panic_write: bool,
            signal_on_write: Option<(Arc<AtomicU8>, AskSignal)>,
            signal_on_flush: Option<(Arc<AtomicU8>, AskSignal)>,
        }

        impl io::Write for RecordingOutput {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if let Some((flag, signal)) = &self.signal_on_write {
                    flag.store(signal_number(*signal), Ordering::Release);
                }
                assert!(!self.panic_write, "injected output panic");
                if self.fail_write {
                    return Err(io::Error::other("injected write failure"));
                }
                self.bytes.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                if let Some((flag, signal)) = &self.signal_on_flush {
                    flag.store(signal_number(*signal), Ordering::Release);
                }
                if self.fail_flush {
                    Err(io::Error::other("injected flush failure"))
                } else {
                    Ok(())
                }
            }
        }

        fn run_script(
            mut turn: ScriptedTurn,
            mut signals: TestSignals,
            output: RecordingOutput,
        ) -> (TurnDriveResult, RecordingOutput) {
            let cancellation = turn.cancellation_flag();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("test runtime should build");
            std::thread::scope(|scope| {
                let (work_sender, work_receiver) = tokio::sync::mpsc::channel(1);
                let (acknowledgement_sender, acknowledgement_receiver) =
                    tokio::sync::mpsc::channel(1);
                let output_worker = scope.spawn(move || {
                    let mut output = output;
                    serve_output(work_receiver, &acknowledgement_sender, &mut output);
                    output
                });
                let result = runtime.block_on(drive_turn_stream(
                    &mut turn,
                    || cancellation.store(true, Ordering::Release),
                    &mut signals,
                    OutputBridge {
                        work: work_sender,
                        acknowledgements: acknowledgement_receiver,
                    },
                ));
                let output = output_worker.join().expect("output worker should join");
                (result, output)
            })
        }

        #[test]
        fn presentation_writes_only_text_delta_bytes_unchanged() {
            assert_eq!(
                classify_turn_event(TurnEvent::Model {
                    event: ModelEvent::ReasoningDelta {
                        text: "hidden-reasoning".to_owned(),
                    },
                }),
                (TurnEventDisposition::Continue, None)
            );
            assert_eq!(
                classify_turn_event(TurnEvent::Model {
                    event: ModelEvent::TextDelta {
                        text: "a\0β".to_owned(),
                    },
                }),
                (
                    TurnEventDisposition::Continue,
                    Some("a\0β".as_bytes().to_vec())
                )
            );
            assert_eq!(
                classify_turn_event(TurnEvent::Started),
                (TurnEventDisposition::Continue, None)
            );
        }

        #[test]
        fn presentation_classifies_terminal_events_without_rendering_them() {
            assert_eq!(
                classify_turn_event(TurnEvent::Completed {
                    reason: StopReason::Completed,
                    usage: TokenUsage::default(),
                }),
                (TurnEventDisposition::Completed, None)
            );
            assert_eq!(
                classify_turn_event(TurnEvent::Failed {
                    component: "provider-secret".to_owned(),
                    code: "secret-code".to_owned(),
                    message: "secret-message".to_owned(),
                    retryable: false,
                }),
                (TurnEventDisposition::Failed, None)
            );
        }

        #[test]
        fn completed_output_preserves_bytes_and_flushes_once() {
            let turn = ScriptedTurn::new([
                TurnEvent::Model {
                    event: ModelEvent::ReasoningDelta {
                        text: "hidden".to_owned(),
                    },
                },
                text_event("a\0β"),
                completed_event(StopReason::Completed),
            ]);
            let (result, output) =
                run_script(turn, TestSignals::default(), RecordingOutput::default());

            assert_eq!(
                result,
                TurnDriveResult {
                    outcome: AskCommandOutcome::Completed,
                    stalled_output_after_signal: false,
                }
            );
            assert_eq!(output.bytes, "a\0β".as_bytes());
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn write_failure_cancels_and_drains_before_output_failure() {
            let turn = ScriptedTurn::new([text_event("provider-secret")]);
            let cancelled = turn.cancellation_flag();
            let drained = turn.drained_flag();
            let output = RecordingOutput {
                fail_write: true,
                ..RecordingOutput::default()
            };

            let (result, output) = run_script(turn, TestSignals::default(), output);

            assert_eq!(result.outcome, AskCommandOutcome::OutputFailure);
            assert!(!result.stalled_output_after_signal);
            assert!(cancelled.load(Ordering::Acquire));
            assert!(drained.load(Ordering::Acquire));
            assert!(output.bytes.is_empty());
            assert_eq!(output.flushes, 0);
        }

        #[test]
        fn flush_failure_maps_to_output_failure() {
            let turn = ScriptedTurn::new([completed_event(StopReason::Completed)]);
            let output = RecordingOutput {
                fail_flush: true,
                ..RecordingOutput::default()
            };

            let (result, output) = run_script(turn, TestSignals::default(), output);

            assert_eq!(result.outcome, AskCommandOutcome::OutputFailure);
            assert!(!result.stalled_output_after_signal);
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn operational_failure_flushes_acknowledged_partial_bytes() {
            let turn = ScriptedTurn::new([
                text_event("partial-without-newline"),
                TurnEvent::Failed {
                    component: "provider-secret".to_owned(),
                    code: "secret-code".to_owned(),
                    message: "secret-message".to_owned(),
                    retryable: false,
                },
            ]);

            let (result, output) =
                run_script(turn, TestSignals::default(), RecordingOutput::default());

            assert_eq!(result.outcome, AskCommandOutcome::OperationalFailure);
            assert_eq!(output.bytes, b"partial-without-newline");
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn signal_during_failed_flush_keeps_signal_precedence() {
            let turn = ScriptedTurn::new([completed_event(StopReason::Completed)]);
            let signals = TestSignals::default();
            let output = RecordingOutput {
                fail_flush: true,
                signal_on_flush: Some((signals.flag(), AskSignal::Terminate)),
                ..RecordingOutput::default()
            };

            let (result, output) = run_script(turn, signals, output);

            assert_eq!(result.outcome, AskCommandOutcome::Terminated);
            assert!(!result.stalled_output_after_signal);
            assert_eq!(output.flushes, 1);
        }

        #[test]
        fn writer_panic_is_caught_then_cancelled_and_drained() {
            let turn = ScriptedTurn::new([text_event("provider-secret")]);
            let cancelled = turn.cancellation_flag();
            let drained = turn.drained_flag();
            let output = RecordingOutput {
                panic_write: true,
                ..RecordingOutput::default()
            };

            let (result, _output) = run_script(turn, TestSignals::default(), output);

            assert_eq!(result.outcome, AskCommandOutcome::OutputFailure);
            assert!(cancelled.load(Ordering::Acquire));
            assert!(drained.load(Ordering::Acquire));
        }

        #[test]
        fn signal_observed_with_output_error_wins_after_terminal_drain() {
            let turn = ScriptedTurn::new([text_event("provider-secret")]);
            let cancelled = turn.cancellation_flag();
            let drained = turn.drained_flag();
            let signals = TestSignals::default();
            let output = RecordingOutput {
                fail_write: true,
                signal_on_write: Some((signals.flag(), AskSignal::Interrupt)),
                ..RecordingOutput::default()
            };

            let (result, _output) = run_script(turn, signals, output);

            assert_eq!(result.outcome, AskCommandOutcome::Interrupted);
            assert!(!result.stalled_output_after_signal);
            assert!(cancelled.load(Ordering::Acquire));
            assert!(drained.load(Ordering::Acquire));
        }

        #[test]
        fn signal_raised_by_terminal_drain_wins_over_output_error() {
            let signals = TestSignals::default();
            let turn = ScriptedTurn::new([text_event("provider-secret")])
                .with_signal_on_drain(signals.flag(), AskSignal::Terminate);
            let drained = turn.drained_flag();
            let output = RecordingOutput {
                fail_write: true,
                ..RecordingOutput::default()
            };

            let (result, output) = run_script(turn, signals, output);

            assert_eq!(result.outcome, AskCommandOutcome::Terminated);
            assert!(!result.stalled_output_after_signal);
            assert!(drained.load(Ordering::Acquire));
            assert_eq!(output.flushes, 0);
        }

        #[test]
        fn idle_output_signal_returns_normally_after_terminal_drain() {
            let turn = ScriptedTurn::new([]);
            let cancelled = turn.cancellation_flag();
            let drained = turn.drained_flag();
            let signals = TestSignals::default();
            signals.request(AskSignal::Terminate);

            let (result, output) = run_script(turn, signals, RecordingOutput::default());

            assert_eq!(result.outcome, AskCommandOutcome::Terminated);
            assert!(!result.stalled_output_after_signal);
            assert!(cancelled.load(Ordering::Acquire));
            assert!(drained.load(Ordering::Acquire));
            assert!(output.bytes.is_empty());
            assert_eq!(output.flushes, 1);
        }

        struct PermanentlyBlockedOutput {
            ready_path: PathBuf,
            block_on_flush: bool,
        }

        impl io::Write for PermanentlyBlockedOutput {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if self.block_on_flush {
                    return Ok(buffer.len());
                }
                fs::write(&self.ready_path, b"write-entered")?;
                loop {
                    std::thread::park();
                }
            }

            fn flush(&mut self) -> io::Result<()> {
                if !self.block_on_flush {
                    return Ok(());
                }
                fs::write(&self.ready_path, b"flush-entered")?;
                loop {
                    std::thread::park();
                }
            }
        }

        #[test]
        #[ignore = "subprocess helper invoked by blocked_output_signals_exit_after_terminal_drain"]
        fn permanently_blocked_output_signal_child() {
            let Ok(mode) = std::env::var(BLOCKED_OUTPUT_CHILD_MODE) else {
                return;
            };
            let (signal, block_on_flush, ready_before_signal) = match mode.as_str() {
                "interrupt-write" => (AskSignal::Interrupt, false, false),
                "terminate-write" => (AskSignal::Terminate, false, false),
                "interrupt-flush" => (AskSignal::Interrupt, true, false),
                "interrupt-preflush" => (AskSignal::Interrupt, true, true),
                _ => panic!("unsupported child signal mode"),
            };
            let ready_path = PathBuf::from(
                std::env::var_os(BLOCKED_OUTPUT_READY_PATH).expect("ready path should be set"),
            );
            let drained_path = PathBuf::from(
                std::env::var_os(BLOCKED_OUTPUT_DRAINED_PATH).expect("drained path should be set"),
            );
            let turn = if block_on_flush && !ready_before_signal {
                ScriptedTurn::new([completed_event(StopReason::Completed)])
            } else {
                ScriptedTurn::new(if ready_before_signal {
                    None
                } else {
                    Some(text_event("blocks"))
                })
            };
            let cancellation = turn.cancellation_flag();
            let drained = turn.drained_flag();
            let listener_ready_path = ready_path.clone();

            std::thread::scope(|scope| {
                let (work_sender, work_receiver) = tokio::sync::mpsc::channel(1);
                let (acknowledgement_sender, acknowledgement_receiver) =
                    tokio::sync::mpsc::channel(1);
                let worker = scope.spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
                        .build()
                        .expect("child runtime should build");
                    let result = runtime.block_on(async move {
                        let mut signals = super::AskSignals::register()
                            .expect("child signal listeners should register");
                        if ready_before_signal {
                            fs::write(&listener_ready_path, b"signal-listeners-ready")
                                .expect("listener marker should be writable");
                        }
                        let mut turn = turn;
                        drive_turn_stream(
                            &mut turn,
                            || cancellation.store(true, Ordering::Release),
                            &mut signals,
                            OutputBridge {
                                work: work_sender,
                                acknowledgements: acknowledgement_receiver,
                            },
                        )
                        .await
                    });
                    assert_eq!(result.outcome, signal.into());
                    assert!(result.stalled_output_after_signal);
                    assert!(drained.load(Ordering::Acquire));
                    fs::write(drained_path, b"terminal-drained")
                        .expect("drained marker should be writable");
                    std::process::exit(i32::from(result.outcome.exit_code()));
                });

                let mut output = PermanentlyBlockedOutput {
                    ready_path,
                    block_on_flush,
                };
                serve_output(work_receiver, &acknowledgement_sender, &mut output);
                worker.join().expect("child worker should join");
            });
        }

        impl From<AskSignal> for AskCommandOutcome {
            fn from(signal: AskSignal) -> Self {
                match signal {
                    AskSignal::Interrupt => Self::Interrupted,
                    AskSignal::Terminate => Self::Terminated,
                }
            }
        }

        fn wait_for_marker(child: &mut std::process::Child, path: &Path) {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if path.exists() {
                    return;
                }
                if let Some(status) = child.try_wait().expect("child status should be readable") {
                    panic!("blocked-output child exited early with {status}");
                }
                assert!(
                    Instant::now() < deadline,
                    "blocked-output child did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn wait_for_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().expect("child status should be readable") {
                    return status;
                }
                if Instant::now() >= deadline {
                    child.kill().expect("stuck child should be killable");
                    panic!("blocked-output child did not exit after signal");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn blocked_output_signals_exit_after_terminal_drain() {
            for (mode, kill_signal, expected_exit) in [
                ("interrupt-write", "-INT", 130),
                ("terminate-write", "-TERM", 143),
                ("interrupt-flush", "-INT", 130),
                ("interrupt-preflush", "-INT", 130),
            ] {
                let prefix = format!("machine-god-ask-{}-{mode}", std::process::id());
                let ready_path = std::env::temp_dir().join(format!("{prefix}-ready"));
                let drained_path = std::env::temp_dir().join(format!("{prefix}-drained"));
                let _ = fs::remove_file(&ready_path);
                let _ = fs::remove_file(&drained_path);
                let mut child = Command::new(
                    std::env::current_exe().expect("current test executable should resolve"),
                )
                .args([
                    "--exact",
                    "ask::production::tests::permanently_blocked_output_signal_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env(BLOCKED_OUTPUT_CHILD_MODE, mode)
                .env(BLOCKED_OUTPUT_READY_PATH, &ready_path)
                .env(BLOCKED_OUTPUT_DRAINED_PATH, &drained_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("blocked-output child should start");

                wait_for_marker(&mut child, &ready_path);
                let kill_status = Command::new("kill")
                    .args([kill_signal, &child.id().to_string()])
                    .status()
                    .expect("signal command should run");
                assert!(kill_status.success());
                let status = wait_for_exit(&mut child);

                assert_eq!(status.code(), Some(expected_exit));
                assert_eq!(
                    fs::read(&drained_path).expect("drained marker should exist"),
                    b"terminal-drained"
                );
                fs::remove_file(ready_path).expect("ready marker should be removable");
                fs::remove_file(drained_path).expect("drained marker should be removable");
            }
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
