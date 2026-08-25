use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
#[cfg(not(target_family = "wasm"))]
use std::future::poll_fn;
use std::io;
use std::path::Path;
use std::process::ExitCode;
#[cfg(not(target_family = "wasm"))]
use std::task::Poll;

#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

#[cfg(not(target_family = "wasm"))]
use machine_god_core::{
    BoxFuture, CancellationToken, ModelCatalogProvider, ProviderError, ProviderErrorKind,
};
use machine_god_core::{ModelCatalog, ModelCatalogAccess, PublicCatalogReason};
#[cfg(not(target_family = "wasm"))]
use machine_god_native::{
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogHttpTransport,
    AiGatewayModelCatalogProvider, AiGatewayModelCatalogTransport,
    DiscoveredAiGatewayCatalogCredential, discover_process_ai_gateway_catalog_credential,
};
use machine_god_native::{
    NativeCredentialSourceKind, NativeProviderKind, NativeStatus, NativeTransportKind,
    PermissionMode, inspect_process_status, load_process_config,
};

const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | models [--json] | permissions [--json] | status [--json]]\n",
);
const CONFIGURATION_FAILURE: &str = "machine-god: failed to load configuration\n";
const OUTPUT_FAILURE: &str = "machine-god: failed to write output\n";
const MAX_MODELS_OUTPUT_BYTES: usize = 64 * 1024;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Identity,
    Help,
    Models { json: bool },
    Permissions { json: bool },
    Status { json: bool },
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

trait ModelsCommandHost {
    fn list_models(&self) -> Result<ModelCatalog, ModelsOperationalFailure>;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProductionModelsCommandHost;

#[cfg(not(target_family = "wasm"))]
trait ModelsCompositionEffects {
    type Credential;

    fn load_and_validate_config(&self) -> Result<(), ModelsOperationalFailure>;

    fn discover_credential(&self) -> Result<Self::Credential, ModelsOperationalFailure>;

    fn create_transport_and_list(
        &self,
        credential: Self::Credential,
    ) -> Result<ModelCatalog, ModelsOperationalFailure>;
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

    fn create_transport_and_list(
        &self,
        credential: Self::Credential,
    ) -> Result<ModelCatalog, ModelsOperationalFailure> {
        let (access_mode, bearer_token) = match credential {
            DiscoveredAiGatewayCatalogCredential::PublicOnly => {
                (AiGatewayModelCatalogAccessMode::PublicOnly, None)
            }
            DiscoveredAiGatewayCatalogCredential::Authenticated(credential) => (
                AiGatewayModelCatalogAccessMode::Authenticated,
                Some(credential.into_bearer_token()),
            ),
        };
        let transport = AiGatewayModelCatalogHttpTransport::new(bearer_token)
            .map_err(|_| ModelsOperationalFailure::Unavailable)?;
        let transport: Arc<dyn AiGatewayModelCatalogTransport> = Arc::new(transport);
        let provider = AiGatewayModelCatalogProvider::new(access_mode, transport);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| ModelsOperationalFailure::Unavailable)?;
        runtime
            .block_on(list_models_with_signals(&provider))
            .map_err(|error| classify_provider_error(&error))
    }
}

#[cfg(not(target_family = "wasm"))]
fn list_models_with_effects(
    effects: &impl ModelsCompositionEffects,
) -> Result<ModelCatalog, ModelsOperationalFailure> {
    effects.load_and_validate_config()?;
    let credential = effects.discover_credential()?;
    effects.create_transport_and_list(credential)
}

impl ModelsCommandHost for ProductionModelsCommandHost {
    fn list_models(&self) -> Result<ModelCatalog, ModelsOperationalFailure> {
        #[cfg(not(target_family = "wasm"))]
        {
            list_models_with_effects(&ProcessModelsCompositionEffects)
        }

        #[cfg(target_family = "wasm")]
        {
            let loaded =
                load_process_config().map_err(|_| ModelsOperationalFailure::Unavailable)?;
            let config = loaded.config();
            if config.provider() != NativeProviderKind::VercelAiGateway
                || config.transport() != NativeTransportKind::AiGatewayHttp
                || config.credential_source() != NativeCredentialSourceKind::Environment
            {
                return Err(ModelsOperationalFailure::Unavailable);
            }
            Err(ModelsOperationalFailure::Unavailable)
        }
    }
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelsSignalEvent {
    Received,
    WaitFailed,
}

#[cfg(not(target_family = "wasm"))]
trait ModelsSignalSource {
    fn register_interrupt(
        &mut self,
    ) -> Result<BoxFuture<'static, ModelsSignalEvent>, ProviderError>;

    #[cfg(unix)]
    fn register_terminate(
        &mut self,
    ) -> Result<BoxFuture<'static, ModelsSignalEvent>, ProviderError>;
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Default)]
struct TokioModelsSignalSource;

