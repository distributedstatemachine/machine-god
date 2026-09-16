//! Per-principal MCP selection over one shared engine, without owning reverse edges.
use super::principal::{
    NativeManagedCallLease, NativePrincipal, NativePrincipalRequester, NativePrincipalTurn,
    NativePrincipalTurnStamp,
};
use crate::{
    McpFeatureAuthority, McpFeatureError, McpFeatureErrorKind, McpFeaturePayload,
    McpFeatureRequest, McpToolCatalog, McpToolCatalogError, McpToolCatalogErrorKind,
    McpToolCatalogSnapshot, NativeMcpFeaturesTool, NativeToolResultArchiveAdapter,
    mcp::runtime::{NativeMcpPublicationCheckpoint, NativeMcpRuntime},
};
use crate::{NativePermissionActionPreparer, mcp::permission::NativeMcpPermissionPreparer};
use machine_god_core::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, ToolContext, TurnId,
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

#[path = "mcp/permission.rs"]
mod permission;
#[path = "mcp/tool.rs"]
mod tool;
#[cfg(any(test, feature = "ai-gateway-http"))]
pub(crate) use permission::NativePrincipalMcpPermissionInputs;
pub(crate) use permission::{NativePrincipalMcpPermissionRouter, NativePrincipalMcpPermissions};
pub(crate) use tool::NativePrincipalMcpTool;
#[cfg(test)]
#[path = "mcp/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrincipalMcpError {
    Unavailable,
    Duplicate,
    Limit,
}
type Result<T> = std::result::Result<T, PrincipalMcpError>;

struct Registry {
    principals: NativePrincipalRequester,
    archive: Arc<NativeToolResultArchiveAdapter>,
    routes: Mutex<Vec<Route>>,
    limit: usize,
}
struct Route {
    owner: Weak<NativePrincipalMcpOwner>,
    runtime: Weak<NativeMcpRuntime>,
    retired: Arc<AtomicBool>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        let routes = std::mem::take(
            self.routes
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        // Upgrades and runtime cancellation are outside any registry lock.
        for route in routes {
            if let Some(owner) = route.owner.upgrade() {
                owner.retire();
            }
        }
    }
}
pub(crate) struct NativePrincipalMcpRegistry(Arc<Registry>);
impl NativePrincipalMcpRegistry {
    pub(crate) fn new(
        limit: usize,
        principals: NativePrincipalRequester,
        archive: Arc<NativeToolResultArchiveAdapter>,
    ) -> Result<Self> {
        if !(1..=64).contains(&limit) {
            return Err(PrincipalMcpError::Limit);
        }
        Ok(Self(Arc::new(Registry {
            principals,
            archive,
            routes: Mutex::new(Vec::new()),
            limit,
        })))
    }
    pub(crate) fn requester(&self) -> NativePrincipalMcpRequester {
        NativePrincipalMcpRequester(Arc::downgrade(&self.0))
    }
    /// Factory receives explicit independent runtime authority, never profile/current-session discovery.
    pub(crate) fn register(
        &self,
        principal: &Arc<NativePrincipal>,
        runtime: &Arc<NativeMcpRuntime>,
        portable: Option<Arc<dyn McpFeatureAuthority>>,
        permissions: Option<&NativePrincipalMcpPermissions>,
    ) -> Result<Arc<NativePrincipalMcpOwner>> {
        if !principal.is_live() {
            return Err(PrincipalMcpError::Unavailable);
        }
        let permissions = permissions
            .map(|permissions| permissions.for_runtime(runtime))
            .transpose()?;
        let runtime = Arc::downgrade(runtime);
        let owner = Arc::new(NativePrincipalMcpOwner {
            principal: Arc::downgrade(principal),
            generation: principal.generation(),
            runtime: runtime.clone(),
            features: Arc::new(NativeMcpFeaturesTool::new(
                runtime.clone(),
                self.0.archive.clone(),
            )),
            portable,
            permissions,
            active: Mutex::new(None),
            retired: Arc::new(AtomicBool::new(true)),
            registry: Arc::downgrade(&self.0),
        });
        let mut routes = self
            .0
            .routes
            .lock()
            .map_err(|_| PrincipalMcpError::Unavailable)?;
        routes.retain(|route| route.owner.strong_count() != 0 || route.runtime.strong_count() != 0);
        let candidates: Vec<_> = routes
            .iter()
            .filter_map(|route| route.owner.upgrade())
            .collect();
        let result = if routes.len() >= self.0.limit {
            Err(PrincipalMcpError::Limit)
        } else if routes.iter().any(|route| route.runtime.ptr_eq(&runtime))
            || candidates
                .iter()
                .any(|candidate| candidate.principal.ptr_eq(&owner.principal))
        {
            Err(PrincipalMcpError::Duplicate)
        } else if !principal.is_live()
            || runtime
                .upgrade()
                .is_none_or(|runtime| runtime.publication_checkpoint().is_err())
        {
            Err(PrincipalMcpError::Unavailable)
        } else {
            owner.retired.store(false, Ordering::Release);
            routes.push(Route {
                owner: Arc::downgrade(&owner),
                runtime,
                retired: owner.retired.clone(),
            });
            Ok(owner.clone())
        };
        drop(routes);
        drop(candidates);
        // Rejected candidate must not close a runtime belonging to a live owner.
        if result.is_err() {
            owner.retired.store(true, Ordering::Release);
        }
        result
    }
    pub(crate) fn wrap_tool(
        &self,
        inner: Arc<dyn machine_god_core::Tool>,
    ) -> NativePrincipalMcpTool {
        NativePrincipalMcpTool::fixed(self.requester(), inner)
    }
    pub(crate) fn features_tool(&self) -> NativePrincipalMcpTool {
        NativePrincipalMcpTool::features(
            self.requester(),
            Arc::new(NativeMcpFeaturesTool::new(
                Weak::new(),
                self.0.archive.clone(),
            )),
        )
    }
    pub(crate) fn permission_preparer(&self) -> NativePrincipalMcpPermissionRouter {
        NativePrincipalMcpPermissionRouter(self.requester())
    }
}

