//! Model catalog composition, signal ownership, and bounded presentation.
use crate::bounded_output::BoundedOutput;
use crate::{OUTPUT_FAILURE, push_json_string, write_json_string, write_json_string_content};
#[cfg(all(not(target_family = "wasm"), not(any(unix, windows))))]
use machine_god_core::BoxFuture;
#[cfg(not(target_family = "wasm"))]
use machine_god_core::{CancellationToken, ModelCatalogProvider, ProviderError, ProviderErrorKind};
use machine_god_core::{ModelCatalog, ModelCatalogAccess, PublicCatalogReason};
#[cfg(not(target_family = "wasm"))]
use machine_god_native::{
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogHttpTransport,
    AiGatewayModelCatalogProvider, AiGatewayModelCatalogTransport,
    DiscoveredAiGatewayCatalogCredential, discover_process_ai_gateway_catalog_credential,
};
use machine_god_native::{
    NativeCredentialSourceKind, NativeProviderKind, NativeTransportKind, load_process_config,
};
use std::{fmt::Write as _, io};
#[cfg(not(target_family = "wasm"))]
use std::{
    future::poll_fn,
    sync::{Arc, mpsc},
    task::{Context, Poll},
    thread::JoinHandle,
};
const MAX_MODELS_OUTPUT_BYTES: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelsOperationalFailure {
    #[cfg(not(target_family = "wasm"))]
    AuthenticationRejected,
    #[cfg(not(target_family = "wasm"))]
    Cancelled,
    #[cfg(not(target_family = "wasm"))]
    MalformedResponse,
    ResourceLimit,
    Unavailable,
}

impl ModelsOperationalFailure {
    const fn detail(self) -> &'static str {
        match self {
            #[cfg(not(target_family = "wasm"))]
            Self::AuthenticationRejected => "AuthenticationRejected",
            #[cfg(not(target_family = "wasm"))]
            Self::Cancelled => "the request was cancelled",
            #[cfg(not(target_family = "wasm"))]
            Self::MalformedResponse => "MalformedResponse",
            Self::ResourceLimit => "ResourceLimit",
            Self::Unavailable => "Unavailable",
        }
    }

    const fn code(self) -> &'static str {
        match self {
            #[cfg(not(target_family = "wasm"))]
            Self::AuthenticationRejected => "AuthenticationRejected",
            #[cfg(not(target_family = "wasm"))]
            Self::Cancelled => "Cancelled",
            #[cfg(not(target_family = "wasm"))]
            Self::MalformedResponse => "MalformedResponse",
            Self::ResourceLimit => "ResourceLimit",
            Self::Unavailable => "Unavailable",
        }
    }
}

pub(crate) struct ModelsCommandExecution {
    result: Result<ModelCatalog, ModelsOperationalFailure>,
    #[cfg(not(target_family = "wasm"))]
    _output_signal_guard: Option<ModelsOutputSignalGuard>,
}

