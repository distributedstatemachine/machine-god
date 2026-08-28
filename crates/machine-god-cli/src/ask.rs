use std::ffi::OsString;
use std::io;

pub(crate) const MAX_ASK_PROMPT_BYTES: usize = 256 * 1024;
pub(crate) const ASK_OPERATIONAL_FAILURE: &str = "machine-god ask: request failed\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AskCommandOutcome {
    Completed,
    OperationalFailure,
    OutputFailure,
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "signals are supported only by the production Unix ask host"
        )
    )]
    Interrupted,
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "signals are supported only by the production Unix ask host"
        )
    )]
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
    fn execute(&self, prompt: String, output: &mut dyn io::Write) -> AskCommandExecution;
}

trait AskCommandFinalizer {
    fn finish(self: Box<Self>, exit_code: u8) -> !;
}

pub(crate) struct AskCommandExecution {
    outcome: AskCommandOutcome,
    finalizer: Option<Box<dyn AskCommandFinalizer>>,
}

impl AskCommandExecution {
    pub(crate) const fn without_finalizer(outcome: AskCommandOutcome) -> Self {
        Self {
            outcome,
            finalizer: None,
        }
    }

    fn with_finalizer(
        outcome: AskCommandOutcome,
        finalizer: impl AskCommandFinalizer + 'static,
    ) -> Self {
        Self {
            outcome,
            finalizer: Some(Box::new(finalizer)),
        }
    }

    const fn outcome(&self) -> AskCommandOutcome {
        self.outcome
    }

    fn finish(mut self) -> u8 {
        let exit_code = self.outcome.exit_code();
        if let Some(finalizer) = self.finalizer.take() {
            finalizer.finish(exit_code);
        }
        exit_code
    }
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
    let execution = host.execute(prompt, stdout);
    let outcome = execution.outcome();
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
    execution.finish()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod production {
    use std::future::poll_fn;
    use std::pin::Pin;
    use std::sync::{Arc, mpsc};
    use std::task::Poll;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use futures_core::Stream;
    use machine_god_core::{ModelEvent, Turn, TurnEvent};
    use machine_god_native::{
        AiGatewayCredentialEnvironment, NativeEnvironment, NativeReferenceHost,
        NativeRootSelection, PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
        PreparedNativeRoots, QuestionPromptError, QuestionPromptOutcome, QuestionPromptRequest,
        QuestionPrompter, TokioWebSearchDeadline, load_native_config,
    };

    use super::{
        AskCommandExecution, AskCommandFinalizer, AskCommandHost, AskCommandOutcome,
        ProductionAskCommandHost,
    };

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
        ControlFailed,
    }

    impl AskSignal {
        const fn outcome(self) -> AskCommandOutcome {
            match self {
                Self::Interrupt => AskCommandOutcome::Interrupted,
                Self::Terminate => AskCommandOutcome::Terminated,
                Self::ControlFailed => AskCommandOutcome::OperationalFailure,
            }
        }

        const fn exit_code(self) -> i32 {
            self.outcome().exit_code() as i32
        }
    }

    struct AskSignals {
        receiver: tokio::sync::mpsc::Receiver<AskSignal>,
    }

    impl AskSignals {
        const fn new(receiver: tokio::sync::mpsc::Receiver<AskSignal>) -> Self {
            Self { receiver }
        }

        fn poll_signal(&mut self, context: &mut std::task::Context<'_>) -> Poll<AskSignal> {
            self.receiver
                .poll_recv(context)
                .map(|signal| signal.unwrap_or(AskSignal::ControlFailed))
        }
    }

    enum AskSignalControl {
        ActivateTurn(mpsc::SyncSender<()>),
        EnterFinal(mpsc::SyncSender<()>),
        Finish(u8),
    }

    #[derive(Clone)]
    struct AskSignalControlSender {
        sender: tokio::sync::mpsc::Sender<AskSignalControl>,
    }

