use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
#[cfg(not(target_family = "wasm"))]
use std::future::poll_fn;
use std::io;
use std::path::Path;
use std::process::ExitCode;
#[cfg(not(target_family = "wasm"))]
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
#[cfg(not(target_family = "wasm"))]
use std::thread::JoinHandle;

mod ask;

#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

use ask::{AskCommandHost, ProductionAskCommandHost, parse_prompt_arguments, run_ask, run_resume};
use machine_god_core::{BoxFuture, SessionId, SessionIncarnationId};
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
    MAX_LIST_SESSIONS, NativeCredentialSourceKind, NativeDoctorCheckStatus, NativeDoctorReport,
    NativeProviderKind, NativeSessionInspection, NativeSessionInspectionError,
    NativeSessionInspectionErrorKind, NativeSessionList, NativeSessionListingError,
    NativeSessionListingErrorKind, NativeStatus, NativeTransportKind, PermissionMode,
    inspect_process_doctor, inspect_process_session, inspect_process_status, list_process_sessions,
    load_process_config,
};

const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | ask [--] <prompt...> | doctor [--json] | models [--json] | permissions [--json] | resume <id> [--] <prompt...> | session <id> [--json] | sessions [--json] | status [--json]]\n",
);
const CONFIGURATION_FAILURE: &str = "machine-god: failed to load configuration\n";
const DOCTOR_RENDER_FAILURE: &str = "machine-god doctor: could not render report\n";
const OUTPUT_FAILURE: &str = "machine-god: failed to write output\n";
const DOCTOR_CHECK_COUNT: usize = 4;
const MAX_DOCTOR_OUTPUT_BYTES: usize = 4096;
const MAX_MODELS_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_SESSION_OUTPUT_BYTES: usize = 4096;
const MAX_SESSIONS_OUTPUT_BYTES: usize = 16 * 1024;

#[derive(Debug)]
struct BoundedDoctorOutput {
    value: String,
}

impl BoundedDoctorOutput {
    fn new() -> Self {
        Self {
            value: String::with_capacity(1024),
        }
    }

    fn finish(self) -> String {
        self.value
    }
}

impl std::fmt::Write for BoundedDoctorOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.value.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > MAX_DOCTOR_OUTPUT_BYTES {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

#[derive(Debug)]
struct BoundedModelsOutput {
    value: String,
}

impl BoundedModelsOutput {
    fn new() -> Self {
        Self {
            value: String::with_capacity(1024),
        }
    }

    fn finish(self) -> String {
        self.value
    }
}

impl std::fmt::Write for BoundedModelsOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.value.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > MAX_MODELS_OUTPUT_BYTES {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

#[derive(Debug)]
struct BoundedSessionOutput {
    value: String,
}

impl BoundedSessionOutput {
    fn new() -> Self {
        Self {
            value: String::with_capacity(512),
        }
    }

    fn finish(self) -> String {
        self.value
    }
}

impl std::fmt::Write for BoundedSessionOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.value.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > MAX_SESSION_OUTPUT_BYTES {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

#[derive(Debug)]
struct BoundedSessionsOutput {
    value: String,
}

impl BoundedSessionsOutput {
    fn new() -> Self {
        Self {
            value: String::with_capacity(1024),
        }
    }

