use super::{
    NativeMcpStartup, NativeMcpStartupBatch, NativeMcpStartupError as Error,
    NativeMcpStartupPhase as Phase, NativeMcpStartupReceipt, NativeMcpStartupServerReceipt,
    NativeMcpStartupState as State, Result,
    batch::{AttemptOwner, BuildPermit, NativeMcpStartupCompletion, protocol},
    control,
    phase::Selection,
};
use crate::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    config::{McpConfig, McpServerConfig, McpTransportConfig},
    pagination::{McpCatalogKind, McpCatalogLimits},
    peer::{McpPeerError, McpStdioPeer},
    runtime::{NativeMcpOwnedPeer, NativeMcpPeerCompletion, NativeMcpServerCandidate},
};
use machine_god_core::CancellationToken;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

pub(super) async fn build(
    startup: &NativeMcpStartup,
    phase: Phase,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
) -> NativeMcpStartupBatch {
    let mut receipts: Vec<_> = startup
        .configuration
        .servers()
        .iter()
        .map(|server| {
            let state = match phase.select(server.enabled(), server.required()) {
                Selection::Connect => State::NotAttempted,
                Selection::Disabled => State::Disabled,
                Selection::Deferred => State::Deferred,
            };
            NativeMcpStartupServerReceipt {
                name: server.name().into(),
                required: server.required(),
                state,
                attempts: 0,
                cleanup: NativeMcpStartupCompletion::new(),
            }
        })
        .collect();
    let mut servers = Vec::new();
    let mut failure = None;
    let permit = startup
        .pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| BuildPermit(startup.pending.clone()));
    if permit.is_none() {
        failure = Some(Error::Unavailable);
    }
    let guards = [
        startup.owner.clone(),
        startup.configuration_generation.clone(),
        cancellation.clone(),
    ];
    let deadline = lifetime_deadline(startup, deadline);
    if failure.is_none() {
        failure = control::check_optional(&startup.clock, &guards, deadline).err();
    }
    if failure.is_none() && startup.clock.now() < startup.catalog_epoch {
        failure = Some(Error::Invalid);
    }
    let mut retained = 0usize;
    for (configuration, receipt) in startup.configuration.servers().iter().zip(&mut receipts) {
        if failure.is_some() {
            break;
        }
        if receipt.state != State::NotAttempted {
            continue;
        }
        if let Err(error) = control::check_optional(&startup.clock, &guards, deadline) {
            failure = Some(error);
            break;
        }
        if let Err(error) = startup.record_server(receipt.cleanup.clone()) {
            failure = Some(error);
            break;
        }
        let mut owner = AttemptOwner {
            completion: receipt.cleanup.clone(),
            cancellation: CancellationToken::new(),
            transferred: false,
        };
        let result = server(
            startup,
            Arc::new(configuration.clone()),
            &guards,
            &mut owner,
            &mut receipt.attempts,
            deadline,
            remaining(startup, configuration.name(), retained),
        )
        .await;
        match result {
            Ok((candidate, charge)) => {
                retained += charge;
                receipt.state = State::Ready(protocol(&candidate.peer));
                servers.push(candidate);
                owner.transferred = true;
            }
            Err(error) => receipt.state = State::Failed(error),
        }
        if let Err(error) = control::check_optional(&startup.clock, &guards, deadline) {
            failure = Some(error);
        }
    }
    for receipt in &receipts {
        receipt.cleanup.seal();
    }
    NativeMcpStartupBatch {
        servers,
        receipt: NativeMcpStartupReceipt {
            phase,
            servers: receipts.into(),
            failure,
        },
        _permit: permit,
    }
}