/// Outer manager owns this registration and the runtime/controller/ephemeral owners separately.
/// Runtime and principal references here are weak; destruction invalidates only this runtime.
pub(crate) struct NativePrincipalMcpOwner {
    principal: Weak<NativePrincipal>,
    generation: u64,
    runtime: Weak<NativeMcpRuntime>,
    features: Arc<NativeMcpFeaturesTool>,
    portable: Option<Arc<dyn McpFeatureAuthority>>,
    permissions: Option<Weak<NativeMcpPermissionPreparer>>,
    active: Mutex<Option<Weak<TurnRoute>>>,
    retired: Arc<AtomicBool>,
    registry: Weak<Registry>,
}
impl NativePrincipalMcpOwner {
    pub(crate) fn matches_principal(&self, principal: &Arc<NativePrincipal>) -> bool {
        self.live()
            && self.generation == principal.generation()
            && self.principal.ptr_eq(&Arc::downgrade(principal))
    }
    fn live(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
            && self.registry.strong_count() != 0
            && self.runtime.strong_count() != 0
            && self.principal.upgrade().is_some_and(|principal| {
                principal.is_live() && principal.generation() == self.generation
            })
    }
    pub(crate) fn begin_turn(
        self: &Arc<Self>,
        turn: &NativePrincipalTurn,
    ) -> Result<NativePrincipalMcpTurn> {
        let stamp = turn.stamp();
        let principal = self
            .principal
            .upgrade()
            .ok_or(PrincipalMcpError::Unavailable)?;
        if !self.live() || !stamp.matches_principal(&principal) {
            return Err(PrincipalMcpError::Unavailable);
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| PrincipalMcpError::Unavailable)?;
        if !self.live()
            || active
                .as_ref()
                .is_some_and(|route| route.strong_count() != 0)
        {
            return Err(PrincipalMcpError::Duplicate);
        }
        let state = Arc::new(TurnRoute {
            owner: Arc::downgrade(self),
            stamp,
            session: principal.owner().session_id().clone(),
            incarnation: principal.owner().session_incarnation_id().clone(),
            turn: turn.turn_id().clone(),
        });
        *active = Some(Arc::downgrade(&state));
        Ok(NativePrincipalMcpTurn { state })
    }
    pub(crate) fn retire(&self) {
        if self.retired.swap(true, Ordering::AcqRel) {
            return;
        }
        let route = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(route) = route.and_then(|route| route.upgrade()) {
            self.close_permissions(&route);
        }
        // Keep the allocation's route occupied until synchronous generation
        // invalidation completes; this does not prove worker/reap settlement.
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.close();
        }
        if let Some(registry) = self.registry.upgrade() {
            registry
                .routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|route| !Arc::ptr_eq(&route.retired, &self.retired));
        }
    }
    fn close_permissions(&self, route: &TurnRoute) {
        if let Some(preparer) = self.permissions.as_ref().and_then(Weak::upgrade) {
            preparer.close_turn(&route.session, &route.incarnation, &route.turn);
        }
    }
}
impl Drop for NativePrincipalMcpOwner {
    fn drop(&mut self) {
        self.retire();
    }
}

