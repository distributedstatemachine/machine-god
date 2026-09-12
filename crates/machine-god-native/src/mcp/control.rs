//! Native-selected feature control, distinct from inert protocol descriptions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt, future::Future, sync::Arc, task::Poll, time::Instant};

use machine_god_core::{BoxFuture, CancellationToken};

use super::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    context::NativeMcpTurnContext,
    feature::{McpFeatureCodecError, McpFeatureCodecLimits, McpFeatureResponse},
    pagination::McpCatalogLimits,
};

enum Principal {
    Model(Arc<NativeMcpTurnContext>),
    Human(CancellationToken),
}
struct Selected {
    principal: Principal,
    operation: CancellationToken,
    runtime: CancellationToken,
    retired: Arc<AtomicBool>,
    guards: Arc<[CancellationToken]>,
}

/// Opaque native selection. Neither descriptors nor model JSON can mint it.
/// Cloning internally retains the same exact principal and retirement signals.
#[derive(Clone)]
pub struct McpFeatureControlAuthority(Arc<Selected>);
impl fmt::Debug for McpFeatureControlAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFeatureControlAuthority { <redacted> }")
    }
}
impl McpFeatureControlAuthority {
    pub(crate) fn for_model(
        context: Arc<NativeMcpTurnContext>,
        operation: CancellationToken,
        runtime: CancellationToken,
        retired: Arc<AtomicBool>,
        guards: Arc<[CancellationToken]>,
    ) -> Result<Self, McpFeatureCodecError> {
        Self::new(
            Principal::Model(context),
            operation,
            runtime,
            retired,
            guards,
        )
    }
    pub(crate) fn for_human(
        command: CancellationToken,
        operation: CancellationToken,
        runtime: CancellationToken,
        retired: Arc<AtomicBool>,
        guards: Arc<[CancellationToken]>,
    ) -> Result<Self, McpFeatureCodecError> {
        Self::new(
            Principal::Human(command),
            operation,
            runtime,
            retired,
            guards,
        )
    }
    fn new(
        principal: Principal,
        operation: CancellationToken,
        runtime: CancellationToken,
        retired: Arc<AtomicBool>,
        guards: Arc<[CancellationToken]>,
    ) -> Result<Self, McpFeatureCodecError> {
        if guards.len() > super::submission::MAX_MCP_RUNTIME_CANCELLATION_GUARDS {
            return Err(McpFeatureCodecError::Limit);
        }
        let selected = Self(Arc::new(Selected {
            principal,
            operation,
            runtime,
            retired,
            guards,
        }));
        if !selected.is_live() {
            return Err(McpFeatureCodecError::Closed);
        }
        Ok(selected)
    }
    pub(crate) fn is_live(&self) -> bool {
        let principal = match &self.0.principal {
            Principal::Model(context) => context.revalidate().is_ok(),
            Principal::Human(command) => !command.is_cancelled(),
        };
        principal
            && !self.0.retired.load(Ordering::Acquire)
            && !self.0.operation.is_cancelled()
            && !self.0.runtime.is_cancelled()
            && self.0.guards.iter().all(|guard| !guard.is_cancelled())
    }
    pub(crate) fn cancelled(&self) -> BoxFuture<'static, ()> {
        let authority = self.clone();
        let principal: BoxFuture<'static, ()> = match &self.0.principal {
            Principal::Model(context) => context.cancelled(),
            Principal::Human(command) => Box::pin(command.cancelled()),
        };
        let operation = self.0.operation.cancelled();
        let runtime = self.0.runtime.cancelled();
        let guards: Vec<_> = self
            .0
            .guards
            .iter()
            .map(CancellationToken::cancelled)
            .collect();
        Box::pin(async move {
            let mut principal = principal;
            let mut operation = std::pin::pin!(operation);
            let mut runtime = std::pin::pin!(runtime);
            let mut guards = guards;
            std::future::poll_fn(|cx| {
                if !authority.is_live()
                    || principal.as_mut().poll(cx).is_ready()
                    || operation.as_mut().poll(cx).is_ready()
                    || runtime.as_mut().poll(cx).is_ready()
                    || guards
                        .iter_mut()
                        .any(|guard| std::pin::Pin::new(guard).poll(cx).is_ready())
                {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
        })
    }
}

/// Explicit lowerable data bounds and metadata advertisements, never consent.
#[derive(Clone, Copy, Debug)]
pub struct McpFeatureOperationOptions {
    pub codec: McpFeatureCodecLimits,
    pub pagination: McpCatalogLimits,
    pub descriptors: McpDescriptorLimits,
    pub epoch: Instant,
    pub progress_token: Option<u64>,
    pub form: bool,
    pub url: bool,
}
impl McpFeatureOperationOptions {
    #[must_use]
    pub fn new(epoch: Instant) -> Self {
        Self {
            codec: McpFeatureCodecLimits::default(),
            pagination: McpCatalogLimits::default(),
            descriptors: McpDescriptorLimits::default(),
            epoch,
            progress_token: None,
            form: false,
            url: false,
        }
    }
}

/// Complete admitted data; publication and any input-required handling are separate.
#[derive(Debug)]
pub enum McpFeatureReply {
    Catalog(McpDescriptorCatalog),
    Response(McpFeatureResponse),
}

mod round;
pub(crate) use round::McpFeatureRound;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) async fn guarded<T>(
    authority: &McpFeatureControlAuthority,
    future: impl Future<Output = T>,
) -> Result<T, ()> {
    let mut future = std::pin::pin!(future);
    let mut cancelled = authority.cancelled();
    std::future::poll_fn(|cx| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(()));
        }
        let result = future.as_mut().poll(cx);
        if !authority.is_live() {
            return Poll::Ready(Err(()));
        }
        result.map(Ok)
    })
    .await
}

pub(crate) fn is_list(request: &crate::McpFeatureRequest) -> bool {
    matches!(
        request.action(),
        crate::McpFeatureAction::ResourceList
            | crate::McpFeatureAction::ResourceTemplates
            | crate::McpFeatureAction::PromptList
    )
}

pub(crate) fn prepare(
    request: &crate::McpFeatureRequest,
    server: &str,
    catalogs: &[McpDescriptorCatalog],
    options: super::feature::McpFeatureExchangeOptions,
    cursor: Option<&str>,
    settings: McpFeatureOperationOptions,
) -> Result<super::feature::McpFeatureExchange, McpFeatureCodecError> {
    let mut options = options.with_elicitation(settings.form, settings.url);
    if let Some(token) = settings.progress_token {
        options = options.with_progress_token(token);
    }
    super::feature::McpFeatureExchange::prepare(
        request,
        server,
        catalogs,
        options,
        cursor,
        settings.codec,
    )
}

pub(crate) fn admit(
    load: &mut Option<super::feature::McpFeatureCatalogLoad>,
    exchange: &super::feature::McpFeatureExchange,
    bytes: &[u8],
    now: Instant,
    epoch: Instant,
) -> Result<Option<McpFeatureReply>, McpFeatureCodecError> {
    let Some(builder) = load else {
        return exchange
            .admit_response(bytes)
            .map(McpFeatureReply::Response)
            .map(Some);
    };
    let elapsed = now
        .checked_duration_since(epoch)
        .ok_or(McpFeatureCodecError::InvalidLimits)?;
    let received = u64::try_from(elapsed.as_millis()).map_err(|_| McpFeatureCodecError::Limit)?;
    if builder.append(exchange, bytes, received)? {
        return Ok(None);
    }
    load.take()
        .ok_or(McpFeatureCodecError::Closed)?
        .finish()
        .map(McpFeatureReply::Catalog)
        .map(Some)
}