async fn server(
    startup: &NativeMcpStartup,
    configuration: Arc<McpServerConfig>,
    guards: &[CancellationToken],
    owner: &mut AttemptOwner,
    attempts: &mut u16,
    deadline: Option<Instant>,
    maximum: usize,
) -> Result<(NativeMcpServerCandidate, usize)> {
    let mut encoded = McpConfig::new();
    encoded
        .insert((*configuration).clone())
        .map_err(|_| Error::Invalid)?;
    let identity: Arc<[u8]> = encoded.encode().map_err(|_| Error::Limit)?.into();
    let minimum = identity
        .len()
        .checked_add(configuration.name().len() + 4096)
        .ok_or(Error::Limit)?;
    if minimum > maximum {
        return Err(Error::Limit);
    }
    let (peer, catalogs, authentication, generations) = match configuration.transport() {
        McpTransportConfig::Stdio(_) => {
            let (peer, catalogs) = stdio_server(
                startup,
                &configuration,
                guards,
                owner,
                attempts,
                deadline,
                maximum - minimum,
            )
            .await?;
            (peer, catalogs, Arc::<[u8]>::from([]), Vec::new())
        }
        McpTransportConfig::Http(remote) | McpTransportConfig::Sse(remote) => {
            #[cfg(feature = "mcp-http")]
            {
                *attempts = 1;
                remote_server(
                    startup,
                    &configuration,
                    remote,
                    guards,
                    owner,
                    deadline,
                    maximum - minimum,
                )
                .await?
            }
            #[cfg(not(feature = "mcp-http"))]
            {
                let _ = remote;
                return Err(Error::Unavailable);
            }
        }
    };
    // Build-operation cancellation is not a post-publication lifetime token.
    // Host/configuration/network/authentication generations are retained instead.
    let mut authority_cancellations = vec![
        startup.owner.clone(),
        startup.configuration_generation.clone(),
    ];
    authority_cancellations.extend(generations);
    control::check_optional(&startup.clock, guards, deadline)?;
    control::check_optional(&startup.clock, &authority_cancellations, deadline)?;
    let charge = catalogs
        .iter()
        .try_fold(minimum, |total, catalog| {
            total
                .checked_add(catalog.retained_byte_charge())
                .ok_or(Error::Limit)
        })?
        .checked_add(authentication.len())
        .ok_or(Error::Limit)?;
    if charge > maximum {
        return Err(Error::Limit);
    }
    Ok((
        NativeMcpServerCandidate {
            server: Arc::from(configuration.name()),
            configuration: identity,
            authentication,
            catalogs,
            catalog_epoch: startup.catalog_epoch,
            peer,
            operation_timeout: Duration::from_millis(u64::from(
                configuration.operation_timeout_ms(),
            )),
            authority_cancellations: authority_cancellations.into(),
        },
        charge,
    ))
}