struct TurnRoute {
    owner: Weak<NativePrincipalMcpOwner>,
    stamp: NativePrincipalTurnStamp,
    // Frozen lookup keys, not authority; retained so cancellation/guard drop
    // can retire original unclaimed proofs even after the principal guard ends.
    session: SessionId,
    incarnation: SessionIncarnationId,
    turn: TurnId,
}
impl TurnRoute {
    fn select(self: &Arc<Self>) -> Result<Selection> {
        if !self.live() {
            return Err(PrincipalMcpError::Unavailable);
        }
        let owner = self.owner.upgrade().ok_or(PrincipalMcpError::Unavailable)?;
        Ok(Selection {
            route: self.clone(),
            runtime: owner
                .runtime
                .upgrade()
                .ok_or(PrincipalMcpError::Unavailable)?,
            features: owner.features.clone(),
            portable: owner.portable.clone(),
        })
    }
    fn live(&self) -> bool {
        self.stamp.is_live()
            && self.owner.upgrade().is_some_and(|owner| {
                owner.live()
                    && owner.active.lock().is_ok_and(|active| {
                        active
                            .as_ref()
                            .is_some_and(|route| std::ptr::eq(route.as_ptr(), self))
                    })
            })
    }
}
pub(crate) struct NativePrincipalMcpTurn {
    state: Arc<TurnRoute>,
}
impl Drop for NativePrincipalMcpTurn {
    fn drop(&mut self) {
        if let Some(owner) = self.state.owner.upgrade() {
            let mut active = owner
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if active
                .as_ref()
                .is_some_and(|route| route.ptr_eq(&Arc::downgrade(&self.state)))
            {
                *active = None;
                drop(active);
                owner.close_permissions(&self.state);
            }
        }
    }
}

struct Selection {
    route: Arc<TurnRoute>,
    runtime: Arc<NativeMcpRuntime>,
    features: Arc<NativeMcpFeaturesTool>,
    portable: Option<Arc<dyn McpFeatureAuthority>>,
}
impl Registry {
    fn route(&self, stamp: &NativePrincipalTurnStamp) -> Result<Weak<TurnRoute>> {
        if !stamp.is_live() {
            return Err(PrincipalMcpError::Unavailable);
        }
        let candidates: Vec<_> = self
            .routes
            .lock()
            .map_err(|_| PrincipalMcpError::Unavailable)?
            .iter()
            .filter_map(|route| route.owner.upgrade())
            .collect();
        for owner in candidates {
            let route = owner
                .active
                .lock()
                .map_err(|_| PrincipalMcpError::Unavailable)?
                .as_ref()
                .and_then(Weak::upgrade);
            let Some(route) = route.filter(|route| route.stamp.same_turn(stamp)) else {
                continue;
            };
            if !route.live() {
                return Err(PrincipalMcpError::Unavailable);
            }
            return Ok(Arc::downgrade(&route));
        }
        Err(PrincipalMcpError::Unavailable)
    }
    fn select_context(&self, context: &ToolContext) -> Result<Selection> {
        self.route(
            &self
                .principals
                .stamp(context)
                .map_err(|_| PrincipalMcpError::Unavailable)?,
        )?
        .upgrade()
        .ok_or(PrincipalMcpError::Unavailable)?
        .select()
    }
}