    fn finish(self) -> String {
        self.value
    }
}

impl std::fmt::Write for BoundedSessionsOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.value.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > MAX_SESSIONS_OUTPUT_BYTES {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Identity,
    Help,
    Ask { prompt: String },
    Doctor { json: bool },
    Models { json: bool },
    Permissions { json: bool },
    Resume { id: SessionId, prompt: String },
    Session { id: SessionId, json: bool },
    Sessions { json: bool },
    Status { json: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DoctorCheckStatus {
    Ok,
    Warn,
    Fail,
}

impl DoctorCheckStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

impl From<NativeDoctorCheckStatus> for DoctorCheckStatus {
    fn from(status: NativeDoctorCheckStatus) -> Self {
        match status {
            NativeDoctorCheckStatus::Ok => Self::Ok,
            NativeDoctorCheckStatus::Warn => Self::Warn,
            NativeDoctorCheckStatus::Fail => Self::Fail,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DoctorCheckSnapshot {
    name: &'static str,
    status: DoctorCheckStatus,
    detail: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DoctorReportSnapshot {
    ok_count: usize,
    warn_count: usize,
    fail_count: usize,
    checks: [DoctorCheckSnapshot; DOCTOR_CHECK_COUNT],
}

impl DoctorReportSnapshot {
    fn from_native(report: &NativeDoctorReport) -> Result<Self, ()> {
        if report.checked_count() != DOCTOR_CHECK_COUNT
            || report.checks().len() != DOCTOR_CHECK_COUNT
            || report
                .ok_count()
                .checked_add(report.warn_count())
                .and_then(|count| count.checked_add(report.fail_count()))
                != Some(DOCTOR_CHECK_COUNT)
        {
            return Err(());
        }

        let checks = std::array::from_fn(|index| {
            let check = &report.checks()[index];
            DoctorCheckSnapshot {
                name: check.name(),
                status: check.status().into(),
                detail: check.detail(),
            }
        });
        Ok(Self {
            ok_count: report.ok_count(),
            warn_count: report.warn_count(),
            fail_count: report.fail_count(),
            checks,
        })
    }
}

trait DoctorCommandHost {
    fn inspect_doctor(&self) -> Result<DoctorReportSnapshot, ()>;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProductionDoctorCommandHost;

impl DoctorCommandHost for ProductionDoctorCommandHost {
    fn inspect_doctor(&self) -> Result<DoctorReportSnapshot, ()> {
        DoctorReportSnapshot::from_native(&inspect_process_doctor())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionSnapshot {
    id: String,
    incarnation_id: String,
    revision: u64,
    next_turn_sequence: u64,
    message_count: usize,
    metadata_entry_count: usize,
}

impl SessionSnapshot {
    fn from_native(
        inspection: &NativeSessionInspection,
    ) -> Result<Self, SessionOperationalFailure> {
        let snapshot = Self {
            id: inspection.session_id().as_str().to_owned(),
            incarnation_id: inspection.incarnation_id().as_str().to_owned(),
            revision: inspection.revision().0,
            next_turn_sequence: inspection.next_turn_sequence(),
            message_count: inspection.message_count(),
            metadata_entry_count: inspection.metadata_entry_count(),
        };
        validate_session_snapshot(&snapshot)?;
        Ok(snapshot)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionOperationalFailure {
    NotFound,
    Corrupt,
    ResourceLimit,
    Unavailable,
    Unsupported,
}

impl SessionOperationalFailure {
    const fn category(self) -> &'static str {
        match self {
            Self::NotFound => "NotFound",
            Self::Corrupt => "Corrupt",
            Self::ResourceLimit => "ResourceLimit",
            Self::Unavailable => "Unavailable",
            Self::Unsupported => "Unsupported",
        }
    }
}

trait SessionCommandHost {
    fn inspect_session(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<SessionSnapshot, SessionOperationalFailure>>;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProductionSessionCommandHost;

impl SessionCommandHost for ProductionSessionCommandHost {
    fn inspect_session(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<SessionSnapshot, SessionOperationalFailure>> {
        Box::pin(async move {
            let inspection = inspect_process_session(id)
                .await
                .map_err(classify_session_inspection_error)?;
            SessionSnapshot::from_native(&inspection)
        })
    }
}

fn classify_session_inspection_error(
    error: NativeSessionInspectionError,
) -> SessionOperationalFailure {
    classify_session_inspection_error_kind(error.kind())
}

fn classify_session_inspection_error_kind(
    kind: NativeSessionInspectionErrorKind,
) -> SessionOperationalFailure {
    match kind {
        NativeSessionInspectionErrorKind::UnsupportedPlatform => {
            SessionOperationalFailure::Unsupported
        }
        NativeSessionInspectionErrorKind::NotFound => SessionOperationalFailure::NotFound,
        NativeSessionInspectionErrorKind::Corrupt => SessionOperationalFailure::Corrupt,
        _ => SessionOperationalFailure::Unavailable,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionsSnapshot {
    session_ids: Vec<String>,
    truncated: bool,
}

impl SessionsSnapshot {
    fn from_native(listing: NativeSessionList) -> Self {
        let truncated = listing.truncated();
        Self {
            session_ids: listing
                .into_session_ids()
                .into_iter()
                .map(|session_id| session_id.as_str().to_owned())
                .collect(),
            truncated,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionsOperationalFailure {
    Corrupt,
    ResourceLimit,
    Unavailable,
    Unsupported,
}

impl SessionsOperationalFailure {
    const fn category(self) -> &'static str {
        match self {
            Self::Corrupt => "Corrupt",
            Self::ResourceLimit => "ResourceLimit",
            Self::Unavailable => "Unavailable",
            Self::Unsupported => "Unsupported",
        }
    }
}

trait SessionsCommandHost {
    fn list_sessions(
        &self,
    ) -> BoxFuture<'static, Result<SessionsSnapshot, SessionsOperationalFailure>>;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProductionSessionsCommandHost;

impl SessionsCommandHost for ProductionSessionsCommandHost {
    fn list_sessions(
        &self,
    ) -> BoxFuture<'static, Result<SessionsSnapshot, SessionsOperationalFailure>> {
        Box::pin(async {
            list_process_sessions()
                .await
                .map(SessionsSnapshot::from_native)
                .map_err(classify_session_listing_error)
        })
    }
}

fn classify_session_listing_error(error: NativeSessionListingError) -> SessionsOperationalFailure {
    classify_session_listing_error_kind(error.kind())
}

fn classify_session_listing_error_kind(
    kind: NativeSessionListingErrorKind,
) -> SessionsOperationalFailure {
    match kind {
        NativeSessionListingErrorKind::UnsupportedPlatform => {
            SessionsOperationalFailure::Unsupported
        }
        NativeSessionListingErrorKind::Corrupt => SessionsOperationalFailure::Corrupt,
        _ => SessionsOperationalFailure::Unavailable,
    }
}

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

struct ModelsCommandExecution {
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

trait ModelsCommandHost {
    fn list_models(&self) -> ModelsCommandExecution;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProductionModelsCommandHost;

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

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    ExitCode::from(run(env::args_os().skip(1), &mut stdout, &mut stderr))
}

fn run(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    run_with_hosts(
        arguments,
        stdout,
        stderr,
        (
            &ProductionModelsCommandHost,
            &ProductionDoctorCommandHost,
            &ProductionSessionCommandHost,
            &ProductionSessionsCommandHost,
            &ProductionAskCommandHost,
        ),
    )
}

#[cfg(test)]
fn run_with_models_host(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    models_host: &impl ModelsCommandHost,
) -> u8 {
    run_with_hosts(
        arguments,
        stdout,
        stderr,
        (
            models_host,
            &ProductionDoctorCommandHost,
            &ProductionSessionCommandHost,
            &ProductionSessionsCommandHost,
            &ProductionAskCommandHost,
        ),
    )
}

#[cfg(test)]
fn run_with_doctor_host(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    doctor_host: &impl DoctorCommandHost,
) -> u8 {
    run_with_hosts(
        arguments,
        stdout,
        stderr,
        (
            &ProductionModelsCommandHost,
            doctor_host,
            &ProductionSessionCommandHost,
            &ProductionSessionsCommandHost,
            &ProductionAskCommandHost,
        ),
    )
}

#[cfg(test)]
fn run_with_session_host(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    session_host: &impl SessionCommandHost,
) -> u8 {
    run_with_hosts(
        arguments,
        stdout,
        stderr,
        (
            &ProductionModelsCommandHost,
            &ProductionDoctorCommandHost,
            session_host,
            &ProductionSessionsCommandHost,
            &ProductionAskCommandHost,
        ),
    )
}

#[cfg(test)]
fn run_with_sessions_host(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    sessions_host: &impl SessionsCommandHost,
) -> u8 {
    run_with_hosts(
        arguments,
        stdout,
        stderr,
        (
            &ProductionModelsCommandHost,
            &ProductionDoctorCommandHost,
            &ProductionSessionCommandHost,
            sessions_host,
            &ProductionAskCommandHost,
        ),
    )
}

#[cfg(test)]
fn run_with_ask_host(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    ask_host: &impl AskCommandHost,
) -> u8 {
    run_with_hosts(
        arguments,
        stdout,
        stderr,
        (
            &ProductionModelsCommandHost,
            &ProductionDoctorCommandHost,
            &ProductionSessionCommandHost,
            &ProductionSessionsCommandHost,
            ask_host,
        ),
    )
}

fn run_with_hosts(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    hosts: (
        &impl ModelsCommandHost,
        &impl DoctorCommandHost,
        &impl SessionCommandHost,
        &impl SessionsCommandHost,
        &impl AskCommandHost,
    ),
) -> u8 {
    let (models_host, doctor_host, inspection_host, listing_host, ask_host) = hosts;
    let Ok(command) = parse_arguments(arguments) else {
        let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
        return 2;
    };

    let output = match command {
        Command::Identity => identity(),
        Command::Help => help(),
        Command::Ask { prompt } => {
            return run_ask(ask_host, prompt, stdout, stderr, OUTPUT_FAILURE);
        }
        Command::Doctor { json } => {
            return run_doctor(doctor_host, json, stdout, stderr);
        }
        Command::Models { json } => {
            return run_models(models_host, json, stdout, stderr);
        }
        Command::Permissions { json } => {
            let Ok(loaded) = load_process_config() else {
                let _ = stderr.write_all(CONFIGURATION_FAILURE.as_bytes());
                return 1;
            };
            permissions(loaded.config().permission_mode(), json)
        }
        Command::Resume { id, prompt } => {
            return run_resume(ask_host, id, prompt, stdout, stderr, OUTPUT_FAILURE);
        }
        Command::Session { id, json } => {
            return run_session(inspection_host, id, json, stdout, stderr);
        }
        Command::Sessions { json } => {
            return run_sessions(listing_host, json, stdout, stderr);
        }
        Command::Status { json } => status(&inspect_process_status(), json),
    };

    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn parse_arguments(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, ()> {
    let mut arguments = arguments.into_iter();
    let Some(first) = arguments.next() else {
        return Ok(Command::Identity);
    };
    let Some(first) = first.to_str() else {
        return Err(());
    };

    let command = match first {
        "help" | "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Identity,
        "ask" => Command::Ask {
            prompt: parse_prompt_arguments(arguments.by_ref())?,
        },
        "doctor" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Doctor { json }
        }
        "models" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Models { json }
        }
        "permissions" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Permissions { json }
        }
        "resume" => {
            let id = parse_explicit_session_id(arguments.next().ok_or(())?)?;
            Command::Resume {
                id,
                prompt: parse_prompt_arguments(arguments.by_ref())?,
            }
        }
        "session" => {
            let id = parse_explicit_session_id(arguments.next().ok_or(())?)?;
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Session { id, json }
        }
        "sessions" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Sessions { json }
        }
        "status" => {
            let json = match arguments.next() {
                None => false,
                Some(argument) if argument == "--json" => true,
                Some(_) => return Err(()),
            };
            Command::Status { json }
        }
        _ => return Err(()),
    };

    if arguments.next().is_some() {
        return Err(());
    }
    Ok(command)
}

fn parse_explicit_session_id(argument: OsString) -> Result<SessionId, ()> {
    let id = argument.into_string().map_err(|_| ())?;
    if id == "last" || id.starts_with('-') {
        return Err(());
    }
    SessionId::new(id).map_err(|_| ())
}

fn identity() -> String {
    format!(
        "machine-god {} (engine API {})\n",
        env!("CARGO_PKG_VERSION"),
        machine_god_native::supported_core_api_version()
    )
}

fn help() -> String {
    format!(
        concat!(
            "machine-god {}\n",
            "Embeddable coding-agent engine\n",
            "\n",
            "Usage:\n",
            "  machine-god\n",
            "  machine-god help\n",
            "  machine-god ask [--] <prompt...>\n",
            "  machine-god doctor [--json]\n",
            "  machine-god models [--json]\n",
            "  machine-god permissions [--json]\n",
            "  machine-god resume <id> [--] <prompt...>\n",
            "  machine-god session <id> [--json]\n",
            "  machine-god sessions [--json]\n",
            "  machine-god status [--json]\n",
            "\n",
            "Commands:\n",
            "  help         Show this help\n",
            "  ask          Run one noninteractive prompt\n",
            "  doctor       Run local health and preflight checks\n",
            "  models       List available models\n",
            "  permissions  Show the permission mode and rules\n",
            "  resume       Resume a saved session with one prompt\n",
            "  session      Inspect a saved session\n",
            "  sessions     List saved sessions\n",
            "  status       Show configuration and runtime information\n",
            "\n",
            "Options:\n",
            "  -h, --help       Show this help\n",
            "  -V, --version    Show version\n",
        ),
        env!("CARGO_PKG_VERSION")
    )
}

fn run_doctor(
    host: &impl DoctorCommandHost,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let Ok(report) = host.inspect_doctor() else {
        let _ = stderr.write_all(DOCTOR_RENDER_FAILURE.as_bytes());
        return 1;
    };
    let Ok(output) = render_doctor(&report, json) else {
        let _ = stderr.write_all(DOCTOR_RENDER_FAILURE.as_bytes());
        return 1;
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn render_doctor(report: &DoctorReportSnapshot, json: bool) -> Result<String, ()> {
    if report.checks.len() != DOCTOR_CHECK_COUNT {
        return Err(());
    }
    let mut ok_count = 0usize;
    let mut warn_count = 0usize;
    let mut fail_count = 0usize;
    for check in &report.checks {
        match check.status {
            DoctorCheckStatus::Ok => ok_count += 1,
            DoctorCheckStatus::Warn => warn_count += 1,
            DoctorCheckStatus::Fail => fail_count += 1,
        }
    }
    if (report.ok_count, report.warn_count, report.fail_count) != (ok_count, warn_count, fail_count)
    {
        return Err(());
    }

    let mut output = BoundedDoctorOutput::new();
    let rendered = if json {
        write_json_doctor(&mut output, report)
    } else {
        write_human_doctor(&mut output, report)
    };
    rendered.map_err(|_| ())?;
    Ok(output.finish())
}

fn write_human_doctor(
    output: &mut BoundedDoctorOutput,
    report: &DoctorReportSnapshot,
) -> std::fmt::Result {
    writeln!(
        output,
        "[doctor] ok={} warn={} fail={}",
        report.ok_count, report.warn_count, report.fail_count
    )?;
    for check in &report.checks {
        writeln!(
            output,
            "[{}] {}: {}",
            check.status.as_str(),
            check.name,
            check.detail
        )?;
    }
    Ok(())
}

fn write_json_doctor(
    output: &mut BoundedDoctorOutput,
    report: &DoctorReportSnapshot,
) -> std::fmt::Result {
    output.write_str("{\"kind\":\"doctor\",\"ok_count\":")?;
    write!(
        output,
        "{},\"warn_count\":{},\"fail_count\":{},\"checks\":[",
        report.ok_count, report.warn_count, report.fail_count
    )?;
    for (index, check) in report.checks.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        output.write_str("{\"name\":")?;
        write_json_string(output, check.name)?;
        output.write_str(",\"status\":")?;
        write_json_string(output, check.status.as_str())?;
        output.write_str(",\"detail\":")?;
        write_json_string(output, check.detail)?;
        output.write_char('}')?;
    }
    output.write_str("]}\n")
}

fn run_models(
    host: &impl ModelsCommandHost,
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
    let mut output = BoundedModelsOutput::new();
    let rendered = if json {
        write_json_models(&mut output, catalog)
    } else {
        write_human_models(&mut output, catalog)
    };
    rendered.map_err(|_| ModelsOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn write_human_models(
    output: &mut BoundedModelsOutput,
    catalog: &ModelCatalog,
) -> std::fmt::Result {
    let models = catalog.models();
    if models.is_empty() {
        output.write_str("[models] no models returned by gateway\n")?;
    } else {
        writeln!(output, "[models] {} available", models.len())?;
        for model in models {
            writeln!(output, " - {}", model.id())?;
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

fn write_json_models(output: &mut BoundedModelsOutput, catalog: &ModelCatalog) -> std::fmt::Result {
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

fn run_session(
    host: &impl SessionCommandHost,
    id: SessionId,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let requested_id = id.as_str().to_owned();
    let mut inspection = host.inspect_session(id);
    let mut context = Context::from_waker(Waker::noop());
    let result = match inspection.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => Err(SessionOperationalFailure::Unavailable),
    };
    let snapshot = match result {
        Ok(snapshot) if snapshot.id == requested_id => snapshot,
        Ok(_) => {
            return write_session_failure(
                SessionOperationalFailure::ResourceLimit,
                json,
                stdout,
                stderr,
            );
        }
        Err(failure) => return write_session_failure(failure, json, stdout, stderr),
    };
    let output = match render_session(&snapshot, json) {
        Ok(output) => output,
        Err(failure) => return write_session_failure(failure, json, stdout, stderr),
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn write_session_failure(
    failure: SessionOperationalFailure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let category = failure.category();
    if json {
        let mut output = BoundedSessionOutput::new();
        let rendered = (|| {
            output.write_str("{\"kind\":\"session\",\"error\":")?;
            write_json_string(
                &mut output,
                &format!("could not inspect session: {category}"),
            )?;
            output.write_str(",\"code\":")?;
            write_json_string(&mut output, category)?;
            output.write_str("}\n")
        })();
        let output = if rendered.is_ok() {
            output.finish()
        } else {
            concat!(
                "{\"kind\":\"session\",\"error\":",
                "\"could not inspect session: ResourceLimit\",",
                "\"code\":\"ResourceLimit\"}\n",
            )
            .to_owned()
        };
        if stdout.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    } else {
        let mut output = String::from("machine-god session: could not inspect session: ");
        output.push_str(category);
        output.push('\n');
        if stderr.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    }
    1
}

fn render_session(
    snapshot: &SessionSnapshot,
    json: bool,
) -> Result<String, SessionOperationalFailure> {
    validate_session_snapshot(snapshot)?;
    let mut output = BoundedSessionOutput::new();
    let rendered = if json {
        write_json_session(&mut output, snapshot)
    } else {
        write_human_session(&mut output, snapshot)
    };
    rendered.map_err(|_| SessionOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn validate_session_snapshot(snapshot: &SessionSnapshot) -> Result<(), SessionOperationalFailure> {
    if SessionId::new(snapshot.id.clone()).is_err()
        || SessionIncarnationId::new(snapshot.incarnation_id.clone()).is_err()
        || snapshot.revision == 0
        || snapshot.next_turn_sequence == 0
    {
        return Err(SessionOperationalFailure::ResourceLimit);
    }
    Ok(())
}

fn write_human_session(
    output: &mut BoundedSessionOutput,
    snapshot: &SessionSnapshot,
) -> std::fmt::Result {
    writeln!(output, "[session] {}", snapshot.id)?;
    writeln!(output, " - incarnation_id: {}", snapshot.incarnation_id)?;
    writeln!(output, " - revision: {}", snapshot.revision)?;
    writeln!(
        output,
        " - next_turn_sequence: {}",
        snapshot.next_turn_sequence
    )?;
    writeln!(output, " - message_count: {}", snapshot.message_count)?;
    writeln!(
        output,
        " - metadata_entry_count: {}",
        snapshot.metadata_entry_count
    )
}

fn write_json_session(
    output: &mut BoundedSessionOutput,
    snapshot: &SessionSnapshot,
) -> std::fmt::Result {
    output.write_str("{\"kind\":\"session\",\"id\":")?;
    write_json_string(output, &snapshot.id)?;
    output.write_str(",\"incarnation_id\":")?;
    write_json_string(output, &snapshot.incarnation_id)?;
    write!(
        output,
        ",\"revision\":{},\"next_turn_sequence\":{},\"message_count\":{},\"metadata_entry_count\":{}",
        snapshot.revision,
        snapshot.next_turn_sequence,
        snapshot.message_count,
        snapshot.metadata_entry_count,
    )?;
    output.write_str("}\n")
}

fn run_sessions(
    host: &impl SessionsCommandHost,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let mut listing = host.list_sessions();
    let mut context = Context::from_waker(Waker::noop());
    let result = match listing.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => Err(SessionsOperationalFailure::Unavailable),
    };
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(failure) => return write_sessions_failure(failure, json, stdout, stderr),
    };
    let output = match render_sessions(&snapshot, json) {
        Ok(output) => output,
        Err(failure) => return write_sessions_failure(failure, json, stdout, stderr),
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn write_sessions_failure(
    failure: SessionsOperationalFailure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let category = failure.category();
    if json {
        let mut output = String::from("{\"kind\":\"sessions\",\"error\":");
        let detail = format!("could not list sessions: {category}");
        push_json_string(&mut output, &detail);
        output.push_str(",\"code\":");
        push_json_string(&mut output, category);
        output.push_str("}\n");
        if stdout.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    } else {
        let mut output = String::from("machine-god sessions: could not list sessions: ");
        output.push_str(category);
        output.push('\n');
        if stderr.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    }
    1
}

fn render_sessions(
    snapshot: &SessionsSnapshot,
    json: bool,
) -> Result<String, SessionsOperationalFailure> {
    validate_sessions_snapshot(snapshot)?;
    let mut output = BoundedSessionsOutput::new();
    let rendered = if json {
        write_json_sessions(&mut output, snapshot)
    } else {
        write_human_sessions(&mut output, snapshot)
    };
    rendered.map_err(|_| SessionsOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn validate_sessions_snapshot(
    snapshot: &SessionsSnapshot,
) -> Result<(), SessionsOperationalFailure> {
    if snapshot.session_ids.len() > MAX_LIST_SESSIONS {
        return Err(SessionsOperationalFailure::ResourceLimit);
    }
    let mut previous: Option<&str> = None;
    for session_id in &snapshot.session_ids {
        if SessionId::new(session_id.clone()).is_err()
            || previous.is_some_and(|previous| previous >= session_id.as_str())
        {
            return Err(SessionsOperationalFailure::ResourceLimit);
        }
        previous = Some(session_id);
    }
    Ok(())
}

fn write_human_sessions(
    output: &mut BoundedSessionsOutput,
    snapshot: &SessionsSnapshot,
) -> std::fmt::Result {
    if snapshot.session_ids.is_empty() && !snapshot.truncated {
        return output.write_str("[sessions] no saved sessions\n");
    }
    writeln!(output, "[sessions] {} saved", snapshot.session_ids.len())?;
    for session_id in &snapshot.session_ids {
        writeln!(output, " - {session_id}")?;
    }
    if snapshot.truncated {
        output.write_str("[sessions] listing incomplete: a resource limit was reached\n")?;
    }
    Ok(())
}

fn write_json_sessions(
    output: &mut BoundedSessionsOutput,
    snapshot: &SessionsSnapshot,
) -> std::fmt::Result {
    write!(
        output,
        "{{\"kind\":\"sessions\",\"count\":{},\"truncated\":{},\"sessions\":[",
        snapshot.session_ids.len(),
        snapshot.truncated,
    )?;
    for (index, session_id) in snapshot.session_ids.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        output.write_str("{\"id\":")?;
        write_json_string(output, session_id)?;
        output.write_char('}')?;
    }
    output.write_str("]}\n")
}

fn permissions(permission_mode: PermissionMode, json: bool) -> String {
    if json {
        json_permissions(permission_mode)
    } else {
        human_permissions(permission_mode)
    }
}

fn human_permissions(permission_mode: PermissionMode) -> String {
    let mut output = identity();
    let _ = writeln!(output, "permission_mode: {}", permission_mode.as_str());
    output.push_str("persistent_rules: unsupported\n");
    output.push_str("runtime_grants: unavailable\n");
    output
}

fn json_permissions(permission_mode: PermissionMode) -> String {
    let mut output = String::from("{\"name\":\"machine-god\",\"version\":");
    push_json_string(&mut output, env!("CARGO_PKG_VERSION"));
    let _ = write!(
        output,
        ",\"engine_api_version\":{},\"kind\":\"permissions\",\"permission_mode\":",
        machine_god_native::supported_core_api_version()
    );
    push_json_string(&mut output, permission_mode.as_str());
    output.push_str(",\"persistent_rules_supported\":false,\"runtime_grants_available\":false}\n");
    output
}

fn status(status: &NativeStatus, json: bool) -> String {
    if json {
        json_status(status)
    } else {
        human_status(status)
    }
}

fn human_status(status: &NativeStatus) -> String {
    let mut output = identity();
    let _ = writeln!(
        output,
        "permission_mode: {}",
        status.permission_mode().as_str()
    );
    let _ = write!(
        output,
        "config_file: state={} path=",
        status.config_file_state().as_str()
    );
    push_json_path(&mut output, status.config_file_path());
    output.push('\n');
    let _ = write!(
        output,
        "state_directory: state={} path=",
        status.state_directory_state().as_str()
    );
    push_json_path(&mut output, status.state_directory_path());
    output.push('\n');
    output
}

fn json_status(status: &NativeStatus) -> String {
    let mut output = String::from("{\"name\":\"machine-god\",\"version\":");
    push_json_string(&mut output, env!("CARGO_PKG_VERSION"));
    let _ = write!(
        output,
        ",\"engine_api_version\":{},\"permission_mode\":",
        machine_god_native::supported_core_api_version()
    );
    push_json_string(&mut output, status.permission_mode().as_str());
    output.push_str(",\"config_file\":{\"path\":");
    push_json_path(&mut output, status.config_file_path());
    output.push_str(",\"state\":");
    push_json_string(&mut output, status.config_file_state().as_str());
    output.push_str("},\"state_directory\":{\"path\":");
    push_json_path(&mut output, status.state_directory_path());
    output.push_str(",\"state\":");
    push_json_string(&mut output, status.state_directory_state().as_str());
    output.push_str("}}\n");
    output
}

fn push_json_path(output: &mut String, path: Option<&Path>) {
    if let Some(path) = path.and_then(Path::to_str) {
        push_json_string(output, path);
    } else {
        output.push_str("null");
    }
}

fn push_json_string(output: &mut String, value: &str) {
    write_json_string(output, value).expect("writing JSON to a String cannot fail");
}

fn write_json_string(output: &mut impl std::fmt::Write, value: &str) -> std::fmt::Result {
    output.write_char('"')?;
    for character in value.chars() {
        match character {
            '"' => output.write_str("\\\"")?,
            '\\' => output.write_str("\\\\")?,
            '\u{08}' => output.write_str("\\b")?,
            '\u{0c}' => output.write_str("\\f")?,
            '\n' => output.write_str("\\n")?,
            '\r' => output.write_str("\\r")?,
            '\t' => output.write_str("\\t")?,
            '\u{00}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{061c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => {
                write!(output, "\\u{:04x}", character as u32)?;
            }
            _ => output.write_char(character)?,
        }
    }
    output.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedSessionOutput, BoundedSessionsOutput, Command, DOCTOR_RENDER_FAILURE,
        DoctorCheckSnapshot, DoctorCheckStatus, DoctorCommandHost, DoctorReportSnapshot,
        INVALID_ARGUMENTS, MAX_DOCTOR_OUTPUT_BYTES, MAX_SESSION_OUTPUT_BYTES,
        MAX_SESSIONS_OUTPUT_BYTES, ModelsCommandExecution, ModelsCommandHost,
        ModelsOperationalFailure, OUTPUT_FAILURE, PermissionMode, SessionCommandHost,
        SessionOperationalFailure, SessionSnapshot, SessionsCommandHost,
        SessionsOperationalFailure, SessionsSnapshot, classify_session_inspection_error_kind,
        classify_session_listing_error_kind, help, json_permissions, parse_arguments, permissions,
        push_json_string, render_doctor, render_session, render_sessions, run, run_with_ask_host,
        run_with_doctor_host, run_with_models_host, run_with_session_host, run_with_sessions_host,
    };
    #[cfg(not(target_family = "wasm"))]
    use super::{ModelsCompositionEffects, classify_provider_error, list_models_with_effects};
    #[cfg(unix)]
    use super::{
        ModelsSignalEvent, ModelsSignalKind, ModelsSignalSource, PendingModelsSignalGuardian,
        TokioModelsSignalSource, list_models_with_signal_source, list_models_with_signals,
        run_models, terminate_signal_event,
    };
    use crate::ask::{
        AskCommandExecution, AskCommandHost, AskCommandOutcome, MAX_ASK_PROMPT_BYTES,
        SessionSelection,
    };
    use machine_god_core::{
        AvailableModel, BoxFuture, ModelCatalog, ModelCatalogAccess, PublicCatalogReason,
    };
    #[cfg(unix)]
    use machine_god_core::{CancellationToken, ModelCatalogProvider};
    #[cfg(not(target_family = "wasm"))]
    use machine_god_core::{ProviderError, ProviderErrorKind};
    use machine_god_native::{NativeSessionInspectionErrorKind, NativeSessionListingErrorKind};
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::fmt::Write as _;
    #[cfg(unix)]
    use std::future::Future;
    use std::io;
    #[cfg(unix)]
    use std::io::Write as _;
    #[cfg(unix)]
    use std::pin::Pin;
    #[cfg(unix)]
    use std::sync::{Arc, Mutex};
    #[cfg(unix)]
    use std::task::{Context, Poll};
    #[cfg(unix)]
    use std::time::Duration;

    #[derive(Clone, Debug)]
    struct FakeModelsHost {
        result: Result<ModelCatalog, ModelsOperationalFailure>,
        calls: Cell<usize>,
    }

    impl FakeModelsHost {
        fn new(result: Result<ModelCatalog, ModelsOperationalFailure>) -> Self {
            Self {
                result,
                calls: Cell::new(0),
            }
        }
    }

    impl ModelsCommandHost for FakeModelsHost {
        fn list_models(&self) -> ModelsCommandExecution {
            self.calls.set(self.calls.get() + 1);
            ModelsCommandExecution::without_signal_guard(self.result.clone())
        }
    }

    #[derive(Debug)]
    struct FakeAskHost {
        outcome: AskCommandOutcome,
        calls: Cell<usize>,
        selections: RefCell<Vec<Option<String>>>,
        prompts: RefCell<Vec<String>>,
        output: &'static [u8],
    }

    impl FakeAskHost {
        fn new(outcome: AskCommandOutcome, output: &'static [u8]) -> Self {
            Self {
                outcome,
                calls: Cell::new(0),
                selections: RefCell::new(Vec::new()),
                prompts: RefCell::new(Vec::new()),
                output,
            }
        }
    }

    impl AskCommandHost for FakeAskHost {
        fn execute(
            &self,
            selection: SessionSelection,
            prompt: String,
            output: &mut dyn io::Write,
        ) -> AskCommandExecution {
            self.calls.set(self.calls.get() + 1);
            self.selections.borrow_mut().push(match selection {
                SessionSelection::CreateGenerated => None,
                SessionSelection::Resume(id) => Some(id.as_str().to_owned()),
            });
            self.prompts.borrow_mut().push(prompt);
            let outcome = if output.write_all(self.output).is_err() {
                AskCommandOutcome::OutputFailure
            } else {
                self.outcome
            };
            AskCommandExecution::without_finalizer(outcome)
        }
    }

    #[derive(Clone, Debug)]
    struct FakeDoctorHost {
        result: Result<DoctorReportSnapshot, ()>,
        calls: Cell<usize>,
    }

    impl FakeDoctorHost {
        fn new(result: Result<DoctorReportSnapshot, ()>) -> Self {
            Self {
                result,
                calls: Cell::new(0),
            }
        }
    }

    impl DoctorCommandHost for FakeDoctorHost {
        fn inspect_doctor(&self) -> Result<DoctorReportSnapshot, ()> {
            self.calls.set(self.calls.get() + 1);
            self.result
        }
    }

    #[derive(Clone, Debug)]
    struct FakeSessionHost {
        result: Result<SessionSnapshot, SessionOperationalFailure>,
        calls: Cell<usize>,
        requested_ids: RefCell<Vec<String>>,
        pending: bool,
    }

    impl FakeSessionHost {
        fn ready(result: Result<SessionSnapshot, SessionOperationalFailure>) -> Self {
            Self {
                result,
                calls: Cell::new(0),
                requested_ids: RefCell::new(Vec::new()),
                pending: false,
            }
        }

        fn pending() -> Self {
            Self {
                result: Err(SessionOperationalFailure::Unavailable),
                calls: Cell::new(0),
                requested_ids: RefCell::new(Vec::new()),
                pending: true,
            }
        }
    }

    impl SessionCommandHost for FakeSessionHost {
        fn inspect_session(
            &self,
            id: machine_god_core::SessionId,
        ) -> BoxFuture<'static, Result<SessionSnapshot, SessionOperationalFailure>> {
            self.calls.set(self.calls.get() + 1);
            self.requested_ids.borrow_mut().push(id.as_str().to_owned());
            if self.pending {
                Box::pin(std::future::pending())
            } else {
                Box::pin(std::future::ready(self.result.clone()))
            }
        }
    }

    #[derive(Clone, Debug)]
    struct FakeSessionsHost {
        result: Result<SessionsSnapshot, SessionsOperationalFailure>,
        calls: Cell<usize>,
        pending: bool,
    }

    impl FakeSessionsHost {
        fn ready(result: Result<SessionsSnapshot, SessionsOperationalFailure>) -> Self {
            Self {
                result,
                calls: Cell::new(0),
                pending: false,
            }
        }

        fn pending() -> Self {
            Self {
                result: Err(SessionsOperationalFailure::Unavailable),
                calls: Cell::new(0),
                pending: true,
            }
        }
    }

    impl SessionsCommandHost for FakeSessionsHost {
        fn list_sessions(
            &self,
        ) -> BoxFuture<'static, Result<SessionsSnapshot, SessionsOperationalFailure>> {
            self.calls.set(self.calls.get() + 1);
            if self.pending {
                Box::pin(std::future::pending())
            } else {
                Box::pin(std::future::ready(self.result.clone()))
            }
        }
    }

    fn sessions_snapshot(session_ids: &[&str], truncated: bool) -> SessionsSnapshot {
        SessionsSnapshot {
            session_ids: session_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            truncated,
        }
    }

    fn session_snapshot(id: &str) -> SessionSnapshot {
        SessionSnapshot {
            id: id.to_owned(),
            incarnation_id: format!("incarnation-{id}"),
            revision: 7,
            next_turn_sequence: 4,
            message_count: 3,
            metadata_entry_count: 2,
        }
    }

    fn doctor_report(
        checks: [DoctorCheckSnapshot; super::DOCTOR_CHECK_COUNT],
    ) -> DoctorReportSnapshot {
        let ok_count = checks
            .iter()
            .filter(|check| check.status == DoctorCheckStatus::Ok)
            .count();
        let warn_count = checks
            .iter()
            .filter(|check| check.status == DoctorCheckStatus::Warn)
            .count();
        let fail_count = checks
            .iter()
            .filter(|check| check.status == DoctorCheckStatus::Fail)
            .count();
        DoctorReportSnapshot {
            ok_count,
            warn_count,
            fail_count,
            checks,
        }
    }

    fn check(
        name: &'static str,
        status: DoctorCheckStatus,
        detail: &'static str,
    ) -> DoctorCheckSnapshot {
        DoctorCheckSnapshot {
            name,
            status,
            detail,
        }
    }

    fn leaked(value: String) -> &'static str {
        Box::leak(value.into_boxed_str())
    }

    #[cfg(not(target_family = "wasm"))]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CompositionEffect {
        LoadConfig,
        DiscoverCredential,
        CreateTransportAndList,
    }

    #[cfg(not(target_family = "wasm"))]
    #[derive(Debug)]
    struct FakeModelsCompositionEffects {
        trace: RefCell<Vec<CompositionEffect>>,
        config_result: Result<(), ModelsOperationalFailure>,
        credential_result: Result<(), ModelsOperationalFailure>,
        catalog_result: Result<ModelCatalog, ModelsOperationalFailure>,
    }

    #[cfg(not(target_family = "wasm"))]
    impl ModelsCompositionEffects for FakeModelsCompositionEffects {
        type Credential = ();

        fn load_and_validate_config(&self) -> Result<(), ModelsOperationalFailure> {
            self.trace.borrow_mut().push(CompositionEffect::LoadConfig);
            self.config_result
        }

        fn discover_credential(&self) -> Result<Self::Credential, ModelsOperationalFailure> {
            self.trace
                .borrow_mut()
                .push(CompositionEffect::DiscoverCredential);
            self.credential_result
        }

        fn create_transport_and_list(&self, (): Self::Credential) -> ModelsCommandExecution {
            self.trace
                .borrow_mut()
                .push(CompositionEffect::CreateTransportAndList);
            ModelsCommandExecution::without_signal_guard(self.catalog_result.clone())
        }
    }

    #[cfg(unix)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum SignalTrace {
        RegisterInterrupt,
        RegisterTerminate,
        PollInterrupt,
        PollTerminate,
        CreateProviderFuture,
        PollProviderFuture,
        DropInterrupt,
        DropTerminate,
        DropProviderFuture,
    }

    #[cfg(unix)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ScriptedSignal {
        Pending,
        ReadyOnPoll {
            poll: usize,
            event: ModelsSignalEvent,
        },
    }

    #[cfg(unix)]
    struct ScriptedSignalFuture {
        trace: Arc<Mutex<Vec<SignalTrace>>>,
        poll_event: SignalTrace,
        drop_event: SignalTrace,
        script: ScriptedSignal,
        polls: usize,
    }

    #[cfg(unix)]
    impl Future for ScriptedSignalFuture {
        type Output = ModelsSignalEvent;

        fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            self.polls += 1;
            self.trace.lock().unwrap().push(self.poll_event);
            match self.script {
                ScriptedSignal::ReadyOnPoll { poll, event } if self.polls >= poll => {
                    Poll::Ready(event)
                }
                ScriptedSignal::Pending | ScriptedSignal::ReadyOnPoll { .. } => Poll::Pending,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for ScriptedSignalFuture {
        fn drop(&mut self) {
            self.trace.lock().unwrap().push(self.drop_event);
        }
    }

    #[cfg(unix)]
    struct FakeModelsSignalSource {
        trace: Arc<Mutex<Vec<SignalTrace>>>,
        interrupt: ScriptedSignalFuture,
        terminate: ScriptedSignalFuture,
        fail_interrupt_registration: bool,
        fail_terminate_registration: bool,
    }

    #[cfg(unix)]
    impl FakeModelsSignalSource {
        fn new(trace: Arc<Mutex<Vec<SignalTrace>>>, interrupt: ScriptedSignal) -> Self {
            Self {
                interrupt: ScriptedSignalFuture {
                    trace: Arc::clone(&trace),
                    poll_event: SignalTrace::PollInterrupt,
                    drop_event: SignalTrace::DropInterrupt,
                    script: interrupt,
                    polls: 0,
                },
                terminate: ScriptedSignalFuture {
                    trace: Arc::clone(&trace),
                    poll_event: SignalTrace::PollTerminate,
                    drop_event: SignalTrace::DropTerminate,
                    script: ScriptedSignal::Pending,
                    polls: 0,
                },
                trace,
                fail_interrupt_registration: false,
                fail_terminate_registration: false,
            }
        }
    }

    #[cfg(unix)]
    impl ModelsSignalSource for FakeModelsSignalSource {
        fn registration_failed(&self) -> bool {
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::RegisterInterrupt);
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::RegisterTerminate);
            self.fail_interrupt_registration || self.fail_terminate_registration
        }

        fn poll_interrupt(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent> {
            Pin::new(&mut self.interrupt).poll(context)
        }

        fn poll_terminate(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent> {
            Pin::new(&mut self.terminate).poll(context)
        }
    }

    #[cfg(unix)]
    #[derive(Clone, Debug)]
    enum FakeProviderResult {
        Ready(Result<ModelCatalog, ProviderError>),
    }

    #[cfg(unix)]
    #[derive(Clone, Debug)]
    struct FakeSignalProvider {
        trace: Arc<Mutex<Vec<SignalTrace>>>,
        result: FakeProviderResult,
        cancelled_when_dropped: Arc<Mutex<Option<bool>>>,
    }

    #[cfg(unix)]
    struct FakeProviderFuture {
        trace: Arc<Mutex<Vec<SignalTrace>>>,
        result: FakeProviderResult,
        cancellation: CancellationToken,
        cancelled_when_dropped: Arc<Mutex<Option<bool>>>,
    }

    #[cfg(unix)]
    impl Future for FakeProviderFuture {
        type Output = Result<ModelCatalog, ProviderError>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::PollProviderFuture);
            match &self.result {
                FakeProviderResult::Ready(result) => Poll::Ready(result.clone()),
            }
        }
    }

    #[cfg(unix)]
    impl Drop for FakeProviderFuture {
        fn drop(&mut self) {
            *self.cancelled_when_dropped.lock().unwrap() = Some(self.cancellation.is_cancelled());
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::DropProviderFuture);
        }
    }

    #[cfg(unix)]
    impl ModelCatalogProvider for FakeSignalProvider {
        fn name(&self) -> &'static str {
            "fake-signal-provider"
        }

        fn list_models(
            &self,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::CreateProviderFuture);
            Box::pin(FakeProviderFuture {
                trace: Arc::clone(&self.trace),
                result: self.result.clone(),
                cancellation,
                cancelled_when_dropped: Arc::clone(&self.cancelled_when_dropped),
            })
        }
    }

    fn catalog(ids: &[&str], access: ModelCatalogAccess) -> ModelCatalog {
        ModelCatalog::new(
            ids.iter()
                .map(|id| AvailableModel::new(*id).expect("valid model ID"))
                .collect(),
            access,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    fn composition_effects(
        config_result: Result<(), ModelsOperationalFailure>,
        credential_result: Result<(), ModelsOperationalFailure>,
    ) -> FakeModelsCompositionEffects {
        FakeModelsCompositionEffects {
            trace: RefCell::new(Vec::new()),
            config_result,
            credential_result,
            catalog_result: Ok(catalog(
                &["provider/model"],
                ModelCatalogAccess::Authenticated,
            )),
        }
    }

    #[cfg(unix)]
    fn signal_provider(
        trace: &Arc<Mutex<Vec<SignalTrace>>>,
        result: FakeProviderResult,
    ) -> FakeSignalProvider {
        FakeSignalProvider {
            trace: Arc::clone(trace),
            result,
            cancelled_when_dropped: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(unix)]
    fn run_signal_coordination(
        provider: &FakeSignalProvider,
        signals: FakeModelsSignalSource,
    ) -> Result<ModelCatalog, ProviderError> {
        let phase = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(list_models_with_signal_source(provider, signals));
        phase.result
    }

    #[cfg(unix)]
    fn coordination_trace(trace: &Arc<Mutex<Vec<SignalTrace>>>) -> Vec<SignalTrace> {
        trace
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|event| {
                !matches!(
                    event,
                    SignalTrace::DropInterrupt
                        | SignalTrace::DropTerminate
                        | SignalTrace::DropProviderFuture
                )
            })
            .collect()
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn models_composition_orders_each_effect_once_and_short_circuits_failures() {
        let success = composition_effects(Ok(()), Ok(()));
        assert!(list_models_with_effects(&success).result().is_ok());
        assert_eq!(
            *success.trace.borrow(),
            [
                CompositionEffect::LoadConfig,
                CompositionEffect::DiscoverCredential,
                CompositionEffect::CreateTransportAndList,
            ]
        );

        let config_failure = composition_effects(
            Err(ModelsOperationalFailure::Unavailable),
            Err(ModelsOperationalFailure::Unavailable),
        );
        assert_eq!(
            list_models_with_effects(&config_failure).result(),
            &Err(ModelsOperationalFailure::Unavailable)
        );
        assert_eq!(
            *config_failure.trace.borrow(),
            [CompositionEffect::LoadConfig]
        );

        let credential_failure =
            composition_effects(Ok(()), Err(ModelsOperationalFailure::Unavailable));
        assert_eq!(
            list_models_with_effects(&credential_failure).result(),
            &Err(ModelsOperationalFailure::Unavailable)
        );
        assert_eq!(
            *credential_failure.trace.borrow(),
            [
                CompositionEffect::LoadConfig,
                CompositionEffect::DiscoverCredential,
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn model_signal_listeners_are_registered_and_polled_before_provider_dispatch() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let provider = signal_provider(
            &trace,
            FakeProviderResult::Ready(Ok(catalog(
                &["provider/model"],
                ModelCatalogAccess::Authenticated,
            ))),
        );
        let signals = FakeModelsSignalSource::new(Arc::clone(&trace), ScriptedSignal::Pending);

        assert!(run_signal_coordination(&provider, signals).is_ok());
        assert_eq!(
            coordination_trace(&trace),
            [
                SignalTrace::RegisterInterrupt,
                SignalTrace::RegisterTerminate,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
                SignalTrace::CreateProviderFuture,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
                SignalTrace::PollProviderFuture,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
            ]
        );
        let trace = trace.lock().unwrap();
        assert_eq!(
            trace
                .iter()
                .filter(|event| **event == SignalTrace::DropInterrupt)
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|event| **event == SignalTrace::DropTerminate)
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|event| **event == SignalTrace::DropProviderFuture)
                .count(),
            1
        );
        assert_eq!(
            *provider.cancelled_when_dropped.lock().unwrap(),
            Some(false)
        );
    }

    #[cfg(unix)]
    #[test]
    fn ready_signal_wins_same_poll_provider_success_and_drops_cancelled_provider() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let provider = signal_provider(
            &trace,
            FakeProviderResult::Ready(Ok(catalog(
                &["provider/model"],
                ModelCatalogAccess::Authenticated,
            ))),
        );
        let signals = FakeModelsSignalSource::new(
            Arc::clone(&trace),
            ScriptedSignal::ReadyOnPoll {
                poll: 3,
                event: ModelsSignalEvent::Received(ModelsSignalKind::Interrupt),
            },
        );

        let error = run_signal_coordination(&provider, signals).unwrap_err();

        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
        assert_eq!(error.code, "Cancelled");
        assert_eq!(
            coordination_trace(&trace),
            [
                SignalTrace::RegisterInterrupt,
                SignalTrace::RegisterTerminate,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
                SignalTrace::CreateProviderFuture,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
                SignalTrace::PollProviderFuture,
                SignalTrace::PollInterrupt,
            ]
        );
        assert_eq!(*provider.cancelled_when_dropped.lock().unwrap(), Some(true));
    }

    #[cfg(unix)]
    #[test]
    fn signal_registration_and_wait_failures_are_authoritative() {
        let registration_trace = Arc::new(Mutex::new(Vec::new()));
        let registration_provider = signal_provider(
            &registration_trace,
            FakeProviderResult::Ready(Ok(catalog(
                &["provider/model"],
                ModelCatalogAccess::Authenticated,
            ))),
        );
        let mut registration_signals =
            FakeModelsSignalSource::new(Arc::clone(&registration_trace), ScriptedSignal::Pending);
        registration_signals.fail_terminate_registration = true;

        let registration_error =
            run_signal_coordination(&registration_provider, registration_signals).unwrap_err();
        assert_eq!(registration_error.kind, ProviderErrorKind::Unavailable);
        assert_eq!(registration_error.code, "SignalUnavailable");
        assert_eq!(
            coordination_trace(&registration_trace),
            [
                SignalTrace::RegisterInterrupt,
                SignalTrace::RegisterTerminate,
            ]
        );
        assert_eq!(
            *registration_provider.cancelled_when_dropped.lock().unwrap(),
            None
        );

        let wait_trace = Arc::new(Mutex::new(Vec::new()));
        let wait_provider = signal_provider(
            &wait_trace,
            FakeProviderResult::Ready(Ok(catalog(
                &["provider/model"],
                ModelCatalogAccess::Authenticated,
            ))),
        );
        let mut wait_signals =
            FakeModelsSignalSource::new(Arc::clone(&wait_trace), ScriptedSignal::Pending);
        wait_signals.terminate.script = ScriptedSignal::ReadyOnPoll {
            poll: 3,
            event: ModelsSignalEvent::WaitFailed,
        };

        let wait_error = run_signal_coordination(&wait_provider, wait_signals).unwrap_err();
        assert_eq!(wait_error.kind, ProviderErrorKind::Unavailable);
        assert_eq!(wait_error.code, "SignalUnavailable");
        assert_eq!(
            coordination_trace(&wait_trace),
            [
                SignalTrace::RegisterInterrupt,
                SignalTrace::RegisterTerminate,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
                SignalTrace::CreateProviderFuture,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
                SignalTrace::PollProviderFuture,
                SignalTrace::PollInterrupt,
                SignalTrace::PollTerminate,
            ]
        );
        assert_eq!(
            *wait_provider.cancelled_when_dropped.lock().unwrap(),
            Some(true)
        );
        assert_eq!(
            terminate_signal_event(Some(())),
            ModelsSignalEvent::Received(ModelsSignalKind::Terminate)
        );
        assert_eq!(terminate_signal_event(None), ModelsSignalEvent::WaitFailed);
    }

    #[cfg(unix)]
    struct SignalOutputChildProvider;

    #[cfg(unix)]
    struct SignalOutputChildFuture {
        cancellation: CancellationToken,
        announced: bool,
    }

    #[cfg(unix)]
    impl Future for SignalOutputChildFuture {
        type Output = Result<ModelCatalog, ProviderError>;

        fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            if !self.announced {
                self.announced = true;
                let mut stderr = io::stderr().lock();
                stderr.write_all(b"PROVIDER_READY\n").unwrap();
                stderr.flush().unwrap();
            }
            Poll::Pending
        }
    }

    #[cfg(unix)]
    impl Drop for SignalOutputChildFuture {
        fn drop(&mut self) {
            let marker = if self.cancellation.is_cancelled() {
                b"PROVIDER_DROPPED_CANCELLED\n".as_slice()
            } else {
                b"PROVIDER_DROPPED_WITHOUT_CANCELLATION\n".as_slice()
            };
            let mut stderr = io::stderr().lock();
            let _ = stderr.write_all(marker);
            let _ = stderr.flush();
        }
    }

    #[cfg(unix)]
    impl ModelCatalogProvider for SignalOutputChildProvider {
        fn name(&self) -> &'static str {
            "signal-output-child"
        }

        fn list_models(
            &self,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
            Box::pin(SignalOutputChildFuture {
                cancellation,
                announced: false,
            })
        }
    }

    #[cfg(unix)]
    struct SignalOutputReadyChildProvider;

    #[cfg(unix)]
    impl ModelCatalogProvider for SignalOutputReadyChildProvider {
        fn name(&self) -> &'static str {
            "signal-output-ready-child"
        }

        fn list_models(
            &self,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
            Box::pin(async { Ok(catalog(&[], ModelCatalogAccess::Authenticated)) })
        }
    }

    #[cfg(unix)]
    struct SingleExecutionHost {
        execution: RefCell<Option<ModelsCommandExecution>>,
    }

    #[cfg(unix)]
    impl ModelsCommandHost for SingleExecutionHost {
        fn list_models(&self) -> ModelsCommandExecution {
            self.execution
                .borrow_mut()
                .take()
                .expect("child execution is consumed once")
        }
    }

    #[cfg(unix)]
    struct OutputStartWriter {
        stdout: io::Stdout,
        announced: bool,
    }

    #[cfg(unix)]
    impl io::Write for OutputStartWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if !self.announced {
                self.announced = true;
                let mut stderr = io::stderr().lock();
                stderr.write_all(b"OUTPUT_START\n")?;
                stderr.flush()?;
            }
            self.stdout.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stdout.flush()
        }
    }

    #[cfg(unix)]
    #[test]
    fn models_signal_output_subprocess_child() {
        let Some(mode) = std::env::var_os("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD") else {
            return;
        };
        if mode == "wait-failed" {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .unwrap();
            let pending_guardian = PendingModelsSignalGuardian::spawn().unwrap();
            let signals = {
                let _entered = runtime.enter();
                TokioModelsSignalSource::register()
            };
            let _guard = pending_guardian.activate(runtime, signals, true);
            panic!("wait-failed guardian activation must terminate the process");
        }

        let execution = if mode == "stop-drain" {
            list_models_with_signals(&SignalOutputReadyChildProvider)
        } else {
            list_models_with_signals(&SignalOutputChildProvider)
        };
        let host = SingleExecutionHost {
            execution: RefCell::new(Some(execution)),
        };
        let mut stdout = OutputStartWriter {
            stdout: io::stdout(),
            announced: false,
        };
        let mut stderr = io::stderr();
        let code = run_models(&host, true, &mut stdout, &mut stderr);
        std::process::exit(i32::from(code));
    }

    #[cfg(unix)]
    fn send_process_signal(process_id: u32, signal: &str) {
        let status = std::process::Command::new("/bin/kill")
            .args([signal, &process_id.to_string()])
            .status()
            .expect("invoke /bin/kill");
        assert!(status.success(), "failed to send {signal} to {process_id}");
    }

    #[cfg(unix)]
    fn wait_for_child_marker(
        receiver: &std::sync::mpsc::Receiver<String>,
        expected: &str,
        process_id: u32,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let line = match receiver.recv_timeout(remaining) {
                Ok(line) => line,
                Err(error) => {
                    send_process_signal(process_id, "-KILL");
                    panic!("child did not emit {expected:?}: {error}");
                }
            };
            if line == expected {
                return;
            }
        }
    }

    #[cfg(unix)]
    fn assert_saturated_output_terminates_on(second_signal: &str, expected_exit: i32) {
        use std::io::{BufRead, Write};
        use std::os::unix::net::UnixStream;
        use std::process::Stdio;

        let (unread_stdout, child_stdout) = UnixStream::pair().expect("stdout socket pair");
        let mut filler = child_stdout.try_clone().expect("clone stdout sender");
        let child_stdout = std::os::fd::OwnedFd::from(child_stdout);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::models_signal_output_subprocess_child",
                "--nocapture",
            ])
            .env("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD", "1")
            .stdout(Stdio::from(child_stdout))
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn signal-output child");
        let process_id = child.id();
        let child_stderr = child.stderr.take().expect("child stderr");
        let (marker_sender, marker_receiver) = std::sync::mpsc::channel();
        let marker_worker = std::thread::spawn(move || {
            for line in io::BufReader::new(child_stderr).lines() {
                match line {
                    Ok(line) => {
                        if marker_sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        wait_for_child_marker(&marker_receiver, "PROVIDER_READY", process_id);
        filler.set_nonblocking(true).unwrap();
        let block = [b'x'; 8 * 1024];
        let mut filled_bytes = 0_usize;
        loop {
            match filler.write(&block) {
                Ok(0) => panic!("stdout socket stopped accepting bytes without saturation"),
                Ok(count) => filled_bytes += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("failed to saturate stdout socket: {error}"),
            }
        }
        assert!(filled_bytes > 0);
        filler.set_nonblocking(false).unwrap();

        send_process_signal(process_id, "-INT");
        wait_for_child_marker(&marker_receiver, "PROVIDER_DROPPED_CANCELLED", process_id);
        wait_for_child_marker(&marker_receiver, "OUTPUT_START", process_id);
        send_process_signal(process_id, second_signal);

        let (status_sender, status_receiver) = std::sync::mpsc::sync_channel(0);
        let wait_worker = std::thread::spawn(move || {
            let _ = status_sender.send(child.wait());
        });
        let status = match status_receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => panic!("failed to wait for signal-output child: {error}"),
            Err(error) => {
                send_process_signal(process_id, "-KILL");
                let _ = status_receiver.recv_timeout(Duration::from_secs(2));
                panic!("signal-output child required SIGKILL cleanup: {error}");
            }
        };
        wait_worker.join().unwrap();
        marker_worker.join().unwrap();
        drop(filler);
        drop(unread_stdout);
        assert_eq!(status.code(), Some(expected_exit));
    }

    #[cfg(unix)]
    #[test]
    fn repeated_signal_terminates_while_models_json_output_is_backpressured() {
        assert_saturated_output_terminates_on("-INT", 130);
        assert_saturated_output_terminates_on("-TERM", 143);
    }

    #[cfg(unix)]
    #[test]
    fn fast_output_stop_drain_observes_signal_before_guardian_shutdown() {
        use std::io::{BufRead, Write};
        use std::process::Stdio;

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::models_signal_output_subprocess_child",
                "--nocapture",
            ])
            .env("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD", "stop-drain")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn signal stop-drain child");
        let process_id = child.id();
        let child_stderr = child.stderr.take().expect("child stderr");
        let (marker_sender, marker_receiver) = std::sync::mpsc::channel();
        let marker_worker = std::thread::spawn(move || {
            for line in io::BufReader::new(child_stderr).lines() {
                match line {
                    Ok(line) => {
                        if marker_sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        wait_for_child_marker(&marker_receiver, "GUARDIAN_STOP_READY", process_id);
        send_process_signal(process_id, "-INT");
        child
            .stdin
            .take()
            .expect("child stdin")
            .write_all(b"x")
            .expect("release guardian stop drain");

        let (status_sender, status_receiver) = std::sync::mpsc::sync_channel(0);
        let wait_worker = std::thread::spawn(move || {
            let _ = status_sender.send(child.wait());
        });
        let status = match status_receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => panic!("failed to wait for signal stop-drain child: {error}"),
            Err(error) => {
                send_process_signal(process_id, "-KILL");
                let _ = status_receiver.recv_timeout(Duration::from_secs(2));
                panic!("signal stop-drain child required SIGKILL cleanup: {error}");
            }
        };
        wait_worker.join().unwrap();
        marker_worker.join().unwrap();
        assert_eq!(status.code(), Some(130));
    }

    #[cfg(unix)]
    #[test]
    fn signal_wait_failure_fail_stops_before_output() {
        use std::process::Stdio;

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::models_signal_output_subprocess_child",
                "--nocapture",
            ])
            .env("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD", "wait-failed")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn signal wait-failure child");
        let process_id = child.id();
        let (status_sender, status_receiver) = std::sync::mpsc::sync_channel(0);
        let wait_worker = std::thread::spawn(move || {
            let _ = status_sender.send(child.wait());
        });
        let status = match status_receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => panic!("failed to wait for wait-failure child: {error}"),
            Err(error) => {
                send_process_signal(process_id, "-KILL");
                let _ = status_receiver.recv_timeout(Duration::from_secs(2));
                panic!("wait-failure child required SIGKILL cleanup: {error}");
            }
        };
        wait_worker.join().unwrap();
        assert_eq!(status.code(), Some(1));
    }

    #[derive(Debug, Default)]
    struct BrokenWriter;

    impl io::Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct ZeroProgressWriter {
        captured: Vec<u8>,
        calls: usize,
    }

    impl io::Write for ZeroProgressWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            assert!(!buffer.is_empty());
            self.calls += 1;
            Ok(0)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct PartialThenBrokenWriter {
        prefix: Vec<u8>,
        prefix_limit: usize,
        accepted_first_write: bool,
    }

    impl PartialThenBrokenWriter {
        fn new(prefix_limit: usize) -> Self {
            assert!(prefix_limit > 0);
            Self {
                prefix: Vec::new(),
                prefix_limit,
                accepted_first_write: false,
            }
        }
    }

    impl io::Write for PartialThenBrokenWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if self.accepted_first_write {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }

            let accepted = buffer.len().min(self.prefix_limit);
            assert!(accepted > 0);
            self.prefix.extend_from_slice(&buffer[..accepted]);
            self.accepted_first_write = true;
            Ok(accepted)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FirstWriteFailsThenCaptures {
        captured: Vec<u8>,
        failed: bool,
    }

    impl io::Write for FirstWriteFailsThenCaptures {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if !self.failed {
                self.failed = true;
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            self.captured.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parser_accepts_only_the_documented_grammar() {
        assert_eq!(parse_arguments([]), Ok(Command::Identity));
        for alias in ["help", "--help", "-h"] {
            assert_eq!(parse_arguments([OsString::from(alias)]), Ok(Command::Help));
        }
        for alias in ["--version", "-V"] {
            assert_eq!(
                parse_arguments([OsString::from(alias)]),
                Ok(Command::Identity)
            );
        }
        assert_eq!(
            parse_arguments([OsString::from("doctor")]),
            Ok(Command::Doctor { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("doctor"), OsString::from("--json")]),
            Ok(Command::Doctor { json: true })
        );
        assert_eq!(
            parse_arguments([OsString::from("models")]),
            Ok(Command::Models { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("models"), OsString::from("--json")]),
            Ok(Command::Models { json: true })
        );
        assert_eq!(
            parse_arguments([OsString::from("permissions")]),
            Ok(Command::Permissions { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("permissions"), OsString::from("--json"),]),
            Ok(Command::Permissions { json: true })
        );
        assert_eq!(
            parse_arguments([OsString::from("sessions")]),
            Ok(Command::Sessions { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("sessions"), OsString::from("--json")]),
            Ok(Command::Sessions { json: true })
        );
        assert_eq!(
            parse_arguments([OsString::from("status")]),
            Ok(Command::Status { json: false })
        );
        assert_eq!(
            parse_arguments([OsString::from("status"), OsString::from("--json")]),
            Ok(Command::Status { json: true })
        );

        for arguments in [
            vec![OsString::from("unknown")],
            vec![OsString::from("help"), OsString::from("extra")],
            vec![OsString::from("--json"), OsString::from("status")],
            vec![OsString::from("doctor"), OsString::from("--json=true")],
            vec![
                OsString::from("doctor"),
                OsString::from("--json"),
                OsString::from("--json"),
            ],
            vec![OsString::from("doctor"), OsString::from("extra")],
            vec![OsString::from("models"), OsString::from("--json=true")],
            vec![
                OsString::from("models"),
                OsString::from("--json"),
                OsString::from("--json"),
            ],
            vec![OsString::from("permissions"), OsString::from("--json=true")],
            vec![
                OsString::from("permissions"),
                OsString::from("--json"),
                OsString::from("--json"),
            ],
            vec![OsString::from("sessions"), OsString::from("--json=true")],
            vec![
                OsString::from("sessions"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
            vec![OsString::from("status"), OsString::from("--json=true")],
            vec![
                OsString::from("status"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
        ] {
            assert_eq!(parse_arguments(arguments), Err(()));
        }
    }

    #[test]
    fn ask_parser_accepts_only_the_documented_top_level_grammar() {
        assert_eq!(
            parse_arguments([
                OsString::from("ask"),
                OsString::from("hello"),
                OsString::from("世界"),
            ]),
            Ok(Command::Ask {
                prompt: "hello 世界".to_owned(),
            })
        );
        assert_eq!(
            parse_arguments([
                OsString::from("ask"),
                OsString::from("--"),
                OsString::from("--flag"),
            ]),
            Ok(Command::Ask {
                prompt: "--flag".to_owned(),
            })
        );

        for arguments in [
            vec![OsString::from("ask")],
            vec![OsString::from("ask"), OsString::from("--")],
            vec![OsString::from("ask"), OsString::from("--flag")],
            vec![OsString::from("ask"), OsString::from(" \t\r\n")],
            vec![
                OsString::from("ask"),
                OsString::from("hello"),
                OsString::from("--"),
            ],
        ] {
            assert_eq!(parse_arguments(arguments), Err(()));
        }
    }

    #[test]
    fn session_parser_accepts_only_the_documented_grammar() {
        assert_eq!(
            parse_arguments([OsString::from("session"), OsString::from("alpha")]),
            Ok(Command::Session {
                id: machine_god_core::SessionId::new("alpha").unwrap(),
                json: false,
            })
        );
        assert_eq!(
            parse_arguments([
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
            ]),
            Ok(Command::Session {
                id: machine_god_core::SessionId::new("alpha").unwrap(),
                json: true,
            })
        );

        for arguments in [
            vec![OsString::from("session")],
            vec![OsString::from("session"), OsString::from("last")],
            vec![OsString::from("session"), OsString::from("--id")],
            vec![OsString::from("session"), OsString::from("--flag")],
            vec![
                OsString::from("session"),
                OsString::from("--id"),
                OsString::from("alpha"),
            ],
            vec![OsString::from("session"), OsString::from("--json")],
            vec![
                OsString::from("session"),
                OsString::from("--json"),
                OsString::from("alpha"),
            ],
            vec![
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json=true"),
            ],
            vec![
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
                OsString::from("--json"),
            ],
            vec![
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
            vec![OsString::from("session"), OsString::from("bad/session")],
            vec![OsString::from("session"), OsString::from("café")],
            vec![OsString::from("session"), OsString::from("a".repeat(129))],
        ] {
            assert_eq!(parse_arguments(arguments), Err(()));
        }
    }

    #[test]
    fn resume_parser_accepts_only_an_explicit_id_and_bounded_prompt() {
        assert_eq!(
            parse_arguments([
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("hello"),
                OsString::from("世界"),
            ]),
            Ok(Command::Resume {
                id: machine_god_core::SessionId::new("alpha").unwrap(),
                prompt: "hello 世界".to_owned(),
            })
        );
        assert_eq!(
            parse_arguments([
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("--"),
                OsString::from("--flag"),
            ]),
            Ok(Command::Resume {
                id: machine_god_core::SessionId::new("alpha").unwrap(),
                prompt: "--flag".to_owned(),
            })
        );

        let oversized_prompt = "x".repeat(MAX_ASK_PROMPT_BYTES + 1);
        for arguments in [
            vec![OsString::from("resume")],
            vec![
                OsString::from("resume"),
                OsString::from("last"),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("--id"),
                OsString::from("alpha"),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("--json"),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("--flag"),
                OsString::from("prompt"),
            ],
            vec![OsString::from("resume"), OsString::from("alpha")],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("--"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("--flag"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from(" \t\r\n"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("nul\0prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("bad/session"),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("café"),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("a".repeat(129)),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from(oversized_prompt),
            ],
        ] {
            assert_eq!(parse_arguments(arguments), Err(()));
        }
    }

    #[test]
    fn help_lists_doctor_before_models_with_the_frozen_summary() {
        let output = help();
        assert!(output.contains("  machine-god ask [--] <prompt...>\n"));
        assert!(output.contains("  ask          Run one noninteractive prompt\n"));
        let doctor_usage = output
            .find("  machine-god doctor [--json]\n")
            .expect("doctor usage");
        let models_usage = output
            .find("  machine-god models [--json]\n")
            .expect("models usage");
        assert!(doctor_usage < models_usage);

        let doctor_command = output
            .find("  doctor       Run local health and preflight checks\n")
            .expect("doctor command");
        let models_command = output
            .find("  models       List available models\n")
            .expect("models command");
        assert!(doctor_command < models_command);

        let permissions_usage = output
            .find("  machine-god permissions [--json]\n")
            .expect("permissions usage");
        let resume_usage = output
            .find("  machine-god resume <id> [--] <prompt...>\n")
            .expect("resume usage");
        let inspection_usage = output
            .find("  machine-god session <id> [--json]\n")
            .expect("session usage");
        let listing_usage = output
            .find("  machine-god sessions [--json]\n")
            .expect("sessions usage");
        let status_usage = output
            .find("  machine-god status [--json]\n")
            .expect("status usage");
        assert!(permissions_usage < resume_usage);
        assert!(resume_usage < inspection_usage);
        assert!(inspection_usage < listing_usage);
        assert!(listing_usage < status_usage);

        let permissions_command = output
            .find("  permissions  Show the permission mode and rules\n")
            .expect("permissions command");
        let resume_command = output
            .find("  resume       Resume a saved session with one prompt\n")
            .expect("resume command");
        let inspection_command = output
            .find("  session      Inspect a saved session\n")
            .expect("session command");
        let listing_command = output
            .find("  sessions     List saved sessions\n")
            .expect("sessions command");
        let status_command = output
            .find("  status       Show configuration and runtime information\n")
            .expect("status command");
        assert!(permissions_command < resume_command);
        assert!(resume_command < inspection_command);
        assert!(inspection_command < listing_command);
        assert!(listing_command < status_command);
    }

    #[test]
    fn ask_dispatches_one_valid_prompt_and_preserves_host_output() {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, "one\0β".as_bytes());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_ask_host(
            [
                OsString::from("ask"),
                OsString::from("hello"),
                OsString::from("world"),
            ],
            &mut stdout,
            &mut stderr,
            &host,
        );
        assert_eq!(exit, 0);
        assert_eq!(host.calls.get(), 1);
        assert_eq!(*host.selections.borrow(), [None]);
        assert_eq!(*host.prompts.borrow(), ["hello world"]);
        assert_eq!(stdout, "one\0β".as_bytes());
        assert!(stderr.is_empty());
    }

    #[test]
    fn invalid_ask_arguments_precede_all_host_effects() {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"never");
        for arguments in [
            vec![OsString::from("ask")],
            vec![OsString::from("ask"), OsString::from("--")],
            vec![OsString::from("ask"), OsString::from("--json")],
            vec![OsString::from("ask"), OsString::from(" \t\r\n")],
            vec![OsString::from("ask"), OsString::from("nul\0prompt")],
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_with_ask_host(arguments, &mut stdout, &mut stderr, &host),
                2
            );
            assert!(stdout.is_empty());
            assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
        }
        assert_eq!(host.calls.get(), 0);
        assert!(host.selections.borrow().is_empty());
        assert!(host.prompts.borrow().is_empty());
    }

    #[test]
    fn resume_dispatches_the_validated_id_and_prompt_once() {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"continued");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_ask_host(
            [
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("continue"),
                OsString::from("now"),
            ],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 0);
        assert_eq!(host.calls.get(), 1);
        assert_eq!(*host.selections.borrow(), [Some("alpha".to_owned())]);
        assert_eq!(*host.prompts.borrow(), ["continue now"]);
        assert_eq!(stdout, b"continued");
        assert!(stderr.is_empty());
    }

    #[test]
    fn invalid_resume_arguments_precede_all_host_effects() {
        let host = FakeAskHost::new(AskCommandOutcome::Completed, b"never");
        let oversized_prompt = "x".repeat(MAX_ASK_PROMPT_BYTES + 1);
        for arguments in [
            vec![OsString::from("resume")],
            vec![
                OsString::from("resume"),
                OsString::from("last"),
                OsString::from("prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("--flag"),
                OsString::from("prompt"),
            ],
            vec![OsString::from("resume"), OsString::from("alpha")],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("--flag"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from("nul\0prompt"),
            ],
            vec![
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from(oversized_prompt),
            ],
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_with_ask_host(arguments, &mut stdout, &mut stderr, &host),
                2
            );
            assert!(stdout.is_empty());
            assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
        }
        assert_eq!(host.calls.get(), 0);
        assert!(host.selections.borrow().is_empty());
        assert!(host.prompts.borrow().is_empty());
    }

    #[test]
    fn doctor_human_output_is_exact_and_fail_findings_exit_zero() {
        let report = doctor_report([
            check(
                "configuration",
                DoctorCheckStatus::Ok,
                "configuration loaded",
            ),
            check(
                "credentials",
                DoctorCheckStatus::Warn,
                "credential is missing",
            ),
            check(
                "state",
                DoctorCheckStatus::Fail,
                "state directory is unavailable",
            ),
            check(
                "workspace",
                DoctorCheckStatus::Ok,
                "working directory is available",
            ),
        ]);
        let host = FakeDoctorHost::new(Ok(report));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit =
            run_with_doctor_host([OsString::from("doctor")], &mut stdout, &mut stderr, &host);

        assert_eq!(exit, 0);
        assert_eq!(
            stdout,
            concat!(
                "[doctor] ok=2 warn=1 fail=1\n",
                "[ok] configuration: configuration loaded\n",
                "[warn] credentials: credential is missing\n",
                "[fail] state: state directory is unavailable\n",
                "[ok] workspace: working directory is available\n",
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn doctor_json_output_has_exact_shape_order_escaping_and_lf() {
        let report = doctor_report([
            check(
                "config\"uration",
                DoctorCheckStatus::Ok,
                "loaded\\ready\nnext",
            ),
            check(
                "cred\u{1b}",
                DoctorCheckStatus::Warn,
                "missing\u{2028}credential",
            ),
            check("state", DoctorCheckStatus::Fail, "not available"),
            check("workspace", DoctorCheckStatus::Ok, "café"),
        ]);
        let host = FakeDoctorHost::new(Ok(report));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = run_with_doctor_host(
            [OsString::from("doctor"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 0);
        assert_eq!(
            stdout,
            concat!(
                "{\"kind\":\"doctor\",\"ok_count\":2,\"warn_count\":1,\"fail_count\":1,\"checks\":[",
                "{\"name\":\"config\\\"uration\",\"status\":\"ok\",\"detail\":\"loaded\\\\ready\\nnext\"},",
                "{\"name\":\"cred\\u001b\",\"status\":\"warn\",\"detail\":\"missing\\u2028credential\"},",
                "{\"name\":\"state\",\"status\":\"fail\",\"detail\":\"not available\"},",
                "{\"name\":\"workspace\",\"status\":\"ok\",\"detail\":\"café\"}]}",
                "\n",
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn invalid_doctor_arguments_are_rejected_before_host_effects() {
        for arguments in [
            vec![OsString::from("doctor"), OsString::from("extra")],
            vec![
                OsString::from("doctor"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
        ] {
            let host = FakeDoctorHost::new(Err(()));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_doctor_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 2);
            assert!(stdout.is_empty());
            assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
            assert_eq!(host.calls.get(), 0);
        }
    }

    #[test]
    fn doctor_output_cap_and_invalid_report_fail_before_stdout() {
        let oversized = doctor_report([
            check(
                "configuration",
                DoctorCheckStatus::Ok,
                leaked("x".repeat(MAX_DOCTOR_OUTPUT_BYTES)),
            ),
            check("credentials", DoctorCheckStatus::Ok, "available"),
            check("state", DoctorCheckStatus::Ok, "available"),
            check("workspace", DoctorCheckStatus::Ok, "available"),
        ]);
        let mut invalid = oversized;
        invalid.fail_count = 1;

        for report in [oversized, invalid] {
            for json in [false, true] {
                let host = FakeDoctorHost::new(Ok(report));
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let arguments = if json {
                    vec![OsString::from("doctor"), OsString::from("--json")]
                } else {
                    vec![OsString::from("doctor")]
                };

                let exit = run_with_doctor_host(arguments, &mut stdout, &mut stderr, &host);

                assert_eq!(exit, 1);
                assert!(stdout.is_empty());
                assert_eq!(stderr, DOCTOR_RENDER_FAILURE.as_bytes());
                assert_eq!(host.calls.get(), 1);
            }
        }
    }

    #[test]
    fn doctor_render_cap_is_inclusive() {
        let baseline = doctor_report([
            check("configuration", DoctorCheckStatus::Ok, ""),
            check("credentials", DoctorCheckStatus::Ok, "available"),
            check("state", DoctorCheckStatus::Ok, "available"),
            check("workspace", DoctorCheckStatus::Ok, "available"),
        ]);
        let baseline_len = render_doctor(&baseline, false)
            .expect("baseline report renders")
            .len();
        let report = doctor_report([
            check(
                "configuration",
                DoctorCheckStatus::Ok,
                leaked("x".repeat(MAX_DOCTOR_OUTPUT_BYTES - baseline_len)),
            ),
            check("credentials", DoctorCheckStatus::Ok, "available"),
            check("state", DoctorCheckStatus::Ok, "available"),
            check("workspace", DoctorCheckStatus::Ok, "available"),
        ]);

        let output = render_doctor(&report, false).expect("inclusive limit is accepted");

        assert_eq!(output.len(), MAX_DOCTOR_OUTPUT_BYTES);
    }

    #[test]
    fn doctor_broken_stdout_uses_fixed_output_diagnostic() {
        let report = doctor_report([
            check("configuration", DoctorCheckStatus::Ok, "loaded"),
            check("credentials", DoctorCheckStatus::Warn, "missing"),
            check("state", DoctorCheckStatus::Ok, "available"),
            check("workspace", DoctorCheckStatus::Ok, "available"),
        ]);
        for json in [false, true] {
            let host = FakeDoctorHost::new(Ok(report));
            let mut stdout = BrokenWriter;
            let mut stderr = Vec::new();
            let arguments = if json {
                vec![OsString::from("doctor"), OsString::from("--json")]
            } else {
                vec![OsString::from("doctor")]
            };

            let exit = run_with_doctor_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn doctor_zero_progress_stdout_uses_fixed_output_diagnostic() {
        let report = doctor_report([
            check("configuration", DoctorCheckStatus::Ok, "loaded"),
            check("credentials", DoctorCheckStatus::Warn, "missing"),
            check("state", DoctorCheckStatus::Ok, "available"),
            check("workspace", DoctorCheckStatus::Ok, "available"),
        ]);
        for json in [false, true] {
            let host = FakeDoctorHost::new(Ok(report));
            let mut stdout = ZeroProgressWriter::default();
            let mut stderr = Vec::new();
            let arguments = if json {
                vec![OsString::from("doctor"), OsString::from("--json")]
            } else {
                vec![OsString::from("doctor")]
            };

            let exit = run_with_doctor_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
            assert!(stdout.captured.is_empty());
            assert_eq!(stdout.calls, 1);
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn doctor_partial_stdout_failure_uses_fixed_output_diagnostic() {
        let report = doctor_report([
            check("configuration", DoctorCheckStatus::Ok, "loaded"),
            check("credentials", DoctorCheckStatus::Warn, "missing"),
            check("state", DoctorCheckStatus::Ok, "available"),
            check("workspace", DoctorCheckStatus::Ok, "available"),
        ]);
        for json in [false, true] {
            let complete = render_doctor(&report, json).expect("valid report renders");
            let host = FakeDoctorHost::new(Ok(report));
            let mut stdout = PartialThenBrokenWriter::new(7);
            let mut stderr = Vec::new();
            let arguments = if json {
                vec![OsString::from("doctor"), OsString::from("--json")]
            } else {
                vec![OsString::from("doctor")]
            };

            let exit = run_with_doctor_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
            assert!(!stdout.prefix.is_empty());
            assert!(stdout.prefix.len() < complete.len());
            assert_eq!(stdout.prefix, complete.as_bytes()[..stdout.prefix.len()]);
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn models_human_output_is_exact_for_all_access_modes_and_empty_catalogs() {
        let cases = [
            (
                catalog(
                    &["anthropic/claude-opus", "openai/gpt-5"],
                    ModelCatalogAccess::Authenticated,
                ),
                concat!(
                    "[models] 2 available\n",
                    " - anthropic/claude-opus\n",
                    " - openai/gpt-5\n",
                ),
            ),
            (
                catalog(
                    &["public/model"],
                    ModelCatalogAccess::PublicOnly {
                        reason: PublicCatalogReason::NoCredential,
                    },
                ),
                concat!(
                    "[models] 1 available\n",
                    " - public/model\n",
                    "[models] Using the public model catalog; set VERCEL_OIDC_TOKEN or ",
                    "AI_GATEWAY_API_KEY to include private models.\n",
                ),
            ),
            (
                catalog(
                    &[],
                    ModelCatalogAccess::PublicOnly {
                        reason: PublicCatalogReason::AuthenticatedCredentialRejected,
                    },
                ),
                concat!(
                    "[models] no models returned by gateway\n",
                    "[models] Gateway authentication was rejected; showing the public model catalog.\n",
                ),
            ),
        ];

        for (catalog, expected) in cases {
            let host = FakeModelsHost::new(Ok(catalog));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit =
                run_with_models_host([OsString::from("models")], &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 0);
            assert_eq!(stdout, expected.as_bytes());
            assert!(stderr.is_empty());
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn models_json_output_has_exact_shape_order_escaping_and_lf() {
        let host = FakeModelsHost::new(Ok(catalog(
            &["provider/model", "quoted\"model\\id"],
            ModelCatalogAccess::PublicOnly {
                reason: PublicCatalogReason::NoCredential,
            },
        )));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit = run_with_models_host(
            [OsString::from("models"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 0);
        assert_eq!(
            stdout,
            concat!(
                "{\"kind\":\"models\",\"count\":2,\"shown_count\":2,",
                "\"more_count\":0,\"private_models_hidden\":true,",
                "\"ids\":[\"provider/model\",\"quoted\\\"model\\\\id\"]}\n",
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);

        let host = FakeModelsHost::new(Ok(catalog(&[], ModelCatalogAccess::Authenticated)));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_models_host(
            [OsString::from("models"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            &host,
        );
        assert_eq!(exit, 0);
        assert_eq!(
            stdout,
            b"{\"kind\":\"models\",\"count\":0,\"shown_count\":0,\"more_count\":0,\"private_models_hidden\":false,\"ids\":[]}\n"
        );
        assert!(stderr.is_empty());
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn models_failures_use_exact_human_and_json_channels() {
        let cases = [
            (
                ModelsOperationalFailure::AuthenticationRejected,
                "AuthenticationRejected",
                "AuthenticationRejected",
            ),
            (
                ModelsOperationalFailure::Cancelled,
                "the request was cancelled",
                "Cancelled",
            ),
            (
                ModelsOperationalFailure::MalformedResponse,
                "MalformedResponse",
                "MalformedResponse",
            ),
            (
                ModelsOperationalFailure::ResourceLimit,
                "ResourceLimit",
                "ResourceLimit",
            ),
            (
                ModelsOperationalFailure::Unavailable,
                "Unavailable",
                "Unavailable",
            ),
        ];

        for (failure, detail, code) in cases {
            let host = FakeModelsHost::new(Err(failure));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit =
                run_with_models_host([OsString::from("models")], &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert!(stdout.is_empty());
            assert_eq!(
                stderr,
                format!("machine-god models: could not list models: {detail}\n").as_bytes()
            );

            let host = FakeModelsHost::new(Err(failure));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_models_host(
                [OsString::from("models"), OsString::from("--json")],
                &mut stdout,
                &mut stderr,
                &host,
            );
            assert_eq!(exit, 1);
            assert_eq!(
                stdout,
                format!(
                    "{{\"kind\":\"models\",\"error\":\"could not list models: {detail}\",\"code\":\"{code}\"}}\n"
                )
                .as_bytes()
            );
            assert!(stderr.is_empty());
        }
    }

    #[test]
    fn invalid_models_arguments_do_not_call_the_host() {
        let host = FakeModelsHost::new(Err(ModelsOperationalFailure::Unavailable));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_models_host(
            [OsString::from("models"), OsString::from("extra")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 2);
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
        assert_eq!(host.calls.get(), 0);
    }

    #[test]
    fn models_output_cap_fails_before_any_success_bytes_are_written() {
        let models = (0..600)
            .map(|index| {
                AvailableModel::new(format!("{index:03}{}", "a".repeat(125)))
                    .expect("128-byte visible ASCII model ID")
            })
            .collect();
        let host = FakeModelsHost::new(Ok(ModelCatalog::new(
            models,
            ModelCatalogAccess::Authenticated,
        )));
        for (arguments, expected_stdout, expected_stderr) in [
            (
                vec![OsString::from("models")],
                &b""[..],
                &b"machine-god models: could not list models: ResourceLimit\n"[..],
            ),
            (
                vec![OsString::from("models"), OsString::from("--json")],
                &b"{\"kind\":\"models\",\"error\":\"could not list models: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"[..],
                &b""[..],
            ),
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_models_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 1);
            assert_eq!(stdout, expected_stdout);
            assert_eq!(stderr, expected_stderr);
        }
    }

    #[test]
    fn models_output_cap_is_inclusive() {
        let mut models = Vec::with_capacity(512);
        for _ in 0..511 {
            models.push(AvailableModel::new("a".repeat(124)).expect("valid model ID"));
        }
        models.push(AvailableModel::new("b".repeat(101)).expect("valid model ID"));
        let catalog = ModelCatalog::new(models, ModelCatalogAccess::Authenticated);

        let output = super::render_models(&catalog, false).expect("inclusive limit is accepted");

        assert_eq!(output.len(), super::MAX_MODELS_OUTPUT_BYTES);
    }

    #[test]
    fn models_broken_stdout_uses_the_fixed_output_diagnostic() {
        let host = FakeModelsHost::new(Ok(catalog(
            &["provider/model"],
            ModelCatalogAccess::Authenticated,
        )));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let exit =
            run_with_models_host([OsString::from("models")], &mut stdout, &mut stderr, &host);

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

        let host = FakeModelsHost::new(Err(ModelsOperationalFailure::Unavailable));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let exit = run_with_models_host(
            [OsString::from("models"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn provider_failures_are_mapped_without_reflecting_provider_diagnostics() {
        let cases = [
            (
                ProviderError::new(
                    ProviderErrorKind::Authentication,
                    "AuthenticationRejected",
                    "secret-message",
                    false,
                ),
                ModelsOperationalFailure::AuthenticationRejected,
            ),
            (
                ProviderError::new(
                    ProviderErrorKind::Cancelled,
                    "Cancelled",
                    "secret-message",
                    false,
                ),
                ModelsOperationalFailure::Cancelled,
            ),
            (
                ProviderError::new(
                    ProviderErrorKind::Protocol,
                    "MalformedResponse",
                    "secret-message",
                    false,
                ),
                ModelsOperationalFailure::MalformedResponse,
            ),
            (
                ProviderError::new(
                    ProviderErrorKind::Other,
                    "ResourceLimit",
                    "secret-message",
                    false,
                ),
                ModelsOperationalFailure::ResourceLimit,
            ),
            (
                ProviderError::new(
                    ProviderErrorKind::Authentication,
                    "secret-code",
                    "secret-message",
                    false,
                ),
                ModelsOperationalFailure::Unavailable,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(classify_provider_error(&error), expected);
        }
        for code in [
            "RateLimited",
            "GatewayUnavailable",
            "Unavailable",
            "TransportFailure",
            "RuntimeRequired",
            "future-code",
        ] {
            let error = ProviderError::new(
                ProviderErrorKind::Authentication,
                code,
                "secret-message",
                false,
            );
            assert_eq!(
                classify_provider_error(&error),
                ModelsOperationalFailure::Unavailable
            );
        }
    }

    #[test]
    fn session_human_output_is_exact_and_requests_the_validated_id_once() {
        let host = FakeSessionHost::ready(Ok(session_snapshot("alpha")));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_session_host(
            [OsString::from("session"), OsString::from("alpha")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 0);
        assert_eq!(
            stdout,
            concat!(
                "[session] alpha\n",
                " - incarnation_id: incarnation-alpha\n",
                " - revision: 7\n",
                " - next_turn_sequence: 4\n",
                " - message_count: 3\n",
                " - metadata_entry_count: 2\n",
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
        assert_eq!(&*host.requested_ids.borrow(), &["alpha"]);
    }

    #[test]
    fn session_json_output_has_exact_shape_key_order_and_lf() {
        let host = FakeSessionHost::ready(Ok(session_snapshot("alpha")));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_session_host(
            [
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
            ],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 0);
        assert_eq!(
            stdout,
            b"{\"kind\":\"session\",\"id\":\"alpha\",\"incarnation_id\":\"incarnation-alpha\",\"revision\":7,\"next_turn_sequence\":4,\"message_count\":3,\"metadata_entry_count\":2}\n"
        );
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
        assert_eq!(&*host.requested_ids.borrow(), &["alpha"]);
    }

    #[test]
    fn native_session_inspection_errors_collapse_to_the_frozen_cli_categories() {
        for (kind, expected) in [
            (
                NativeSessionInspectionErrorKind::UnsupportedPlatform,
                SessionOperationalFailure::Unsupported,
            ),
            (
                NativeSessionInspectionErrorKind::InvalidEnvironment,
                SessionOperationalFailure::Unavailable,
            ),
            (
                NativeSessionInspectionErrorKind::UnsafeStateRoot,
                SessionOperationalFailure::Unavailable,
            ),
            (
                NativeSessionInspectionErrorKind::NotFound,
                SessionOperationalFailure::NotFound,
            ),
            (
                NativeSessionInspectionErrorKind::Corrupt,
                SessionOperationalFailure::Corrupt,
            ),
            (
                NativeSessionInspectionErrorKind::Unavailable,
                SessionOperationalFailure::Unavailable,
            ),
        ] {
            assert_eq!(classify_session_inspection_error_kind(kind), expected);
        }
    }

    #[test]
    fn session_failures_use_exact_human_and_json_channels() {
        for failure in [
            SessionOperationalFailure::NotFound,
            SessionOperationalFailure::Corrupt,
            SessionOperationalFailure::Unavailable,
            SessionOperationalFailure::Unsupported,
            SessionOperationalFailure::ResourceLimit,
        ] {
            let category = failure.category();
            let host = FakeSessionHost::ready(Err(failure));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_session_host(
                [OsString::from("session"), OsString::from("alpha")],
                &mut stdout,
                &mut stderr,
                &host,
            );
            assert_eq!(exit, 1);
            assert!(stdout.is_empty());
            assert_eq!(
                stderr,
                format!("machine-god session: could not inspect session: {category}\n").as_bytes()
            );
            assert_eq!(host.calls.get(), 1);

            let host = FakeSessionHost::ready(Err(failure));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_session_host(
                [
                    OsString::from("session"),
                    OsString::from("alpha"),
                    OsString::from("--json"),
                ],
                &mut stdout,
                &mut stderr,
                &host,
            );
            assert_eq!(exit, 1);
            assert_eq!(
                stdout,
                format!(
                    "{{\"kind\":\"session\",\"error\":\"could not inspect session: {category}\",\"code\":\"{category}\"}}\n"
                )
                .as_bytes()
            );
            assert!(stderr.is_empty());
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn pending_session_inspection_is_polled_once_and_maps_to_unavailable() {
        let host = FakeSessionHost::pending();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_session_host(
            [OsString::from("session"), OsString::from("alpha")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            b"machine-god session: could not inspect session: Unavailable\n"
        );
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn invalid_session_arguments_are_rejected_before_host_effects() {
        for arguments in [
            vec![OsString::from("session")],
            vec![OsString::from("session"), OsString::from("last")],
            vec![OsString::from("session"), OsString::from("--id")],
            vec![
                OsString::from("session"),
                OsString::from("--id"),
                OsString::from("alpha"),
            ],
            vec![OsString::from("session"), OsString::from("--json")],
            vec![
                OsString::from("session"),
                OsString::from("--json"),
                OsString::from("alpha"),
            ],
            vec![
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json=true"),
            ],
            vec![
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
            vec![OsString::from("session"), OsString::from("bad/session")],
        ] {
            let host = FakeSessionHost::ready(Err(SessionOperationalFailure::Corrupt));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_session_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 2);
            assert!(stdout.is_empty());
            assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
            assert_eq!(host.calls.get(), 0);
            assert!(host.requested_ids.borrow().is_empty());
        }
    }

    #[test]
    fn session_snapshot_invariants_fail_closed_as_resource_limit() {
        let mut invalid_id = session_snapshot("alpha");
        invalid_id.id = "bad/session".to_owned();
        let mut invalid_incarnation = session_snapshot("alpha");
        invalid_incarnation.incarnation_id = "bad incarnation".to_owned();
        let mut zero_revision = session_snapshot("alpha");
        zero_revision.revision = 0;
        let mut zero_allocator = session_snapshot("alpha");
        zero_allocator.next_turn_sequence = 0;
        let wrong_id = session_snapshot("beta");

        for snapshot in [
            invalid_id,
            invalid_incarnation,
            zero_revision,
            zero_allocator,
            wrong_id,
        ] {
            for json in [false, true] {
                let host = FakeSessionHost::ready(Ok(snapshot.clone()));
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let arguments = if json {
                    vec![
                        OsString::from("session"),
                        OsString::from("alpha"),
                        OsString::from("--json"),
                    ]
                } else {
                    vec![OsString::from("session"), OsString::from("alpha")]
                };
                let exit = run_with_session_host(arguments, &mut stdout, &mut stderr, &host);

                assert_eq!(exit, 1);
                if json {
                    assert_eq!(
                        stdout,
                        b"{\"kind\":\"session\",\"error\":\"could not inspect session: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"
                    );
                    assert!(stderr.is_empty());
                } else {
                    assert!(stdout.is_empty());
                    assert_eq!(
                        stderr,
                        b"machine-god session: could not inspect session: ResourceLimit\n"
                    );
                }
                assert_eq!(host.calls.get(), 1);
            }
        }
    }

    #[test]
    fn maximal_session_snapshot_stays_within_the_output_cap() {
        let snapshot = SessionSnapshot {
            id: "a".repeat(128),
            incarnation_id: "b".repeat(128),
            revision: u64::MAX,
            next_turn_sequence: u64::MAX,
            message_count: usize::MAX,
            metadata_entry_count: usize::MAX,
        };

        for json in [false, true] {
            let output = render_session(&snapshot, json).expect("valid maximum snapshot renders");
            assert!(output.len() <= MAX_SESSION_OUTPUT_BYTES);
            assert!(output.ends_with('\n'));
        }
    }

    #[test]
    fn session_output_cap_is_inclusive() {
        let mut output = BoundedSessionOutput::new();
        output
            .write_str(&"x".repeat(MAX_SESSION_OUTPUT_BYTES))
            .expect("inclusive output limit is accepted");
        assert_eq!(output.value.len(), MAX_SESSION_OUTPUT_BYTES);
        assert!(output.write_char('x').is_err());
    }

    #[test]
    fn session_broken_zero_progress_and_partial_stdout_use_output_diagnostic() {
        for json in [false, true] {
            let arguments = if json {
                vec![
                    OsString::from("session"),
                    OsString::from("alpha"),
                    OsString::from("--json"),
                ]
            } else {
                vec![OsString::from("session"), OsString::from("alpha")]
            };
            let snapshot = session_snapshot("alpha");

            let host = FakeSessionHost::ready(Ok(snapshot.clone()));
            let mut stdout = BrokenWriter;
            let mut stderr = Vec::new();
            let exit = run_with_session_host(arguments.clone(), &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

            let host = FakeSessionHost::ready(Ok(snapshot.clone()));
            let mut stdout = ZeroProgressWriter::default();
            let mut stderr = Vec::new();
            let exit = run_with_session_host(arguments.clone(), &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert!(stdout.captured.is_empty());
            assert_eq!(stdout.calls, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

            let complete = render_session(&snapshot, json).expect("snapshot renders");
            let host = FakeSessionHost::ready(Ok(snapshot));
            let mut stdout = PartialThenBrokenWriter::new(5);
            let mut stderr = Vec::new();
            let exit = run_with_session_host(arguments, &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert!(!stdout.prefix.is_empty());
            assert_eq!(stdout.prefix, complete.as_bytes()[..stdout.prefix.len()]);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        }
    }

    #[test]
    fn session_json_failure_write_errors_use_output_diagnostic() {
        let host = FakeSessionHost::ready(Err(SessionOperationalFailure::Corrupt));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let exit = run_with_session_host(
            [
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from("--json"),
            ],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn session_human_failure_write_errors_use_output_diagnostic() {
        let host = FakeSessionHost::ready(Err(SessionOperationalFailure::Corrupt));
        let mut stdout = Vec::new();
        let mut stderr = FirstWriteFailsThenCaptures::default();
        let exit = run_with_session_host(
            [OsString::from("session"), OsString::from("alpha")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(stderr.captured, OUTPUT_FAILURE.as_bytes());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn sessions_human_output_is_exact_for_empty_populated_and_truncated_results() {
        let cases = [
            (
                sessions_snapshot(&[], false),
                "[sessions] no saved sessions\n",
            ),
            (
                sessions_snapshot(&["alpha", "beta"], false),
                "[sessions] 2 saved\n - alpha\n - beta\n",
            ),
            (
                sessions_snapshot(&[], true),
                concat!(
                    "[sessions] 0 saved\n",
                    "[sessions] listing incomplete: a resource limit was reached\n",
                ),
            ),
            (
                sessions_snapshot(&["alpha"], true),
                concat!(
                    "[sessions] 1 saved\n",
                    " - alpha\n",
                    "[sessions] listing incomplete: a resource limit was reached\n",
                ),
            ),
        ];

        for (snapshot, expected) in cases {
            let host = FakeSessionsHost::ready(Ok(snapshot));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(
                [OsString::from("sessions")],
                &mut stdout,
                &mut stderr,
                &host,
            );

            assert_eq!(exit, 0);
            assert_eq!(stdout, expected.as_bytes());
            assert!(stderr.is_empty());
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn sessions_json_output_has_exact_shape_key_order_and_lf() {
        for (snapshot, expected) in [
            (
                sessions_snapshot(&[], false),
                "{\"kind\":\"sessions\",\"count\":0,\"truncated\":false,\"sessions\":[]}\n",
            ),
            (
                sessions_snapshot(&["alpha", "quoted.id"], true),
                concat!(
                    "{\"kind\":\"sessions\",\"count\":2,\"truncated\":true,",
                    "\"sessions\":[{\"id\":\"alpha\"},{\"id\":\"quoted.id\"}]}\n",
                ),
            ),
        ] {
            let host = FakeSessionsHost::ready(Ok(snapshot));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(
                [OsString::from("sessions"), OsString::from("--json")],
                &mut stdout,
                &mut stderr,
                &host,
            );

            assert_eq!(exit, 0);
            assert_eq!(stdout, expected.as_bytes());
            assert!(stderr.is_empty());
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn native_session_listing_errors_collapse_to_the_frozen_cli_categories() {
        for (kind, expected) in [
            (
                NativeSessionListingErrorKind::UnsupportedPlatform,
                SessionsOperationalFailure::Unsupported,
            ),
            (
                NativeSessionListingErrorKind::InvalidEnvironment,
                SessionsOperationalFailure::Unavailable,
            ),
            (
                NativeSessionListingErrorKind::UnsafeStateRoot,
                SessionsOperationalFailure::Unavailable,
            ),
            (
                NativeSessionListingErrorKind::Corrupt,
                SessionsOperationalFailure::Corrupt,
            ),
            (
                NativeSessionListingErrorKind::Unavailable,
                SessionsOperationalFailure::Unavailable,
            ),
        ] {
            assert_eq!(classify_session_listing_error_kind(kind), expected);
        }
    }

    #[test]
    fn sessions_failures_use_exact_human_and_json_channels() {
        for failure in [
            SessionsOperationalFailure::Corrupt,
            SessionsOperationalFailure::Unavailable,
            SessionsOperationalFailure::Unsupported,
            SessionsOperationalFailure::ResourceLimit,
        ] {
            let category = failure.category();
            let host = FakeSessionsHost::ready(Err(failure));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(
                [OsString::from("sessions")],
                &mut stdout,
                &mut stderr,
                &host,
            );
            assert_eq!(exit, 1);
            assert!(stdout.is_empty());
            assert_eq!(
                stderr,
                format!("machine-god sessions: could not list sessions: {category}\n").as_bytes()
            );
            assert_eq!(host.calls.get(), 1);

            let host = FakeSessionsHost::ready(Err(failure));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(
                [OsString::from("sessions"), OsString::from("--json")],
                &mut stdout,
                &mut stderr,
                &host,
            );
            assert_eq!(exit, 1);
            assert_eq!(
                stdout,
                format!(
                    "{{\"kind\":\"sessions\",\"error\":\"could not list sessions: {category}\",\"code\":\"{category}\"}}\n"
                )
                .as_bytes()
            );
            assert!(stderr.is_empty());
            assert_eq!(host.calls.get(), 1);
        }
    }

    #[test]
    fn pending_sessions_listing_is_polled_once_and_maps_to_unavailable() {
        let host = FakeSessionsHost::pending();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_sessions_host(
            [OsString::from("sessions")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            b"machine-god sessions: could not list sessions: Unavailable\n"
        );
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn invalid_sessions_arguments_are_rejected_before_host_effects() {
        for arguments in [
            vec![OsString::from("sessions"), OsString::from("extra")],
            vec![OsString::from("sessions"), OsString::from("--json=true")],
            vec![
                OsString::from("sessions"),
                OsString::from("--json"),
                OsString::from("extra"),
            ],
        ] {
            let host = FakeSessionsHost::ready(Err(SessionsOperationalFailure::Corrupt));
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(arguments, &mut stdout, &mut stderr, &host);

            assert_eq!(exit, 2);
            assert!(stdout.is_empty());
            assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
            assert_eq!(host.calls.get(), 0);
        }
    }

    #[test]
    fn sessions_snapshot_invariants_fail_closed_as_resource_limit() {
        let excessive = SessionsSnapshot {
            session_ids: (0..=super::MAX_LIST_SESSIONS)
                .map(|index| format!("session-{index:03}"))
                .collect(),
            truncated: false,
        };
        let cases = [
            excessive,
            sessions_snapshot(&["bad/session"], false),
            sessions_snapshot(&["alpha", "alpha"], false),
            sessions_snapshot(&["beta", "alpha"], false),
        ];

        for snapshot in cases {
            for json in [false, true] {
                let host = FakeSessionsHost::ready(Ok(snapshot.clone()));
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let arguments = if json {
                    vec![OsString::from("sessions"), OsString::from("--json")]
                } else {
                    vec![OsString::from("sessions")]
                };
                let exit = run_with_sessions_host(arguments, &mut stdout, &mut stderr, &host);

                assert_eq!(exit, 1);
                if json {
                    assert_eq!(
                        stdout,
                        b"{\"kind\":\"sessions\",\"error\":\"could not list sessions: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"
                    );
                    assert!(stderr.is_empty());
                } else {
                    assert!(stdout.is_empty());
                    assert_eq!(
                        stderr,
                        b"machine-god sessions: could not list sessions: ResourceLimit\n"
                    );
                }
                assert_eq!(host.calls.get(), 1);
            }
        }
    }

    #[test]
    fn maximal_valid_sessions_snapshot_stays_within_the_output_cap() {
        let session_ids: Vec<String> = (0..super::MAX_LIST_SESSIONS)
            .map(|index| format!("{index:03}{}", "a".repeat(125)))
            .collect();
        let snapshot = SessionsSnapshot {
            session_ids,
            truncated: true,
        };

        for json in [false, true] {
            let output = render_sessions(&snapshot, json).expect("valid maximum snapshot renders");
            assert!(output.len() <= MAX_SESSIONS_OUTPUT_BYTES);
            assert!(output.ends_with('\n'));
        }
    }

    #[test]
    fn sessions_output_cap_is_inclusive() {
        let mut output = BoundedSessionsOutput::new();
        output
            .write_str(&"x".repeat(MAX_SESSIONS_OUTPUT_BYTES))
            .expect("inclusive output limit is accepted");
        assert_eq!(output.value.len(), MAX_SESSIONS_OUTPUT_BYTES);
        assert!(output.write_char('x').is_err());
    }

    #[test]
    fn sessions_broken_zero_progress_and_partial_stdout_use_output_diagnostic() {
        for json in [false, true] {
            let arguments = if json {
                vec![OsString::from("sessions"), OsString::from("--json")]
            } else {
                vec![OsString::from("sessions")]
            };
            let snapshot = sessions_snapshot(&["alpha"], false);

            let host = FakeSessionsHost::ready(Ok(snapshot.clone()));
            let mut stdout = BrokenWriter;
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(arguments.clone(), &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

            let host = FakeSessionsHost::ready(Ok(snapshot.clone()));
            let mut stdout = ZeroProgressWriter::default();
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(arguments.clone(), &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert!(stdout.captured.is_empty());
            assert_eq!(stdout.calls, 1);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

            let complete = render_sessions(&snapshot, json).expect("snapshot renders");
            let host = FakeSessionsHost::ready(Ok(snapshot));
            let mut stdout = PartialThenBrokenWriter::new(5);
            let mut stderr = Vec::new();
            let exit = run_with_sessions_host(arguments, &mut stdout, &mut stderr, &host);
            assert_eq!(exit, 1);
            assert!(!stdout.prefix.is_empty());
            assert_eq!(stdout.prefix, complete.as_bytes()[..stdout.prefix.len()]);
            assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        }
    }

    #[test]
    fn sessions_json_failure_write_errors_use_output_diagnostic() {
        let host = FakeSessionsHost::ready(Err(SessionsOperationalFailure::Corrupt));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let exit = run_with_sessions_host(
            [OsString::from("sessions"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn sessions_human_failure_write_errors_use_output_diagnostic() {
        let host = FakeSessionsHost::ready(Err(SessionsOperationalFailure::Corrupt));
        let mut stdout = Vec::new();
        let mut stderr = FirstWriteFailsThenCaptures::default();
        let exit = run_with_sessions_host(
            [OsString::from("sessions")],
            &mut stdout,
            &mut stderr,
            &host,
        );

        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(stderr.captured, OUTPUT_FAILURE.as_bytes());
        assert_eq!(host.calls.get(), 1);
    }

    #[test]
    fn permissions_outputs_are_exact() {
        assert_eq!(
            permissions(PermissionMode::Ask, false),
            concat!(
                "machine-god 0.1.0 (engine API 1)\n",
                "permission_mode: ask\n",
                "persistent_rules: unsupported\n",
                "runtime_grants: unavailable\n",
            )
        );
        assert_eq!(
            json_permissions(PermissionMode::Ask),
            concat!(
                "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
                "\"engine_api_version\":1,\"kind\":\"permissions\",",
                "\"permission_mode\":\"ask\",",
                "\"persistent_rules_supported\":false,",
                "\"runtime_grants_available\":false}\n",
            )
        );
    }

    #[test]
    fn json_encoder_escapes_terminal_controls_and_json_metacharacters() {
        let mut encoded = String::new();
        push_json_string(
            &mut encoded,
            concat!(
                "quote\" slash\\ controls\n\r\t\u{1b}\u{7f}\u{85} ",
                "bidi\u{061c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069} ",
                "separators\u{2028}\u{2029}",
            ),
        );
        assert_eq!(
            encoded,
            concat!(
                "\"quote\\\" slash\\\\ controls\\n\\r\\t\\u001b\\u007f\\u0085 ",
                "bidi\\u061c\\u200e\\u200f\\u202a\\u202e\\u2066\\u2069 ",
                "separators\\u2028\\u2029\"",
            )
        );
    }

    #[test]
    fn output_failure_is_a_fixed_diagnostic_without_panicking() {
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        let exit = run([], &mut stdout, &mut stderr);

        assert_eq!(exit, 1);
        assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
    }

    #[test]
    fn invalid_arguments_do_not_touch_stdout() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run([OsString::from("nope")], &mut stdout, &mut stderr);

        assert_eq!(exit, 2);
        assert!(stdout.is_empty());
        assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_arguments_are_rejected() {
        use std::os::unix::ffi::OsStringExt;

        assert_eq!(parse_arguments([OsString::from_vec(vec![0xff])]), Err(()));
        assert_eq!(
            parse_arguments([OsString::from("doctor"), OsString::from_vec(vec![0xff]),]),
            Err(())
        );
        assert_eq!(
            parse_arguments([OsString::from("models"), OsString::from_vec(vec![0xff]),]),
            Err(())
        );
        assert_eq!(
            parse_arguments([OsString::from("session"), OsString::from_vec(vec![0xff]),]),
            Err(())
        );
        assert_eq!(
            parse_arguments([OsString::from("resume"), OsString::from_vec(vec![0xff]),]),
            Err(())
        );
        assert_eq!(
            parse_arguments([
                OsString::from("resume"),
                OsString::from("alpha"),
                OsString::from_vec(vec![0xff]),
            ]),
            Err(())
        );
        assert_eq!(
            parse_arguments([
                OsString::from("session"),
                OsString::from("alpha"),
                OsString::from_vec(vec![0xff]),
            ]),
            Err(())
        );
        assert_eq!(
            parse_arguments([OsString::from("sessions"), OsString::from_vec(vec![0xff]),]),
            Err(())
        );
    }
}