    impl AskSignalControlSender {
        fn transition(
            &self,
            command: impl FnOnce(mpsc::SyncSender<()>) -> AskSignalControl,
        ) -> Result<(), ()> {
            let (ready, receiver) = mpsc::sync_channel(1);
            self.sender.try_send(command(ready)).map_err(|_| ())?;
            receiver.recv().map_err(|_| ())
        }

        fn activate_turn(&self) -> Result<(), ()> {
            self.transition(AskSignalControl::ActivateTurn)
        }

        fn enter_final(&self) -> Result<(), ()> {
            self.transition(AskSignalControl::EnterFinal)
        }

        async fn finish(&self, exit_code: u8) -> Result<(), ()> {
            self.sender
                .send(AskSignalControl::Finish(exit_code))
                .await
                .map_err(|_| ())
        }
    }

    struct AskSignalController {
        control: AskSignalControlSender,
        signals: Option<AskSignals>,
        worker: Option<JoinHandle<()>>,
    }

    fn map_thread_spawn<T>(result: std::io::Result<T>) -> Result<T, ()> {
        result.map_err(|_| ())
    }

    impl AskSignalController {
        fn spawn() -> Result<Self, ()> {
            let (control_sender, control_receiver) = tokio::sync::mpsc::channel(1);
            let (signal_sender, signal_receiver) = tokio::sync::mpsc::channel(1);
            let (ready_sender, ready_receiver) = mpsc::sync_channel(0);
            let worker = map_thread_spawn(
                std::thread::Builder::new()
                    .name("machine-god-ask-signals".to_owned())
                    .spawn(move || {
                        run_signal_guardian(control_receiver, &signal_sender, &ready_sender);
                    }),
            )?;
            if let Ok(Ok(())) = ready_receiver.recv() {
                Ok(Self {
                    control: AskSignalControlSender {
                        sender: control_sender,
                    },
                    signals: Some(AskSignals::new(signal_receiver)),
                    worker: Some(worker),
                })
            } else {
                let _ = worker.join();
                Err(())
            }
        }

        fn control(&self) -> AskSignalControlSender {
            self.control.clone()
        }

        fn take_signals(&mut self) -> Result<AskSignals, ()> {
            self.signals.take().ok_or(())
        }

        fn enter_final(&self) -> Result<(), ()> {
            self.control.enter_final()
        }
    }