#[cfg(not(target_family = "wasm"))]
impl ModelsSignalSource for TokioModelsSignalSource {
    fn register_interrupt(
        &mut self,
    ) -> Result<BoxFuture<'static, ModelsSignalEvent>, ProviderError> {
        Ok(Box::pin(async {
            match tokio::signal::ctrl_c().await {
                Ok(()) => ModelsSignalEvent::Received,
                Err(_) => ModelsSignalEvent::WaitFailed,
            }
        }))
    }

    #[cfg(unix)]
    fn register_terminate(
        &mut self,
    ) -> Result<BoxFuture<'static, ModelsSignalEvent>, ProviderError> {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|_| signal_unavailable_error())?;
        Ok(Box::pin(async move {
            terminate_signal_event(terminate.recv().await)
        }))
    }
}

#[cfg(all(not(target_family = "wasm"), unix))]
const fn terminate_signal_event(received: Option<()>) -> ModelsSignalEvent {
    match received {
        Some(()) => ModelsSignalEvent::Received,
        None => ModelsSignalEvent::WaitFailed,
    }
}

#[cfg(not(target_family = "wasm"))]
async fn list_models_with_signals(
    provider: &dyn ModelCatalogProvider,
) -> Result<ModelCatalog, ProviderError> {
    list_models_with_signal_source(provider, &mut TokioModelsSignalSource).await
}

#[cfg(not(target_family = "wasm"))]
async fn list_models_with_signal_source(
    provider: &dyn ModelCatalogProvider,
    signals: &mut impl ModelsSignalSource,
) -> Result<ModelCatalog, ProviderError> {
    let cancellation = CancellationToken::new();
    let mut interrupt = signals.register_interrupt()?;
    #[cfg(unix)]
    let mut terminate = signals.register_terminate()?;

    let initial_signal = poll_fn(|context| {
        if let Poll::Ready(event) = interrupt.as_mut().poll(context) {
            return Poll::Ready(Some(event));
        }
        #[cfg(unix)]
        if let Poll::Ready(event) = terminate.as_mut().poll(context) {
            return Poll::Ready(Some(event));
        }
        Poll::Ready(None)
    })
    .await;
    if let Some(event) = initial_signal {
        cancellation.cancel();
        return Err(signal_event_error(event));
    }

    let mut provider_future = provider.list_models(cancellation.clone());
    poll_fn(|context| {
        if let Poll::Ready(event) = interrupt.as_mut().poll(context) {
            cancellation.cancel();
            return Poll::Ready(Err(signal_event_error(event)));
        }
        #[cfg(unix)]
        if let Poll::Ready(event) = terminate.as_mut().poll(context) {
            cancellation.cancel();
            return Poll::Ready(Err(signal_event_error(event)));
        }
        let provider_result = match provider_future.as_mut().poll(context) {
            Poll::Ready(result) => result,
            Poll::Pending => return Poll::Pending,
        };
        if let Poll::Ready(event) = interrupt.as_mut().poll(context) {
            cancellation.cancel();
            return Poll::Ready(Err(signal_event_error(event)));
        }
        #[cfg(unix)]
        if let Poll::Ready(event) = terminate.as_mut().poll(context) {
            cancellation.cancel();
            return Poll::Ready(Err(signal_event_error(event)));
        }
        Poll::Ready(provider_result)
    })
    .await
}