async fn stdio_server(
    startup: &NativeMcpStartup,
    configuration: &Arc<McpServerConfig>,
    guards: &[CancellationToken],
    owner: &AttemptOwner,
    attempts: &mut u16,
    deadline: Option<Instant>,
    maximum: usize,
) -> Result<(NativeMcpOwnedPeer, Vec<McpDescriptorCatalog>)> {
    let McpTransportConfig::Stdio(config) = configuration.transport() else {
        return Err(Error::Invalid);
    };
    let authority = startup.stdio.as_ref().ok_or(Error::Unavailable)?;
    let mut factory = authority
        .factory(configuration.clone())
        .map_err(|_| Error::Invalid)?;
    let mut last = Error::Unavailable;
    for _ in 0..=config.restart_limit() {
        control::check_optional(&startup.clock, guards, deadline)?;
        let cleanup_deadline = control::housekeeping_deadline(&startup.clock, deadline)?;
        owner
            .completion
            .settle(&startup.clock, guards, cleanup_deadline)
            .await?;
        *attempts += 1;
        let custody = owner.completion.clone();
        let observer =
            Arc::new(move |completion| custody.record(NativeMcpPeerCompletion::Stdio(completion)));
        let timeout = Duration::from_millis(u64::from(configuration.startup_timeout_ms()));
        let connect = async {
            match deadline {
                Some(deadline) => {
                    McpStdioPeer::connect_observed(
                        &mut factory,
                        startup.workers.clone(),
                        startup.clock.clone(),
                        owner.cancellation.clone(),
                        deadline,
                        timeout,
                        observer,
                    )
                    .await
                }
                None => {
                    McpStdioPeer::connect_configured_observed(
                        &mut factory,
                        startup.workers.clone(),
                        startup.clock.clone(),
                        owner.cancellation.clone(),
                        timeout,
                        observer,
                    )
                    .await
                }
            }
        };
        let result = control::bounded_optional(connect, &startup.clock, guards, deadline).await;
        let (mut peer, selected_deadline) = match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                last = stdio_error(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        peer.restrict_lifetime(startup.lifetime);
        let mut peer = NativeMcpOwnedPeer::Stdio(peer);
        match tools(
            startup,
            (configuration.name(), None),
            &mut peer,
            guards,
            selected_deadline,
            maximum,
        )
        .await
        {
            Ok(catalogs) => return Ok((peer, catalogs)),
            Err(error) => {
                last = error;
                drop(peer);
            }
        }
    }
    Err(last)
}

fn stdio_error(error: McpPeerError) -> Error {
    match error {
        McpPeerError::Cancelled => Error::Cancelled,
        McpPeerError::Deadline => Error::Deadline,
        McpPeerError::Capacity => Error::Limit,
        _ => Error::Unavailable,
    }
}

async fn tools(
    startup: &NativeMcpStartup,
    authentication: (&str, Option<&CancellationToken>),
    peer: &mut NativeMcpOwnedPeer,
    guards: &[CancellationToken],
    deadline: Instant,
    maximum: usize,
) -> Result<Vec<McpDescriptorCatalog>> {
    #[cfg(not(feature = "mcp-http"))]
    let _ = authentication;
    let supported = match peer {
        NativeMcpOwnedPeer::Stdio(peer) => peer.capabilities().tools(),
        #[cfg(feature = "mcp-http")]
        NativeMcpOwnedPeer::Http(peer) => peer.capabilities().tools(),
        #[cfg(test)]
        NativeMcpOwnedPeer::Script(_) => unreachable!("startup only creates concrete peers"),
    };
    if !supported {
        return Ok(Vec::new());
    }
    if maximum == 0 {
        return Err(Error::Limit);
    }
    let limits = McpCatalogLimits {
        max_item_bytes: maximum.min(McpCatalogLimits::default().max_item_bytes),
        ..McpCatalogLimits::default()
    };
    let epoch = startup.catalog_epoch;
    let fetch = async {
        match peer {
            NativeMcpOwnedPeer::Stdio(peer) => peer
                .catalog(McpCatalogKind::Tools, limits, epoch, deadline)
                .await
                .map_err(|_| Error::Catalog),
            #[cfg(feature = "mcp-http")]
            NativeMcpOwnedPeer::Http(peer) => peer
                .catalog(McpCatalogKind::Tools, limits, epoch, deadline)
                .await
                .map_err(|error| {
                    startup.observe_http_error(authentication.0, authentication.1, maximum, error);
                    Error::Catalog
                }),
            #[cfg(test)]
            NativeMcpOwnedPeer::Script(_) => unreachable!("startup only creates concrete peers"),
        }
    };
    let raw = control::bounded(fetch, &startup.clock, guards, deadline).await??;
    let limits = McpDescriptorLimits {
        max_catalog_bytes: maximum.min(McpDescriptorLimits::default().max_catalog_bytes),
        ..McpDescriptorLimits::default()
    };
    let catalog = McpDescriptorCatalog::admit(raw, limits).map_err(|_| Error::Catalog)?;
    control::check(&startup.clock, guards, deadline)?;
    Ok(vec![catalog])
}

#[cfg(feature = "mcp-http")]
async fn remote_server(
    startup: &NativeMcpStartup,
    configuration: &McpServerConfig,
    remote: &crate::mcp::config::McpRemoteConfig,
    guards: &[CancellationToken],
    owner: &AttemptOwner,
    deadline: Option<Instant>,
    maximum: usize,
) -> Result<(
    NativeMcpOwnedPeer,
    Vec<McpDescriptorCatalog>,
    Arc<[u8]>,
    Vec<CancellationToken>,
)> {
    use crate::mcp::{endpoint::McpEndpoint, http_peer::McpHttpPeer};
    let network = startup.network.as_ref().ok_or(Error::Unavailable)?;
    let mut selected = guards.to_vec();
    selected.push(network.owner_cancellation());
    let attempt_deadline = startup
        .clock
        .deadline(configuration.startup_timeout_ms(), deadline)?;
    let (headers, auth_generation) = control::bounded(
        startup.headers(
            configuration.name(),
            remote,
            &owner.cancellation,
            attempt_deadline,
        ),
        &startup.clock,
        &selected,
        attempt_deadline,
    )
    .await??;
    if let Some(generation) = &auth_generation {
        selected.push(generation.clone());
    }
    let endpoint = McpEndpoint::parse(remote.url()).map_err(|_| Error::Invalid)?;
    let admitted = control::bounded(
        network.admit_endpoint(&endpoint, &owner.cancellation, attempt_deadline),
        &startup.clock,
        &selected,
        attempt_deadline,
    )
    .await?
    .map_err(|_| Error::Unavailable)?;
    let authentication: Arc<[u8]> = headers.authentication_identity_bytes().into();
    if authentication.len() >= maximum {
        return Err(Error::Limit);
    }
    let custody = owner.completion.clone();
    let options = http_options(startup, configuration, admitted, headers);
    let observer =
        Arc::new(move |completion| custody.record(NativeMcpPeerCompletion::Http(completion)));
    let timeout = Duration::from_millis(u64::from(configuration.startup_timeout_ms()));
    let connect = async {
        let result = match deadline {
            Some(deadline) => {
                McpHttpPeer::connect_observed(
                    options,
                    owner.cancellation.clone(),
                    deadline,
                    timeout,
                    Some(attempt_deadline),
                    observer,
                )
                .await
            }
            None => {
                McpHttpPeer::connect_configured_observed(
                    options,
                    owner.cancellation.clone(),
                    timeout,
                    Some(attempt_deadline),
                    observer,
                )
                .await
            }
        };
        result.map_err(|error| {
            startup.observe_http_error(
                configuration.name(),
                auth_generation.as_ref(),
                maximum - authentication.len(),
                error,
            )
        })
    };
    let (peer, selected_deadline) =
        control::bounded_optional(connect, &startup.clock, &selected, deadline).await??;
    let mut peer = NativeMcpOwnedPeer::Http(Box::new(peer));
    let catalogs = tools(
        startup,
        (configuration.name(), auth_generation.as_ref()),
        &mut peer,
        &selected,
        selected_deadline,
        maximum - authentication.len(),
    )
    .await?;
    let mut generations = vec![network.owner_cancellation()];
    generations.extend(auth_generation);
    Ok((peer, catalogs, authentication, generations))
}

fn lifetime_deadline(startup: &NativeMcpStartup, outer: Option<Instant>) -> Option<Instant> {
    outer.map_or(startup.lifetime.deadline(), |deadline| {
        Some(startup.lifetime.constrain(deadline))
    })
}

#[cfg(feature = "mcp-http")]
fn http_options(
    startup: &NativeMcpStartup,
    configuration: &McpServerConfig,
    admitted: crate::mcp::auth::McpAuthDestination,
    headers: crate::mcp::headers::McpResolvedHeaders,
) -> crate::mcp::http_peer::McpHttpPeerOptions {
    use crate::mcp::{http_peer::McpHttpPeerOptions, protocol::TransportKind};
    McpHttpPeerOptions {
        destination: admitted.destination,
        trust: admitted.trust,
        headers,
        clock: startup.clock.clone(),
        transport: if matches!(configuration.transport(), McpTransportConfig::Sse(_)) {
            TransportKind::LegacySse
        } else {
            TransportKind::StreamableHttp
        },
        lifetime: startup.lifetime,
    }
}

fn remaining(startup: &NativeMcpStartup, server: &str, retained: usize) -> usize {
    #[cfg(feature = "mcp-http")]
    let challenge_charge = {
        startup.clear_challenge(server);
        startup.challenge_charge()
    };
    #[cfg(not(feature = "mcp-http"))]
    let challenge_charge = {
        let _ = server;
        0
    };
    startup
        .max_retained_bytes
        .saturating_sub(retained + challenge_charge)
}