impl ModelsCommandExecution {
    fn without_signal_guard(result: Result<ModelCatalog, ModelsOperationalFailure>) -> Self {
        Self {
            result,
            #[cfg(not(target_family = "wasm"))]
            _output_signal_guard: None,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn with_signal_guard(
        result: Result<ModelCatalog, ModelsOperationalFailure>,
        output_signal_guard: ModelsOutputSignalGuard,
    ) -> Self {
        Self {
            result,
            _output_signal_guard: Some(output_signal_guard),
        }
    }

    fn result(&self) -> &Result<ModelCatalog, ModelsOperationalFailure> {
        &self.result
    }
}

pub(crate) trait ModelsCommandHost {
    fn list_models(&self) -> ModelsCommandExecution;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProductionModelsCommandHost;

#[cfg(not(target_family = "wasm"))]
trait ModelsCompositionEffects {
    type Credential;

    fn load_and_validate_config(&self) -> Result<(), ModelsOperationalFailure>;

    fn discover_credential(&self) -> Result<Self::Credential, ModelsOperationalFailure>;

    fn create_transport_and_list(&self, credential: Self::Credential) -> ModelsCommandExecution;
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Default)]
struct ProcessModelsCompositionEffects;

#[cfg(not(target_family = "wasm"))]
impl ModelsCompositionEffects for ProcessModelsCompositionEffects {
    type Credential = DiscoveredAiGatewayCatalogCredential;

    fn load_and_validate_config(&self) -> Result<(), ModelsOperationalFailure> {
        let loaded = load_process_config().map_err(|_| ModelsOperationalFailure::Unavailable)?;
        let config = loaded.config();
        if config.provider() != NativeProviderKind::VercelAiGateway
            || config.transport() != NativeTransportKind::AiGatewayHttp
            || config.credential_source() != NativeCredentialSourceKind::Environment
        {
            return Err(ModelsOperationalFailure::Unavailable);
        }
        Ok(())
    }

    fn discover_credential(&self) -> Result<Self::Credential, ModelsOperationalFailure> {
        discover_process_ai_gateway_catalog_credential()
            .map_err(|_| ModelsOperationalFailure::Unavailable)
    }

    fn create_transport_and_list(&self, credential: Self::Credential) -> ModelsCommandExecution {
        let (access_mode, bearer_token) = match credential {
            DiscoveredAiGatewayCatalogCredential::PublicOnly => {
                (AiGatewayModelCatalogAccessMode::PublicOnly, None)
            }
            DiscoveredAiGatewayCatalogCredential::Authenticated(credential) => (
                AiGatewayModelCatalogAccessMode::Authenticated,
                Some(credential.into_bearer_token()),
            ),
        };
        let Ok(transport) = AiGatewayModelCatalogHttpTransport::new(bearer_token) else {
            return ModelsCommandExecution::without_signal_guard(Err(
                ModelsOperationalFailure::Unavailable,
            ));
        };
        let transport: Arc<dyn AiGatewayModelCatalogTransport> = Arc::new(transport);
        let provider = AiGatewayModelCatalogProvider::new(access_mode, transport);
        list_models_with_signals(&provider)
    }
}

#[cfg(not(target_family = "wasm"))]
fn list_models_with_effects(effects: &impl ModelsCompositionEffects) -> ModelsCommandExecution {
    if let Err(failure) = effects.load_and_validate_config() {
        return ModelsCommandExecution::without_signal_guard(Err(failure));
    }
    let credential = match effects.discover_credential() {
        Ok(credential) => credential,
        Err(failure) => return ModelsCommandExecution::without_signal_guard(Err(failure)),
    };
    effects.create_transport_and_list(credential)
}

impl ModelsCommandHost for ProductionModelsCommandHost {
    fn list_models(&self) -> ModelsCommandExecution {
        #[cfg(not(target_family = "wasm"))]
        {
            list_models_with_effects(&ProcessModelsCompositionEffects)
        }

        #[cfg(target_family = "wasm")]
        {
            let Ok(loaded) = load_process_config() else {
                return ModelsCommandExecution::without_signal_guard(Err(
                    ModelsOperationalFailure::Unavailable,
                ));
            };
            let config = loaded.config();
            if config.provider() != NativeProviderKind::VercelAiGateway
                || config.transport() != NativeTransportKind::AiGatewayHttp
                || config.credential_source() != NativeCredentialSourceKind::Environment
            {
                return ModelsCommandExecution::without_signal_guard(Err(
                    ModelsOperationalFailure::Unavailable,
                ));
            }
            ModelsCommandExecution::without_signal_guard(Err(ModelsOperationalFailure::Unavailable))
        }
    }
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelsSignalKind {
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

#[cfg(not(target_family = "wasm"))]
impl ModelsSignalKind {
    const fn exit_code(self) -> i32 {
        match self {
            Self::Interrupt => 130,
            #[cfg(unix)]
            Self::Terminate => 143,
        }
    }
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelsSignalEvent {
    Received(ModelsSignalKind),
    WaitFailed,
}

#[cfg(not(target_family = "wasm"))]
trait ModelsSignalSource: Send + 'static {
    fn registration_failed(&self) -> bool;

    fn poll_interrupt(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent>;

    #[cfg(unix)]
    fn poll_terminate(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent>;
}

#[cfg(not(target_family = "wasm"))]
struct TokioModelsSignalSource {
    #[cfg(unix)]
    interrupt: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
    #[cfg(windows)]
    interrupt: Option<tokio::signal::windows::CtrlC>,
    #[cfg(not(any(unix, windows)))]
    interrupt: BoxFuture<'static, ModelsSignalEvent>,
    registration_failed: bool,
}

#[cfg(not(target_family = "wasm"))]
impl TokioModelsSignalSource {
    fn register() -> Self {
        #[cfg(unix)]
        {
            let interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok();
            let terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
            let registration_failed = interrupt.is_none() || terminate.is_none();
            Self {
                interrupt,
                terminate,
                registration_failed,
            }
        }

        #[cfg(windows)]
        {
            let interrupt = tokio::signal::windows::ctrl_c().ok();
            let registration_failed = interrupt.is_none();
            Self {
                interrupt,
                registration_failed,
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            Self {
                interrupt: ctrl_c_signal_event(),
                registration_failed: false,
            }
        }
    }
}

#[cfg(all(not(target_family = "wasm"), not(any(unix, windows))))]
fn ctrl_c_signal_event() -> BoxFuture<'static, ModelsSignalEvent> {
    Box::pin(async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => ModelsSignalEvent::Received(ModelsSignalKind::Interrupt),
            Err(_) => ModelsSignalEvent::WaitFailed,
        }
    })
}

#[cfg(not(target_family = "wasm"))]
impl ModelsSignalSource for TokioModelsSignalSource {
    fn registration_failed(&self) -> bool {
        self.registration_failed
    }

    fn poll_interrupt(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent> {
        #[cfg(unix)]
        {
            match self.interrupt.as_mut() {
                Some(interrupt) => interrupt.poll_recv(context).map(|received| match received {
                    Some(()) => ModelsSignalEvent::Received(ModelsSignalKind::Interrupt),
                    None => ModelsSignalEvent::WaitFailed,
                }),
                None => Poll::Pending,
            }
        }

        #[cfg(windows)]
        {
            match self.interrupt.as_mut() {
                Some(interrupt) => interrupt.poll_recv(context).map(|received| match received {
                    Some(()) => ModelsSignalEvent::Received(ModelsSignalKind::Interrupt),
                    None => ModelsSignalEvent::WaitFailed,
                }),
                None => Poll::Pending,
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            let event = self.interrupt.as_mut().poll(context);
            if matches!(event, Poll::Ready(ModelsSignalEvent::Received(_))) {
                self.interrupt = ctrl_c_signal_event();
            }
            event
        }
    }

    #[cfg(unix)]
    fn poll_terminate(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent> {
        match self.terminate.as_mut() {
            Some(terminate) => terminate.poll_recv(context).map(|received| match received {
                Some(()) => ModelsSignalEvent::Received(ModelsSignalKind::Terminate),
                None => ModelsSignalEvent::WaitFailed,
            }),
            None => Poll::Pending,
        }
    }
}

#[cfg(all(test, not(target_family = "wasm"), unix))]
const fn terminate_signal_event(received: Option<()>) -> ModelsSignalEvent {
    match received {
        Some(()) => ModelsSignalEvent::Received(ModelsSignalKind::Terminate),
        None => ModelsSignalEvent::WaitFailed,
    }
}

#[cfg(not(target_family = "wasm"))]
struct ModelsSignalPhase<S> {
    result: Result<ModelCatalog, ProviderError>,
    signals: S,
    wait_failed: bool,
}

#[cfg(not(target_family = "wasm"))]
fn list_models_with_signals(provider: &dyn ModelCatalogProvider) -> ModelsCommandExecution {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    else {
        return ModelsCommandExecution::without_signal_guard(Err(
            ModelsOperationalFailure::Unavailable,
        ));
    };
    let Ok(pending_guardian) = PendingModelsSignalGuardian::spawn() else {
        return ModelsCommandExecution::without_signal_guard(Err(
            ModelsOperationalFailure::Unavailable,
        ));
    };
    let signals = {
        let _entered = runtime.enter();
        TokioModelsSignalSource::register()
    };
    let phase = runtime.block_on(list_models_with_signal_source(provider, signals));
    let result = phase
        .result
        .map_err(|error| classify_provider_error(&error));
    let output_signal_guard = pending_guardian.activate(runtime, phase.signals, phase.wait_failed);
    ModelsCommandExecution::with_signal_guard(result, output_signal_guard)
}

#[cfg(not(target_family = "wasm"))]
async fn list_models_with_signal_source<S: ModelsSignalSource>(
    provider: &dyn ModelCatalogProvider,
    mut signals: S,
) -> ModelsSignalPhase<S> {
    let cancellation = CancellationToken::new();

    if signals.registration_failed() {
        return ModelsSignalPhase {
            result: Err(signal_unavailable_error()),
            signals,
            wait_failed: false,
        };
    }

    let initial_signal = poll_fn(|context| match poll_models_signal(&mut signals, context) {
        Poll::Ready(event) => Poll::Ready(Some(event)),
        Poll::Pending => Poll::Ready(None),
    })
    .await;
    if let Some(event) = initial_signal {
        cancellation.cancel();
        return ModelsSignalPhase {
            result: Err(signal_event_error(event)),
            signals,
            wait_failed: event == ModelsSignalEvent::WaitFailed,
        };
    }

    let mut provider_future = provider.list_models(cancellation.clone());
    let (result, wait_failed) = poll_fn(|context| {
        if let Poll::Ready(event) = poll_models_signal(&mut signals, context) {
            cancellation.cancel();
            return Poll::Ready((
                Err(signal_event_error(event)),
                event == ModelsSignalEvent::WaitFailed,
            ));
        }
        let provider_result = match provider_future.as_mut().poll(context) {
            Poll::Ready(result) => result,
            Poll::Pending => return Poll::Pending,
        };
        if let Poll::Ready(event) = poll_models_signal(&mut signals, context) {
            cancellation.cancel();
            return Poll::Ready((
                Err(signal_event_error(event)),
                event == ModelsSignalEvent::WaitFailed,
            ));
        }
        Poll::Ready((provider_result, false))
    })
    .await;
    drop(provider_future);
    ModelsSignalPhase {
        result,
        signals,
        wait_failed,
    }
}

#[cfg(not(target_family = "wasm"))]
fn poll_models_signal(
    signals: &mut impl ModelsSignalSource,
    context: &mut Context<'_>,
) -> Poll<ModelsSignalEvent> {
    if let Poll::Ready(event) = signals.poll_interrupt(context) {
        return Poll::Ready(event);
    }
    #[cfg(unix)]
    if let Poll::Ready(event) = signals.poll_terminate(context) {
        return Poll::Ready(event);
    }
    Poll::Pending
}

#[cfg(not(target_family = "wasm"))]
struct ModelsSignalGuardianActivation {
    runtime: tokio::runtime::Runtime,
    signals: TokioModelsSignalSource,
    stop: CancellationToken,
    ready: mpsc::SyncSender<()>,
    wait_failed: bool,
}

#[cfg(not(target_family = "wasm"))]
struct PendingModelsSignalGuardian {
    sender: Option<mpsc::SyncSender<ModelsSignalGuardianActivation>>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(not(target_family = "wasm"))]
impl PendingModelsSignalGuardian {
    fn spawn() -> Result<Self, ()> {
        let (sender, receiver) = mpsc::sync_channel(0);
        let worker = std::thread::Builder::new()
            .name("machine-god-models-signals".to_owned())
            .spawn(move || run_models_signal_guardian(&receiver))
            .map_err(|_| ())?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    fn activate(
        mut self,
        runtime: tokio::runtime::Runtime,
        signals: TokioModelsSignalSource,
        wait_failed: bool,
    ) -> ModelsOutputSignalGuard {
        let stop = CancellationToken::new();
        let (ready, ready_receiver) = mpsc::sync_channel(0);
        let activation = ModelsSignalGuardianActivation {
            runtime,
            signals,
            stop: stop.clone(),
            ready,
            wait_failed,
        };
        let Some(sender) = self.sender.take() else {
            signal_control_failed();
        };
        if sender.send(activation).is_err() || ready_receiver.recv().is_err() {
            signal_control_failed();
        }
        ModelsOutputSignalGuard {
            stop,
            worker: self.worker.take(),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
impl Drop for PendingModelsSignalGuardian {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(target_family = "wasm"))]
struct ModelsOutputSignalGuard {
    stop: CancellationToken,
    worker: Option<JoinHandle<()>>,
}

#[cfg(not(target_family = "wasm"))]
impl Drop for ModelsOutputSignalGuard {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn run_models_signal_guardian(receiver: &mpsc::Receiver<ModelsSignalGuardianActivation>) {
    let Ok(activation) = receiver.recv() else {
        return;
    };
    if activation.wait_failed {
        signal_control_failed();
    }
    let ModelsSignalGuardianActivation {
        runtime,
        mut signals,
        stop,
        ready,
        wait_failed: _,
    } = activation;
    let mut stopped = Box::pin(stop.cancelled());
    let mut ready = Some(ready);
    let mut stop_drain: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
    let event = runtime.block_on(poll_fn(|context| {
        if let Poll::Ready(event) = poll_models_signal(&mut signals, context) {
            return Poll::Ready(Some(event));
        }
        if let Some(ready) = ready.take()
            && ready.send(()).is_err()
        {
            return Poll::Ready(None);
        }
        if let Some(drain) = stop_drain.as_mut() {
            return drain.as_mut().poll(context).map(|()| None);
        }
        if stopped.as_mut().poll(context).is_ready() {
            #[cfg(all(test, unix))]
            pause_models_signal_guardian_after_stop_for_test();
            let mut drain = Box::pin(tokio::time::sleep(std::time::Duration::from_millis(1)));
            let poll = drain.as_mut().poll(context).map(|()| None);
            stop_drain = Some(drain);
            return poll;
        }
        Poll::Pending
    }));
    match event {
        Some(ModelsSignalEvent::Received(kind)) => std::process::exit(kind.exit_code()),
        Some(ModelsSignalEvent::WaitFailed) => signal_control_failed(),
        None => {}
    }
}

#[cfg(all(test, not(target_family = "wasm"), unix))]
fn pause_models_signal_guardian_after_stop_for_test() {
    use std::io::{Read, Write};

    if std::env::var_os("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD").as_deref()
        != Some(std::ffi::OsStr::new("stop-drain"))
    {
        return;
    }
    let mut stderr = io::stderr().lock();
    stderr.write_all(b"GUARDIAN_STOP_READY\n").unwrap();
    stderr.flush().unwrap();
    let mut release = [0_u8];
    io::stdin().read_exact(&mut release).unwrap();
}

#[cfg(not(target_family = "wasm"))]
fn signal_control_failed() -> ! {
    std::process::exit(1)
}

#[cfg(not(target_family = "wasm"))]
fn signal_event_error(event: ModelsSignalEvent) -> ProviderError {
    match event {
        ModelsSignalEvent::Received(_) => ProviderError::new(
            ProviderErrorKind::Cancelled,
            "Cancelled",
            "model catalog request was cancelled",
            false,
        ),
        ModelsSignalEvent::WaitFailed => signal_unavailable_error(),
    }
}

#[cfg(not(target_family = "wasm"))]
fn signal_unavailable_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Unavailable,
        "SignalUnavailable",
        "model catalog signal handling is unavailable",
        true,
    )
}

#[cfg(not(target_family = "wasm"))]
fn classify_provider_error(error: &ProviderError) -> ModelsOperationalFailure {
    match error.code.as_str() {
        "AuthenticationRejected" => ModelsOperationalFailure::AuthenticationRejected,
        "Cancelled" => ModelsOperationalFailure::Cancelled,
        "MalformedResponse" => ModelsOperationalFailure::MalformedResponse,
        "ResourceLimit" => ModelsOperationalFailure::ResourceLimit,
        _ => ModelsOperationalFailure::Unavailable,
    }
}

pub(crate) fn run_models(
    host: &(impl ModelsCommandHost + ?Sized),
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let execution = host.list_models();
    let catalog = match execution.result() {
        Ok(catalog) => catalog,
        Err(failure) => return write_models_failure(*failure, json, stdout, stderr),
    };

    let output = match render_models(catalog, json) {
        Ok(output) => output,
        Err(failure) => return write_models_failure(failure, json, stdout, stderr),
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn write_models_failure(
    failure: ModelsOperationalFailure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    if json {
        let mut output = String::from("{\"kind\":\"models\",\"error\":");
        let detail = format!("could not list models: {}", failure.detail());
        push_json_string(&mut output, &detail);
        output.push_str(",\"code\":");
        push_json_string(&mut output, failure.code());
        output.push_str("}\n");
        if stdout.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    } else {
        let mut output = String::from("machine-god models: could not list models: ");
        output.push_str(failure.detail());
        output.push('\n');
        let _ = stderr.write_all(output.as_bytes());
    }
    1
}

fn render_models(catalog: &ModelCatalog, json: bool) -> Result<String, ModelsOperationalFailure> {
    let mut output = BoundedOutput::with_capacity(MAX_MODELS_OUTPUT_BYTES, 1024);
    let rendered = if json {
        write_json_models(&mut output, catalog)
    } else {
        write_human_models(&mut output, catalog)
    };
    rendered.map_err(|_| ModelsOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn write_human_models(output: &mut BoundedOutput, catalog: &ModelCatalog) -> std::fmt::Result {
    let models = catalog.models();
    if models.is_empty() {
        output.write_str("[models] no models returned by gateway\n")?;
    } else {
        writeln!(output, "[models] {} available", models.len())?;
        for model in models {
            output.write_str(" - ")?;
            write_json_string_content(output, model.id())?;
            output.write_char('\n')?;
        }
    }
    if let ModelCatalogAccess::PublicOnly { reason } = catalog.access() {
        output.write_str(match reason {
            PublicCatalogReason::NoCredential => concat!(
                "[models] Using the public model catalog; set VERCEL_OIDC_TOKEN or ",
                "AI_GATEWAY_API_KEY to include private models.\n",
            ),
            PublicCatalogReason::AuthenticatedCredentialRejected => {
                "[models] Gateway authentication was rejected; showing the public model catalog.\n"
            }
            _ => "[models] Using the public model catalog.\n",
        })?;
    }
    Ok(())
}

fn write_json_models(output: &mut BoundedOutput, catalog: &ModelCatalog) -> std::fmt::Result {
    let models = catalog.models();
    output.write_str("{\"kind\":\"models\",\"count\":")?;
    write!(
        output,
        "{},\"shown_count\":{},\"more_count\":0,\"private_models_hidden\":{},\"ids\":[",
        models.len(),
        models.len(),
        matches!(catalog.access(), ModelCatalogAccess::PublicOnly { .. }),
    )?;
    for (index, model) in models.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        write_json_string(output, model.id())?;
    }
    output.write_str("]}\n")
}

#[cfg(test)]
pub(crate) mod tests;