impl Selection {
    fn matches_lease(&self, lease: &NativeManagedCallLease) -> bool {
        lease.is_live()
            && self.route.live()
            && self.route.stamp.matches_principal(lease.principal())
            && self.route.stamp.matches_turn(lease.witness())
    }
}

#[derive(Clone)]
pub(crate) struct NativePrincipalMcpRequester(Weak<Registry>);
struct CapturedRoute {
    route: Weak<TurnRoute>,
    publication: NativeMcpPublicationCheckpoint,
}
impl NativePrincipalMcpRequester {
    // Bounded non-consuming lookup captures identity, not a runtime owner or
    // authority. Resolving only on first poll could retarget a reopened owner.
    fn capture(&self, context: &ToolContext) -> Result<CapturedRoute> {
        let registry = self.0.upgrade().ok_or(PrincipalMcpError::Unavailable)?;
        let stamp = registry
            .principals
            .stamp(context)
            .map_err(|_| PrincipalMcpError::Unavailable)?;
        capture_route(registry.route(&stamp)?)
    }
}
fn capture_route(route: Weak<TurnRoute>) -> Result<CapturedRoute> {
    let selected = route
        .upgrade()
        .ok_or(PrincipalMcpError::Unavailable)?
        .select()?;
    let publication = selected
        .runtime
        .publication_checkpoint()
        .map_err(|_| PrincipalMcpError::Unavailable)?;
    Ok(CapturedRoute { route, publication })
}
fn select_captured(captured: Result<CapturedRoute>) -> Result<Selection> {
    let captured = captured?;
    let selected = captured
        .route
        .upgrade()
        .ok_or(PrincipalMcpError::Unavailable)?
        .select()?;
    let current = selected
        .runtime
        .publication_checkpoint()
        .map_err(|_| PrincipalMcpError::Unavailable)?;
    if !captured.publication.same_selection(&current) {
        return Err(PrincipalMcpError::Unavailable);
    }
    Ok(selected)
}
impl McpToolCatalog for NativePrincipalMcpRequester {
    fn snapshot_for_turn(
        &self,
        context: ToolContext,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        let route = self.capture(&context);
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(McpToolCatalogError::new(McpToolCatalogErrorKind::Cancelled));
            }
            let selected = select_captured(route).map_err(|_| catalog_error())?;
            let snapshot = selected
                .runtime
                .snapshot_for_turn(context, cancellation.clone())
                .await?;
            if cancellation.is_cancelled() {
                return Err(McpToolCatalogError::new(McpToolCatalogErrorKind::Cancelled));
            }
            if !selected.route.live() {
                return Err(catalog_error());
            }
            Ok(snapshot)
        })
    }
}
impl McpFeatureAuthority for NativePrincipalMcpRequester {
    fn call_for_turn(
        &self,
        context: ToolContext,
        request: McpFeatureRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<McpFeaturePayload, McpFeatureError>> {
        let route = self.capture(&context);
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(McpFeatureError::new(McpFeatureErrorKind::Cancelled));
            }
            let selected = select_captured(route).map_err(|_| feature_error())?;
            let authority = selected.portable.as_ref().ok_or_else(feature_error)?;
            let result = authority
                .call_for_turn(context, request, cancellation.clone())
                .await?;
            if cancellation.is_cancelled() {
                return Err(McpFeatureError::new(McpFeatureErrorKind::Cancelled));
            }
            if !selected.route.live() {
                return Err(feature_error());
            }
            Ok(result)
        })
    }
}
fn catalog_error() -> McpToolCatalogError {
    McpToolCatalogError::new(McpToolCatalogErrorKind::Unavailable)
}
fn feature_error() -> McpFeatureError {
    McpFeatureError::new(McpFeatureErrorKind::Unavailable)
}

macro_rules! redacted_debug { ($($ty:ty),+ $(,)?)=>{$(
    impl std::fmt::Debug for $ty { fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.debug_struct(stringify!($ty)).finish_non_exhaustive() } }
)+}; }
redacted_debug!(
    NativePrincipalMcpRegistry,
    NativePrincipalMcpRequester,
    NativePrincipalMcpOwner,
    NativePrincipalMcpTurn
);