#[cfg(not(target_family = "wasm"))]
fn signal_event_error(event: ModelsSignalEvent) -> ProviderError {
    match event {
        ModelsSignalEvent::Received => ProviderError::new(
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
    run_with_models_host(arguments, stdout, stderr, &ProductionModelsCommandHost)
}

fn run_with_models_host(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    models_host: &impl ModelsCommandHost,
) -> u8 {
    let Ok(command) = parse_arguments(arguments) else {
        let _ = stderr.write_all(INVALID_ARGUMENTS.as_bytes());
        return 2;
    };

    let output = match command {
        Command::Identity => identity(),
        Command::Help => help(),
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
            "  machine-god models [--json]\n",
            "  machine-god permissions [--json]\n",
            "  machine-god status [--json]\n",
            "\n",
            "Commands:\n",
            "  help         Show this help\n",
            "  models       List available models\n",
            "  permissions  Show the permission mode and rules\n",
            "  status       Show configuration and runtime information\n",
            "\n",
            "Options:\n",
            "  -h, --help       Show this help\n",
            "  -V, --version    Show version\n",
        ),
        env!("CARGO_PKG_VERSION")
    )
}

fn run_models(
    host: &impl ModelsCommandHost,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let catalog = match host.list_models() {
        Ok(catalog) => catalog,
        Err(failure) => return write_models_failure(failure, json, stdout, stderr),
    };

    let output = match render_models(&catalog, json) {
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
        Command, INVALID_ARGUMENTS, ModelsCommandHost, ModelsOperationalFailure, OUTPUT_FAILURE,
        PermissionMode, json_permissions, parse_arguments, permissions, push_json_string, run,
        run_with_models_host,
    };
    #[cfg(not(target_family = "wasm"))]
    use super::{ModelsCompositionEffects, classify_provider_error, list_models_with_effects};
    #[cfg(unix)]
    use super::{
        ModelsSignalEvent, ModelsSignalSource, list_models_with_signal_source,
        signal_unavailable_error, terminate_signal_event,
    };
    use machine_god_core::{AvailableModel, ModelCatalog, ModelCatalogAccess, PublicCatalogReason};
    #[cfg(unix)]
    use machine_god_core::{BoxFuture, CancellationToken, ModelCatalogProvider};
    #[cfg(not(target_family = "wasm"))]
    use machine_god_core::{ProviderError, ProviderErrorKind};
    use std::cell::Cell;
    #[cfg(not(target_family = "wasm"))]
    use std::cell::RefCell;
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::future::Future;
    use std::io;
    #[cfg(unix)]
    use std::pin::Pin;
    #[cfg(unix)]
    use std::sync::{Arc, Mutex};
    #[cfg(unix)]
    use std::task::{Context, Poll};

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
        fn list_models(&self) -> Result<ModelCatalog, ModelsOperationalFailure> {
            self.calls.set(self.calls.get() + 1);
            self.result.clone()
        }
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

        fn create_transport_and_list(
            &self,
            (): Self::Credential,
        ) -> Result<ModelCatalog, ModelsOperationalFailure> {
            self.trace
                .borrow_mut()
                .push(CompositionEffect::CreateTransportAndList);
            self.catalog_result.clone()
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
        interrupt: Option<ScriptedSignal>,
        #[cfg(unix)]
        terminate: Option<ScriptedSignal>,
        fail_interrupt_registration: bool,
        #[cfg(unix)]
        fail_terminate_registration: bool,
    }

    #[cfg(unix)]
    impl FakeModelsSignalSource {
        fn new(trace: Arc<Mutex<Vec<SignalTrace>>>, interrupt: ScriptedSignal) -> Self {
            Self {
                trace,
                interrupt: Some(interrupt),
                #[cfg(unix)]
                terminate: Some(ScriptedSignal::Pending),
                fail_interrupt_registration: false,
                #[cfg(unix)]
                fail_terminate_registration: false,
            }
        }

        fn signal_future(
            &self,
            script: ScriptedSignal,
            poll_event: SignalTrace,
            drop_event: SignalTrace,
        ) -> BoxFuture<'static, ModelsSignalEvent> {
            Box::pin(ScriptedSignalFuture {
                trace: Arc::clone(&self.trace),
                poll_event,
                drop_event,
                script,
                polls: 0,
            })
        }
    }

    #[cfg(unix)]
    impl ModelsSignalSource for FakeModelsSignalSource {
        fn register_interrupt(
            &mut self,
        ) -> Result<BoxFuture<'static, ModelsSignalEvent>, ProviderError> {
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::RegisterInterrupt);
            if self.fail_interrupt_registration {
                return Err(signal_unavailable_error());
            }
            let script = self.interrupt.take().expect("interrupt registered once");
            Ok(self.signal_future(
                script,
                SignalTrace::PollInterrupt,
                SignalTrace::DropInterrupt,
            ))
        }

        #[cfg(unix)]
        fn register_terminate(
            &mut self,
        ) -> Result<BoxFuture<'static, ModelsSignalEvent>, ProviderError> {
            self.trace
                .lock()
                .unwrap()
                .push(SignalTrace::RegisterTerminate);
            if self.fail_terminate_registration {
                return Err(signal_unavailable_error());
            }
            let script = self.terminate.take().expect("terminate registered once");
            Ok(self.signal_future(
                script,
                SignalTrace::PollTerminate,
                SignalTrace::DropTerminate,
            ))
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
        signals: &mut FakeModelsSignalSource,
    ) -> Result<ModelCatalog, ProviderError> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(list_models_with_signal_source(provider, signals))
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
        assert!(list_models_with_effects(&success).is_ok());
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
            list_models_with_effects(&config_failure),
            Err(ModelsOperationalFailure::Unavailable)
        );
        assert_eq!(
            *config_failure.trace.borrow(),
            [CompositionEffect::LoadConfig]
        );

        let credential_failure =
            composition_effects(Ok(()), Err(ModelsOperationalFailure::Unavailable));
        assert_eq!(
            list_models_with_effects(&credential_failure),
            Err(ModelsOperationalFailure::Unavailable)
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
        let mut signals = FakeModelsSignalSource::new(Arc::clone(&trace), ScriptedSignal::Pending);

        assert!(run_signal_coordination(&provider, &mut signals).is_ok());
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
        let mut signals = FakeModelsSignalSource::new(
            Arc::clone(&trace),
            ScriptedSignal::ReadyOnPoll {
                poll: 3,
                event: ModelsSignalEvent::Received,
            },
        );

        let error = run_signal_coordination(&provider, &mut signals).unwrap_err();

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
            run_signal_coordination(&registration_provider, &mut registration_signals).unwrap_err();
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
        wait_signals.terminate = Some(ScriptedSignal::ReadyOnPoll {
            poll: 3,
            event: ModelsSignalEvent::WaitFailed,
        });

        let wait_error = run_signal_coordination(&wait_provider, &mut wait_signals).unwrap_err();
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
            ModelsSignalEvent::Received
        );
        assert_eq!(terminate_signal_event(None), ModelsSignalEvent::WaitFailed);
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
            parse_arguments([OsString::from("models"), OsString::from_vec(vec![0xff]),]),
            Err(())
        );
    }
}