    impl AskCommandFinalizer for AskSignalController {
        fn finish(mut self: Box<Self>, exit_code: u8) -> ! {
            if self
                .control
                .sender
                .blocking_send(AskSignalControl::Finish(exit_code))
                .is_err()
            {
                std::process::exit(1);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            std::process::exit(1)
        }
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum AskSignalPhase {
        Setup,
        Turn,
        Final,
    }

    enum AskSignalGuardianResult {
        Exit(i32),
        ControlClosed,
    }

    fn run_signal_guardian(
        mut control: tokio::sync::mpsc::Receiver<AskSignalControl>,
        turn_signals: &tokio::sync::mpsc::Sender<AskSignal>,
        ready: &mpsc::SyncSender<Result<(), ()>>,
    ) {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
        else {
            let _ = ready.send(Err(()));
            return;
        };
        let (mut interrupt, mut terminate) = {
            let _entered = runtime.enter();
            let interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());
            let terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
            if let (Ok(interrupt), Ok(terminate)) = (interrupt, terminate) {
                (interrupt, terminate)
            } else {
                let _ = ready.send(Err(()));
                return;
            }
        };
        if ready.send(Ok(())).is_err() {
            return;
        }
        let mut phase = AskSignalPhase::Setup;
        let mut finishing: Option<(i32, Pin<Box<tokio::time::Sleep>>)> = None;
        let result = runtime.block_on(poll_fn(|context| {
            let signal = match interrupt.poll_recv(context) {
                Poll::Ready(Some(())) => Some(AskSignal::Interrupt),
                Poll::Ready(None) => {
                    return Poll::Ready(AskSignalGuardianResult::Exit(1));
                }
                Poll::Pending => match terminate.poll_recv(context) {
                    Poll::Ready(Some(())) => Some(AskSignal::Terminate),
                    Poll::Ready(None) => {
                        return Poll::Ready(AskSignalGuardianResult::Exit(1));
                    }
                    Poll::Pending => None,
                },
            };
            if let Some(signal) = signal
                && (phase != AskSignalPhase::Turn || turn_signals.try_send(signal).is_err())
            {
                return Poll::Ready(AskSignalGuardianResult::Exit(signal.exit_code()));
            }

            if let Some((exit_code, drain)) = finishing.as_mut() {
                return drain
                    .as_mut()
                    .poll(context)
                    .map(|()| AskSignalGuardianResult::Exit(*exit_code));
            }

            match control.poll_recv(context) {
                Poll::Ready(Some(AskSignalControl::ActivateTurn(ready))) => {
                    phase = AskSignalPhase::Turn;
                    if ready.send(()).is_err() {
                        return Poll::Ready(AskSignalGuardianResult::ControlClosed);
                    }
                    context.waker().wake_by_ref();
                }
                Poll::Ready(Some(AskSignalControl::EnterFinal(ready))) => {
                    phase = AskSignalPhase::Final;
                    if ready.send(()).is_err() {
                        return Poll::Ready(AskSignalGuardianResult::ControlClosed);
                    }
                    context.waker().wake_by_ref();
                }
                Poll::Ready(Some(AskSignalControl::Finish(exit_code))) => {
                    phase = AskSignalPhase::Final;
                    finishing = Some((
                        i32::from(exit_code),
                        Box::pin(tokio::time::sleep(Duration::from_millis(1))),
                    ));
                    context.waker().wake_by_ref();
                }
                Poll::Ready(None) => {
                    return Poll::Ready(AskSignalGuardianResult::ControlClosed);
                }
                Poll::Pending => {}
            }
            Poll::Pending
        }));
        if let AskSignalGuardianResult::Exit(exit_code) = result {
            std::process::exit(exit_code);
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
        signal_output_deadline: Option<tokio::time::Instant>,
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
            let succeeded = match work {
                OutputWork::Write(bytes) => output.write_all(&bytes),
                OutputWork::Flush => output.flush(),
            }
            .is_ok();
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
        deadline: tokio::time::Instant,
    ) -> SignalGraceResult {
        match tokio::time::timeout_at(deadline, acknowledgements.recv()).await {
            Ok(acknowledgement) => SignalGraceResult::Acknowledged(acknowledgement),
            Err(_) => SignalGraceResult::TimedOut,
        }
    }

    async fn record_signal_grace(
        acknowledgements: &mut tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
        state: &mut TurnDriveState,
    ) {
        let deadline = *state
            .signal_output_deadline
            .get_or_insert_with(|| tokio::time::Instant::now() + SIGNAL_OUTPUT_GRACE);
        match acknowledgement_finishes_within_signal_grace(acknowledgements, deadline).await {
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
        fn execute(&self, prompt: String, output: &mut dyn std::io::Write) -> AskCommandExecution {
            let Ok(controller) = AskSignalController::spawn() else {
                return AskCommandExecution::without_finalizer(
                    AskCommandOutcome::OperationalFailure,
                );
            };
            let (outcome, controller) = execute_production(prompt, output, controller);
            AskCommandExecution::with_finalizer(outcome, controller)
        }
    }

    fn execute_production(
        prompt: String,
        output: &mut dyn std::io::Write,
        mut controller: AskSignalController,
    ) -> (AskCommandOutcome, AskSignalController) {
        let control = controller.control();
        let Ok(signals) = controller.take_signals() else {
            return (AskCommandOutcome::OperationalFailure, controller);
        };
        let result = std::thread::scope(|scope| {
            let (work_sender, work_receiver) = tokio::sync::mpsc::channel(1);
            let (acknowledgement_sender, acknowledgement_receiver) = tokio::sync::mpsc::channel(1);
            let worker = map_thread_spawn(
                std::thread::Builder::new()
                    .name("machine-god-ask-turn".to_owned())
                    .spawn_scoped(scope, move || {
                        let environment = NativeEnvironment::from_process();
                        let loaded_config = load_native_config(&environment).map_err(|_| ())?;
                        let selection = NativeRootSelection::from_current_process(&environment)
                            .map_err(|_| ())?;
                        let prepared_roots =
                            PreparedNativeRoots::prepare(selection).map_err(|_| ())?;
                        let (runtime, deadline) =
                            TokioWebSearchDeadline::build_runtime_pair().map_err(|_| ())?;
                        let host =
                            NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots(
                                loaded_config,
                                AiGatewayCredentialEnvironment::from_process(),
                                prepared_roots,
                                Arc::new(DenyPermissionPrompter),
                                Arc::new(UnavailableQuestionPrompter),
                                Arc::new(deadline),
                            )
                            .map_err(|_| ())?;
                        runtime.block_on(execute_turn(
                            &host,
                            prompt,
                            OutputBridge {
                                work: work_sender,
                                acknowledgements: acknowledgement_receiver,
                            },
                            signals,
                            &control,
                        ))
                    }),
            )?;

            serve_output(work_receiver, &acknowledgement_sender, output);
            worker.join().map_err(|_| ())?
        });
        if let Ok(outcome) = result {
            (outcome, controller)
        } else {
            let _ = controller.enter_final();
            (AskCommandOutcome::OperationalFailure, controller)
        }
    }

    async fn execute_turn(
        host: &NativeReferenceHost,
        prompt: String,
        output: OutputBridge,
        mut signals: AskSignals,
        control: &AskSignalControlSender,
    ) -> Result<AskCommandOutcome, ()> {
        let session = host
            .session_lifecycle()
            .create_generated()
            .await
            .map_err(|_| ())?;
        let turn = session.prompt(prompt).await.map_err(|_| ())?;
        control.activate_turn()?;
        let mut result = drive_turn(turn, &mut signals, output).await;
        control.enter_final()?;
        if !matches!(
            result.outcome,
            AskCommandOutcome::Interrupted | AskCommandOutcome::Terminated
        ) && let Ok(signal) = signals.receiver.try_recv()
        {
            result.outcome = signal.outcome();
        }
        if result.stalled_output_after_signal {
            control.finish(result.outcome.exit_code()).await?;
        }
        Ok(result.outcome)
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
                AskSignal::ControlFailed => AskCommandOutcome::OperationalFailure,
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
        use std::process::{Child, Command, Stdio};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
        use std::task::{Context, Poll};
        use std::time::{Duration, Instant};

        use futures_core::Stream;
        use machine_god_core::{
            ModelEvent, SessionId, SessionIncarnationId, SessionRecord, SessionStore, StopReason,
            TokenUsage, TurnEvent,
        };
        use machine_god_native::FileSessionStore;

        use super::super::{AskCommandExecution, AskCommandHost, run_ask};
        use super::{
            AskCommandOutcome, AskSignal, AskSignalController, OutputBridge, SignalSource,
            TurnDriveResult, TurnEventDisposition, classify_turn_event, drive_turn_stream,
            final_outcome, map_thread_spawn, serve_output,
        };

        const BLOCKED_OUTPUT_CHILD_MODE: &str = "MACHINE_GOD_ASK_BLOCKED_OUTPUT_CHILD";
        const BLOCKED_OUTPUT_READY_PATH: &str = "MACHINE_GOD_ASK_BLOCKED_OUTPUT_READY_PATH";
        const BLOCKED_OUTPUT_DRAINED_PATH: &str = "MACHINE_GOD_ASK_BLOCKED_OUTPUT_DRAINED_PATH";
        const SETUP_LOCK_CHILD_MODE: &str = "MACHINE_GOD_ASK_SETUP_LOCK_CHILD";
        const SETUP_LOCK_ROOT: &str = "MACHINE_GOD_ASK_SETUP_LOCK_ROOT";
        const SIGNAL_STAGE_READY_PATH: &str = "MACHINE_GOD_ASK_SIGNAL_STAGE_READY_PATH";
        const DIAGNOSTIC_CHILD_MODE: &str = "MACHINE_GOD_ASK_DIAGNOSTIC_CHILD";

        struct ScopedChild {
            child: Child,
        }

        struct ScopedTestDirectory {
            path: PathBuf,
        }

        struct ScopedMarkerPaths {
            ready: PathBuf,
            drained: PathBuf,
        }

        impl ScopedMarkerPaths {
            fn new(prefix: &str) -> Self {
                let ready = std::env::temp_dir().join(format!("{prefix}-ready"));
                let drained = std::env::temp_dir().join(format!("{prefix}-drained"));
                let _ = fs::remove_file(&ready);
                let _ = fs::remove_file(&drained);
                Self { ready, drained }
            }
        }

        impl Drop for ScopedMarkerPaths {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.ready);
                let _ = fs::remove_file(&self.drained);
            }
        }

        impl ScopedTestDirectory {
            fn new(label: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "machine-god-ask-stage-{}-{label}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&path);
                fs::create_dir(&path).expect("stage directory should be creatable");
                Self { path }
            }

            fn path(&self) -> &Path {
                &self.path
            }
        }

        impl Drop for ScopedTestDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.path);
            }
        }

        impl ScopedChild {
            fn spawn(command: &mut Command) -> Self {
                Self {
                    child: command.spawn().expect("subprocess should start"),
                }
            }

            fn id(&self) -> u32 {
                self.child.id()
            }

            fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
                self.child.try_wait()
            }

            fn wait_for_exit(&mut self, timeout: Duration) -> std::process::ExitStatus {
                let deadline = Instant::now() + timeout;
                loop {
                    if let Some(status) = self
                        .child
                        .try_wait()
                        .expect("child status should be readable")
                    {
                        return status;
                    }
                    if Instant::now() >= deadline {
                        self.child.kill().expect("stuck child should be killable");
                        let _ = self.child.wait();
                        panic!("subprocess did not exit within {timeout:?}");
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }

        impl Drop for ScopedChild {
            fn drop(&mut self) {
                if !matches!(self.child.try_wait(), Ok(Some(_))) {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                }
            }
        }

        #[test]
        #[ignore = "subprocess helper invoked by setup_and_final_stage_signals_have_precedence"]
        fn contended_session_lock_signal_child() {
            if std::env::var_os(SETUP_LOCK_CHILD_MODE).is_none() {
                return;
            }
            let root = PathBuf::from(
                std::env::var_os(SETUP_LOCK_ROOT).expect("session root should be set"),
            );
            let ready_path = PathBuf::from(
                std::env::var_os(SIGNAL_STAGE_READY_PATH).expect("ready path should be set"),
            );
            let _controller = AskSignalController::spawn()
                .expect("setup-stage signal guardian should register first");
            let store = FileSessionStore::open(&root).expect("session store should open");
            let id = SessionId::new("setup-lock").expect("fixed session ID should be valid");
            let record = SessionRecord::empty(
                id.clone(),
                SessionIncarnationId::new("setup-lock-incarnation")
                    .expect("fixed incarnation should be valid"),
            );
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("setup child runtime should build");
            runtime
                .block_on(store.save(record, None))
                .expect("initial session should persist");
            let lock_path = fs::read_dir(&root)
                .expect("session root should be readable")
                .map(|entry| entry.expect("session entry should be readable").path())
                .find(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "lock")
                })
                .expect("session lock should exist");
            let holder_ready = root.join("holder-ready");
            let mut holder_command = Command::new("python3");
            holder_command
                .args([
                    "-c",
                    concat!(
                        "import fcntl, pathlib, sys\n",
                        "lock = open(sys.argv[1], 'r+b', buffering=0)\n",
                        "fcntl.flock(lock.fileno(), fcntl.LOCK_EX)\n",
                        "pathlib.Path(sys.argv[2]).write_bytes(b'ready')\n",
                        "sys.stdin.buffer.read(1)\n",
                    ),
                ])
                .arg(lock_path)
                .arg(&holder_ready)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let mut holder = ScopedChild::spawn(&mut holder_command);
            wait_for_marker(&mut holder, &holder_ready);
            fs::write(&ready_path, b"contended-session-load-entering")
                .expect("setup-stage marker should be writable");

            let _ = runtime.block_on(store.load(id));
            panic!("contended session load unexpectedly completed");
        }

        struct RegisteredFailureHost;

        impl AskCommandHost for RegisteredFailureHost {
            fn execute(&self, _prompt: String, _output: &mut dyn io::Write) -> AskCommandExecution {
                let controller = AskSignalController::spawn()
                    .expect("diagnostic-stage signal guardian should register");
                controller
                    .enter_final()
                    .expect("diagnostic stage should activate");
                AskCommandExecution::with_finalizer(
                    AskCommandOutcome::OperationalFailure,
                    controller,
                )
            }
        }

        struct SaturatedDiagnostic {
            ready_path: PathBuf,
        }

        impl io::Write for SaturatedDiagnostic {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                fs::write(&self.ready_path, b"diagnostic-write-blocked")?;
                loop {
                    std::thread::park();
                }
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        #[test]
        #[ignore = "subprocess helper invoked by setup_and_final_stage_signals_have_precedence"]
        fn saturated_diagnostic_signal_child() {
            if std::env::var_os(DIAGNOSTIC_CHILD_MODE).is_none() {
                return;
            }
            let ready_path = PathBuf::from(
                std::env::var_os(SIGNAL_STAGE_READY_PATH).expect("ready path should be set"),
            );
            let mut stdout = io::sink();
            let mut stderr = SaturatedDiagnostic { ready_path };
            let _ = run_ask(
                &RegisteredFailureHost,
                "redacted-prompt".to_owned(),
                &mut stdout,
                &mut stderr,
                "fixed-output-failure\n",
            );
            panic!("saturated diagnostic unexpectedly returned");
        }

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
                    3 => Poll::Ready(AskSignal::ControlFailed),
                    _ => Poll::Pending,
                }
            }
        }

        const fn signal_number(signal: AskSignal) -> u8 {
            match signal {
                AskSignal::Interrupt => 1,
                AskSignal::Terminate => 2,
                AskSignal::ControlFailed => 3,
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
        fn thread_spawn_failures_map_to_fixed_operational_failure_seam() {
            let error = io::Error::other("injected thread creation detail");
            assert_eq!(map_thread_spawn::<()>(Err(error)), Err(()));
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
        fn writer_panic_propagates_after_scoped_turn_cancels_and_drains() {
            let turn = ScriptedTurn::new([text_event("provider-secret")]);
            let cancelled = turn.cancellation_flag();
            let drained = turn.drained_flag();
            let output = RecordingOutput {
                panic_write: true,
                ..RecordingOutput::default()
            };

            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_script(turn, TestSignals::default(), output)
            }));

            assert!(panic.is_err());
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
            delayed_write: bool,
        }

        impl io::Write for PermanentlyBlockedOutput {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if self.block_on_flush {
                    if self.delayed_write {
                        fs::write(&self.ready_path, b"write-entered")?;
                        std::thread::sleep(Duration::from_millis(90));
                    }
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
            let (signal, block_on_flush, ready_before_signal, delayed_write) = match mode.as_str() {
                "interrupt-write" => (AskSignal::Interrupt, false, false, false),
                "terminate-write" => (AskSignal::Terminate, false, false, false),
                "interrupt-flush" => (AskSignal::Interrupt, true, false, false),
                "interrupt-preflush" => (AskSignal::Interrupt, true, true, false),
                "interrupt-delayed-write-flush" => (AskSignal::Interrupt, true, false, true),
                _ => panic!("unsupported child signal mode"),
            };
            let ready_path = PathBuf::from(
                std::env::var_os(BLOCKED_OUTPUT_READY_PATH).expect("ready path should be set"),
            );
            let drained_path = PathBuf::from(
                std::env::var_os(BLOCKED_OUTPUT_DRAINED_PATH).expect("drained path should be set"),
            );
            let turn = if delayed_write {
                ScriptedTurn::new([
                    text_event("delayed-write"),
                    completed_event(StopReason::Completed),
                ])
            } else if block_on_flush && !ready_before_signal {
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
            let mut controller =
                super::AskSignalController::spawn().expect("child signal guardian should register");
            let control = controller.control();
            let signals = controller
                .take_signals()
                .expect("child signal receiver should be owned");

            std::thread::scope(|scope| {
                let (work_sender, work_receiver) = tokio::sync::mpsc::channel(1);
                let (acknowledgement_sender, acknowledgement_receiver) =
                    tokio::sync::mpsc::channel(1);
                let worker = scope.spawn(move || {
                    let runtime_control = control.clone();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
                        .build()
                        .expect("child runtime should build");
                    let result = runtime.block_on(async move {
                        let mut signals = signals;
                        runtime_control
                            .activate_turn()
                            .expect("child turn phase should activate");
                        if ready_before_signal {
                            fs::write(&listener_ready_path, b"signal-listeners-ready")
                                .expect("listener marker should be writable");
                        }
                        let mut turn = turn;
                        let result = drive_turn_stream(
                            &mut turn,
                            || cancellation.store(true, Ordering::Release),
                            &mut signals,
                            OutputBridge {
                                work: work_sender,
                                acknowledgements: acknowledgement_receiver,
                            },
                        )
                        .await;
                        runtime_control
                            .enter_final()
                            .expect("child final phase should activate");
                        result
                    });
                    assert_eq!(result.outcome, signal.into());
                    assert!(result.stalled_output_after_signal);
                    assert!(drained.load(Ordering::Acquire));
                    fs::write(drained_path, b"terminal-drained")
                        .expect("drained marker should be writable");
                    runtime
                        .block_on(control.finish(result.outcome.exit_code()))
                        .expect("child guardian should finish");
                });

                let mut output = PermanentlyBlockedOutput {
                    ready_path,
                    block_on_flush,
                    delayed_write,
                };
                serve_output(work_receiver, &acknowledgement_sender, &mut output);
                worker.join().expect("child worker should join");
            });
            drop(controller);
        }

        impl From<AskSignal> for AskCommandOutcome {
            fn from(signal: AskSignal) -> Self {
                match signal {
                    AskSignal::Interrupt => Self::Interrupted,
                    AskSignal::Terminate => Self::Terminated,
                    AskSignal::ControlFailed => Self::OperationalFailure,
                }
            }
        }

        fn wait_for_marker(child: &mut ScopedChild, path: &Path) {
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

        #[test]
        fn setup_and_final_stage_signals_have_precedence() {
            for (helper, mode_variable, mode, kill_signal, expected_exit) in [
                (
                    "ask::production::tests::contended_session_lock_signal_child",
                    SETUP_LOCK_CHILD_MODE,
                    "setup-lock-interrupt",
                    "-INT",
                    130,
                ),
                (
                    "ask::production::tests::contended_session_lock_signal_child",
                    SETUP_LOCK_CHILD_MODE,
                    "setup-lock-terminate",
                    "-TERM",
                    143,
                ),
                (
                    "ask::production::tests::saturated_diagnostic_signal_child",
                    DIAGNOSTIC_CHILD_MODE,
                    "diagnostic-interrupt",
                    "-INT",
                    130,
                ),
                (
                    "ask::production::tests::saturated_diagnostic_signal_child",
                    DIAGNOSTIC_CHILD_MODE,
                    "diagnostic-terminate",
                    "-TERM",
                    143,
                ),
            ] {
                let temporary = ScopedTestDirectory::new(mode);
                let session_root = temporary.path().join("sessions");
                fs::create_dir(&session_root).expect("session root should be creatable");
                let ready_path = temporary.path().join("stage-ready");
                let mut command = Command::new(
                    std::env::current_exe().expect("current test executable should resolve"),
                );
                command
                    .args(["--exact", helper, "--ignored", "--nocapture"])
                    .env(mode_variable, mode)
                    .env(SETUP_LOCK_ROOT, &session_root)
                    .env(SIGNAL_STAGE_READY_PATH, &ready_path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let mut child = ScopedChild::spawn(&mut command);

                wait_for_marker(&mut child, &ready_path);
                let kill_status = Command::new("kill")
                    .args([kill_signal, &child.id().to_string()])
                    .status()
                    .expect("signal command should run");
                assert!(kill_status.success());
                let status = child.wait_for_exit(Duration::from_secs(10));

                assert_eq!(status.code(), Some(expected_exit), "stage mode {mode}");
            }
        }

        #[test]
        fn blocked_output_signals_exit_after_terminal_drain() {
            for (mode, kill_signal, expected_exit, max_signal_elapsed) in [
                ("interrupt-write", "-INT", 130, None),
                ("terminate-write", "-TERM", 143, None),
                ("interrupt-flush", "-INT", 130, None),
                ("interrupt-preflush", "-INT", 130, None),
                (
                    "interrupt-delayed-write-flush",
                    "-INT",
                    130,
                    Some(Duration::from_millis(165)),
                ),
            ] {
                let prefix = format!("machine-god-ask-{}-{mode}", std::process::id());
                let markers = ScopedMarkerPaths::new(&prefix);
                let mut command = Command::new(
                    std::env::current_exe().expect("current test executable should resolve"),
                );
                command
                    .args([
                        "--exact",
                        "ask::production::tests::permanently_blocked_output_signal_child",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(BLOCKED_OUTPUT_CHILD_MODE, mode)
                    .env(BLOCKED_OUTPUT_READY_PATH, &markers.ready)
                    .env(BLOCKED_OUTPUT_DRAINED_PATH, &markers.drained)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let mut child = ScopedChild::spawn(&mut command);

                wait_for_marker(&mut child, &markers.ready);
                let signal_started = Instant::now();
                let kill_status = Command::new("kill")
                    .args([kill_signal, &child.id().to_string()])
                    .status()
                    .expect("signal command should run");
                assert!(kill_status.success());
                let status = child.wait_for_exit(Duration::from_secs(10));
                if let Some(max_signal_elapsed) = max_signal_elapsed {
                    assert!(
                        signal_started.elapsed() < max_signal_elapsed,
                        "write and flush restarted the absolute output grace: {:?}",
                        signal_started.elapsed()
                    );
                }

                assert_eq!(status.code(), Some(expected_exit));
                assert_eq!(
                    fs::read(&markers.drained).expect("drained marker should exist"),
                    b"terminal-drained"
                );
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
    fn execute(&self, _prompt: String, _output: &mut dyn io::Write) -> AskCommandExecution {
        AskCommandExecution::without_finalizer(AskCommandOutcome::OperationalFailure)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::io;

    use super::{
        ASK_OPERATIONAL_FAILURE, AskCommandExecution, AskCommandHost, AskCommandOutcome,
        MAX_ASK_PROMPT_BYTES, parse_prompt_arguments, run_ask,
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
        fn execute(&self, prompt: String, output: &mut dyn io::Write) -> AskCommandExecution {
            self.calls.set(self.calls.get() + 1);
            self.prompts.borrow_mut().push(prompt);
            let outcome = if output.write_all(self.bytes).is_err() {
                AskCommandOutcome::OutputFailure
            } else {
                self.outcome
            };
            AskCommandExecution::without_finalizer(outcome)
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
