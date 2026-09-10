//! Non-owning, generation-bound background UI access to the full terminal host.

use super::Requester;
use super::lifecycle::TerminalAccessPrincipals;
use crate::terminal_catalog_view::{TerminalCatalogViewError, describe_selected_with};
use crate::terminal_host_dispatch::{
    TerminalCatalogState, TerminalHostDispatchError, dispatch_with_access,
};
use crate::terminal_input::TerminalWriterId;
use crate::terminal_journal::TerminalJournalPage;
use crate::terminal_monitor::TerminalMonitorActivation;
use crate::terminal_owner::{TerminalOwnerContext, TerminalOwnerError};
use crate::terminal_registry::TerminalRegistryError;
use crate::terminal_resident_dispatch::{TerminalResidentAuthority, TerminalResidentError};
use crate::terminal_runtime::{TerminalRuntimeError, TerminalRuntimeRequester};
use crate::terminal_session::{TerminalSessionBackend, TerminalSessionError};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, TerminalActionRequest,
    TerminalActionResult, TerminalActorRole, TerminalAllowedControls, TerminalClosePolicy,
    TerminalCursor, TerminalEventQuery, TerminalGap, TerminalSessionFacts, TerminalSessionId,
};
use std::fmt;

const MAX_ROWS: usize = 128;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_PAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTerminalBackgroundError {
    Cancelled,
    Closed,
    NotFound,
    Revoked,
    Invalid,
    ResourceLimit,
    Unavailable,
    /// A native effect committed but its final public projection was unavailable.
    Committed,
    /// An admitted operation may have effects; never infer rollback or retry it.
    Uncertain,
}
impl fmt::Display for NativeTerminalBackgroundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("terminal background operation unavailable")
    }
}
impl std::error::Error for NativeTerminalBackgroundError {}
type Result<T> = std::result::Result<T, NativeTerminalBackgroundError>;

macro_rules! redacted {
    ($($name:ident),+ $(,)?) => { $(impl fmt::Debug for $name {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($name)).finish_non_exhaustive()
        }
    })+ };
}

/// Bounded descriptive data. Neither saved text nor public controls grant authority.
#[derive(Clone)]
pub struct NativeTerminalBackgroundEntry {
    facts: TerminalSessionFacts,
    created_at_ms: i64,
    last_output_ms: i64,
    command: Option<String>,
    cwd: String,
    recovered: bool,
    owns_backend: bool,
}
impl NativeTerminalBackgroundEntry {
    #[must_use]
    pub fn id(&self) -> &TerminalSessionId {
        &self.facts.session_id
    }
    #[must_use]
    pub fn facts(&self) -> &TerminalSessionFacts {
        &self.facts
    }
    #[must_use]
    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }
    #[must_use]
    pub fn last_output_ms(&self) -> i64 {
        self.last_output_ms
    }
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        self.command.as_deref()
    }
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
    #[must_use]
    pub fn recovered(&self) -> bool {
        self.recovered
    }
    #[must_use]
    pub fn owns_backend(&self) -> bool {
        self.owns_backend
    }
}

/// Complete, creation-time-descending snapshot; equal times use descending IDs.
pub struct NativeTerminalBackgroundSnapshot {
    entries: Vec<NativeTerminalBackgroundEntry>,
}
impl NativeTerminalBackgroundSnapshot {
    #[must_use]
    pub fn entries(&self) -> &[NativeTerminalBackgroundEntry] {
        &self.entries
    }
}

/// Selection fixes one ID, owner, host registry and access generation exactly once.
/// It is pure observation/routing data, not a host or backend lifetime vote.
#[derive(Clone)]
pub struct NativeTerminalBackgroundTarget {
    owner: BackgroundOutputOwner,
    id: TerminalSessionId,
    access: CancellationToken,
    writer: TerminalWriterId,
    principals: TerminalAccessPrincipals,
}
impl NativeTerminalBackgroundTarget {
    #[must_use]
    pub fn id(&self) -> &TerminalSessionId {
        &self.id
    }
    /// Non-authority observation for a native URL opener's final admission check.
    pub(crate) fn revocation_token(&self) -> CancellationToken {
        self.access.clone()
    }
}

/// Raw, lossless durable-log page. Gaps and cursor bounds are never hidden.
pub struct NativeTerminalBackgroundPage {
    page: TerminalJournalPage,
}
impl NativeTerminalBackgroundPage {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.page.bytes
    }
    #[must_use]
    pub fn next(&self) -> &TerminalCursor {
        &self.page.next
    }
    #[must_use]
    pub fn earliest(&self) -> &TerminalCursor {
        &self.page.earliest
    }
    #[must_use]
    pub fn latest(&self) -> &TerminalCursor {
        &self.page.latest
    }
    #[must_use]
    pub fn gap(&self) -> Option<&TerminalGap> {
        self.page.gap.as_ref()
    }
}

/// Validated inspect facts plus native ownership observed at the same admission.
pub struct NativeTerminalBackgroundInspection {
    result: TerminalActionResult,
    owned_backend_at_admission: bool,
}
impl NativeTerminalBackgroundInspection {
    #[must_use]
    pub fn result(&self) -> &TerminalActionResult {
        &self.result
    }
    #[must_use]
    pub fn owns_backend(&self) -> bool {
        self.owned_backend_at_admission
    }
}

/// Closing recovered history is not evidence that a native process was stopped.
pub struct NativeTerminalBackgroundStopReceipt {
    result: TerminalActionResult,
    was_live: bool,
}
impl NativeTerminalBackgroundStopReceipt {
    #[must_use]
    pub fn result(&self) -> &TerminalActionResult {
        &self.result
    }
    #[must_use]
    pub fn was_live(&self) -> bool {
        self.was_live
    }
}

#[derive(Clone)]
pub struct NativeTerminalBackgroundRequester {
    requester: Requester,
    principals: TerminalAccessPrincipals,
}
redacted!(
    NativeTerminalBackgroundEntry,
    NativeTerminalBackgroundSnapshot,
    NativeTerminalBackgroundTarget,
    NativeTerminalBackgroundPage,
    NativeTerminalBackgroundInspection,
    NativeTerminalBackgroundStopReceipt,
    NativeTerminalBackgroundRequester
);

impl NativeTerminalBackgroundRequester {
    pub(super) fn new(requester: Requester, principals: TerminalAccessPrincipals) -> Self {
        Self {
            requester,
            principals,
        }
    }

    /// No effects before polling. Unknown/retired principals never create authority.
    #[must_use]
    pub fn snapshot(
        &self,
        owner: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundSnapshot>> {
        snapshot_request(
            self.requester.clone(),
            self.principals.clone(),
            owner,
            cancellation,
        )
    }

    /// None selects the latest creation time, then greatest ID. Any incomplete
    /// catalog, duplicate identity or resource overflow fails the whole selection.
    #[must_use]
    pub fn select(
        &self,
        owner: BackgroundOutputOwner,
        id: Option<TerminalSessionId>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundTarget>> {
        select_request(
            self.requester.clone(),
            self.principals.clone(),
            owner,
            id,
            cancellation,
        )
    }

    #[must_use]
    pub fn inspect(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundInspection>> {
        let request = TerminalActionRequest::Inspect {
            session_id: target.id.clone(),
            events: TerminalEventQuery {
                after_event_id: 0,
                acknowledge_event_id: None,
                max_events: 1,
            },
        };
        let operation = action_request(
            self.requester.clone(),
            self.principals.clone(),
            target.clone(),
            request,
            cancellation,
        );
        Box::pin(async move {
            let reply = operation.await?;
            Ok(NativeTerminalBackgroundInspection {
                result: reply.result,
                owned_backend_at_admission: reply.owned_backend_at_admission,
            })
        })
    }

    /// One bounded page; callers may compose head/tail or URL evidence under
    /// their own smaller aggregate budget. No bytes are cached by the requester.
    #[must_use]
    pub fn read(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cursor: TerminalCursor,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundPage>> {
        read_request(
            self.requester.clone(),
            self.principals.clone(),
            target.clone(),
            cursor,
            maximum,
            cancellation,
        )
    }

    /// Start of up to 64 KiB of retained tail bytes, including previous segments.
    /// Subsequent pages retain ordinary gap evidence if retention advances.
    #[must_use]
    pub fn tail_start(
        &self,
        target: &NativeTerminalBackgroundTarget,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalCursor>> {
        tail_request(
            self.requester.clone(),
            self.principals.clone(),
            target.clone(),
            maximum,
            cancellation,
        )
    }

    /// Graceful close uses explicit Human authority, never a persisted PID.
    /// Cancellation/drop after execution begins cannot erase committed effects.
    #[must_use]
    pub fn stop(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalBackgroundStopReceipt>> {
        let request = TerminalActionRequest::Close {
            session_id: target.id.clone(),
            policy: TerminalClosePolicy::Graceful,
        };
        let operation = action_request(
            self.requester.clone(),
            self.principals.clone(),
            target.clone(),
            request,
            cancellation,
        );
        Box::pin(async move {
            let reply = operation.await?;
            Ok(NativeTerminalBackgroundStopReceipt {
                result: reply.result,
                was_live: reply.owned_backend_at_admission,
            })
        })
    }
}

fn snapshot_request<B, S>(
    requester: TerminalRuntimeRequester<B, S>,
    principals: TerminalAccessPrincipals,
    owner: BackgroundOutputOwner,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeTerminalBackgroundSnapshot>>
where
    B: TerminalSessionBackend + Send + 'static,
    S: TerminalCatalogState + 'static,
{
    Box::pin(async move {
        check_cancel(&cancellation)?;
        let (access, _) = principals
            .current(&owner)
            .ok_or(NativeTerminalBackgroundError::NotFound)?;
        requester
            .request_with_context(cancellation, move |mut context| {
                snapshot_with(&mut context, &owner, &access)
            })
            .await
            .map_err(runtime_error)?
    })
}

fn select_request<B, S>(
    requester: TerminalRuntimeRequester<B, S>,
    principals: TerminalAccessPrincipals,
    owner: BackgroundOutputOwner,
    id: Option<TerminalSessionId>,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeTerminalBackgroundTarget>>
where
    B: TerminalSessionBackend + Send + 'static,
    S: TerminalCatalogState + 'static,
{
    Box::pin(async move {
        check_cancel(&cancellation)?;
        let (access, writer) = principals
            .current(&owner)
            .ok_or(NativeTerminalBackgroundError::NotFound)?;
        let admitted_owner = owner.clone();
        let admitted_access = access.clone();
        let snapshot = requester
            .request_with_context(cancellation, move |mut context| {
                snapshot_with(&mut context, &admitted_owner, &admitted_access)
            })
            .await
            .map_err(runtime_error)??;
        if access.is_cancelled() {
            return Err(NativeTerminalBackgroundError::Revoked);
        }
        let row = match id {
            Some(id) => snapshot.entries.iter().find(|row| row.id() == &id),
            None => snapshot.entries.first(),
        }
        .ok_or(NativeTerminalBackgroundError::NotFound)?;
        Ok(NativeTerminalBackgroundTarget {
            owner,
            id: row.id().clone(),
            access,
            writer,
            principals,
        })
    })
}

fn snapshot_with<B: TerminalSessionBackend, S: TerminalCatalogState>(
    context: &mut TerminalOwnerContext<'_, B, S>,
    owner: &BackgroundOutputOwner,
    generation: &CancellationToken,
) -> Result<NativeTerminalBackgroundSnapshot> {
    if generation.is_cancelled() {
        return Err(NativeTerminalBackgroundError::Revoked);
    }
    let access = context
        .state
        .access()
        .ok_or(NativeTerminalBackgroundError::Unavailable)?
        .snapshot();
    access.check(owner).map_err(catalog_error)?;
    let mut entries = Vec::new();
    let mut remaining_text = MAX_TEXT_BYTES;
    for origin in access.origins(owner) {
        check_cancel(context.cancellation)?;
        let catalog = context
            .state
            .catalogs()
            .catalog(context.store, &origin, context.cancellation)
            .map_err(catalog_error)?;
        let rows = describe_selected_with(
            context.registry,
            context.store,
            catalog,
            *context.budget,
            &origin,
            TerminalActorRole::Human,
            &controls(),
            context.now_ms,
            MAX_ROWS - entries.len(),
            |id| access.visible(owner, &origin, id),
            |facts, public, resident, owns_backend| {
                if generation.is_cancelled() || context.cancellation.is_cancelled() {
                    return Err(TerminalCatalogViewError::Cancelled);
                }
                let metadata = facts.metadata.ok_or(TerminalCatalogViewError::Invalid)?;
                let text = metadata
                    .cwd
                    .len()
                    .checked_add(metadata.command.as_ref().map_or(0, String::len))
                    .ok_or(TerminalCatalogViewError::ResourceLimit)?;
                remaining_text = remaining_text
                    .checked_sub(text)
                    .ok_or(TerminalCatalogViewError::ResourceLimit)?;
                Ok(NativeTerminalBackgroundEntry {
                    facts: public,
                    created_at_ms: facts.created_at_ms,
                    last_output_ms: facts.last_output_ms,
                    command: metadata.command,
                    cwd: metadata.cwd,
                    recovered: !resident,
                    owns_backend,
                })
            },
        )
        .map_err(catalog_error)?;
        entries.extend(rows);
    }
    entries.sort_unstable_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.id().as_str().cmp(a.id().as_str()))
    });
    let mut ids: Vec<_> = entries
        .iter()
        .map(NativeTerminalBackgroundEntry::id)
        .collect();
    ids.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(NativeTerminalBackgroundError::Invalid);
    }
    if generation.is_cancelled() {
        return Err(NativeTerminalBackgroundError::Revoked);
    }
    Ok(NativeTerminalBackgroundSnapshot { entries })
}

fn check_target(
    principals: &TerminalAccessPrincipals,
    target: &NativeTerminalBackgroundTarget,
) -> Result<()> {
    if !principals.same_registry(&target.principals) {
        return Err(NativeTerminalBackgroundError::NotFound);
    }
    if target.access.is_cancelled() {
        return Err(NativeTerminalBackgroundError::Revoked);
    }
    Ok(())
}

fn action_request<B, S>(
    requester: TerminalRuntimeRequester<B, S>,
    principals: TerminalAccessPrincipals,
    target: NativeTerminalBackgroundTarget,
    request: TerminalActionRequest,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<crate::terminal_host_dispatch::TerminalHostReply>>
where
    B: TerminalSessionBackend + Send + 'static,
    S: TerminalCatalogState + 'static,
{
    Box::pin(async move {
        check_cancel(&cancellation)?;
        check_target(&principals, &target)?;
        let access = target.revocation_token();
        dispatch_with_access(
            requester,
            TerminalResidentAuthority {
                owner: target.owner,
                actor: TerminalActorRole::Human,
                writer: target.writer,
                controls: controls(),
                revoke_authorized: false,
            },
            request,
            TerminalMonitorActivation::default(),
            cancellation,
            Some(access),
        )
        .await
        .map_err(|error| dispatch_error(&error))
    })
}

fn read_request<B, S>(
    requester: TerminalRuntimeRequester<B, S>,
    principals: TerminalAccessPrincipals,
    target: NativeTerminalBackgroundTarget,
    cursor: TerminalCursor,
    maximum: usize,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeTerminalBackgroundPage>>
where
    B: TerminalSessionBackend + Send + 'static,
    S: TerminalCatalogState + 'static,
{
    Box::pin(async move {
        check_cancel(&cancellation)?;
        check_target(&principals, &target)?;
        if !(1..=MAX_PAGE_BYTES).contains(&maximum) || cursor.validate().is_err() {
            return Err(NativeTerminalBackgroundError::Invalid);
        }
        requester
            .request_with_context(cancellation, move |context| {
                check_target(&principals, &target)?;
                let owner = context
                    .state
                    .access()
                    .ok_or(NativeTerminalBackgroundError::Unavailable)?
                    .resolve(&target.owner, &target.id)
                    .map_err(catalog_error)?;
                let page = match context.registry.authorize_resident(&owner, &target.id) {
                    Ok(()) => context
                        .registry
                        .read(&owner, &target.id, &cursor, maximum)
                        .map_err(registry_error)?,
                    Err(TerminalRegistryError::NotFound) => context
                        .state
                        .catalogs()
                        .with_recovered(
                            context.store,
                            *context.budget,
                            &owner,
                            &target.id,
                            context.now_ms,
                            context.cancellation,
                            |session, _| session.read(&owner, &cursor, maximum),
                        )
                        .map_err(catalog_error)?,
                    Err(error) => return Err(registry_error(error)),
                };
                if page.bytes.len() > maximum
                    || page.earliest > page.latest
                    || page.next > page.latest
                    || page.next < cursor
                    || page.next.offset() < page.bytes.len() as u64
                {
                    return Err(NativeTerminalBackgroundError::Unavailable);
                }
                Ok(NativeTerminalBackgroundPage { page })
            })
            .await
            .map_err(runtime_error)?
    })
}

fn controls() -> TerminalAllowedControls {
    TerminalAllowedControls {
        read: true,
        inspect: true,
        list: true,
        close: true,
        ..TerminalAllowedControls::default()
    }
}

fn tail_request<B, S>(
    requester: TerminalRuntimeRequester<B, S>,
    principals: TerminalAccessPrincipals,
    target: NativeTerminalBackgroundTarget,
    maximum: usize,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<TerminalCursor>>
where
    B: TerminalSessionBackend + Send + 'static,
    S: TerminalCatalogState + 'static,
{
    Box::pin(async move {
        check_cancel(&cancellation)?;
        check_target(&principals, &target)?;
        if !(1..=MAX_PAGE_BYTES).contains(&maximum) {
            return Err(NativeTerminalBackgroundError::Invalid);
        }
        requester
            .request_with_context(cancellation, move |context| {
                check_target(&principals, &target)?;
                let owner = context
                    .state
                    .access()
                    .ok_or(NativeTerminalBackgroundError::Unavailable)?
                    .resolve(&target.owner, &target.id)
                    .map_err(catalog_error)?;
                match context.registry.authorize_resident(&owner, &target.id) {
                    Ok(()) => context
                        .registry
                        .tail_start(&owner, &target.id, maximum)
                        .map_err(registry_error),
                    Err(TerminalRegistryError::NotFound) => context
                        .state
                        .catalogs()
                        .with_recovered(
                            context.store,
                            *context.budget,
                            &owner,
                            &target.id,
                            context.now_ms,
                            context.cancellation,
                            |session, _| session.tail_start(&owner, maximum),
                        )
                        .map_err(catalog_error),
                    Err(error) => Err(registry_error(error)),
                }
            })
            .await
            .map_err(runtime_error)?
    })
}
fn check_cancel(token: &CancellationToken) -> Result<()> {
    if token.is_cancelled() {
        Err(NativeTerminalBackgroundError::Cancelled)
    } else {
        Ok(())
    }
}
fn runtime_error(error: TerminalRuntimeError) -> NativeTerminalBackgroundError {
    match error {
        TerminalRuntimeError::Owner(TerminalOwnerError::Cancelled) => {
            NativeTerminalBackgroundError::Cancelled
        }
        TerminalRuntimeError::Owner(TerminalOwnerError::Closed) => {
            NativeTerminalBackgroundError::Closed
        }
        TerminalRuntimeError::Owner(TerminalOwnerError::Busy) => {
            NativeTerminalBackgroundError::ResourceLimit
        }
        TerminalRuntimeError::Spawn | TerminalRuntimeError::Initialization => {
            NativeTerminalBackgroundError::Unavailable
        }
        _ => NativeTerminalBackgroundError::Uncertain,
    }
}
fn registry_error(error: TerminalRegistryError) -> NativeTerminalBackgroundError {
    match error {
        TerminalRegistryError::NotFound => NativeTerminalBackgroundError::NotFound,
        TerminalRegistryError::Capacity | TerminalRegistryError::Busy => {
            NativeTerminalBackgroundError::ResourceLimit
        }
        TerminalRegistryError::Session(TerminalSessionError::NotFound) => {
            NativeTerminalBackgroundError::NotFound
        }
        _ => NativeTerminalBackgroundError::Unavailable,
    }
}
fn catalog_error(error: TerminalCatalogViewError) -> NativeTerminalBackgroundError {
    match error {
        TerminalCatalogViewError::Cancelled => NativeTerminalBackgroundError::Cancelled,
        TerminalCatalogViewError::ResourceLimit => NativeTerminalBackgroundError::ResourceLimit,
        TerminalCatalogViewError::Registry(error) => registry_error(error),
        _ => NativeTerminalBackgroundError::Unavailable,
    }
}
fn dispatch_error(error: &TerminalHostDispatchError) -> NativeTerminalBackgroundError {
    match error {
        TerminalHostDispatchError::Runtime(error) => runtime_error(*error),
        TerminalHostDispatchError::Resident(TerminalResidentError::Committed { .. }) => {
            NativeTerminalBackgroundError::Committed
        }
        TerminalHostDispatchError::Resident(TerminalResidentError::Registry(
            TerminalRegistryError::NotFound,
        )) => NativeTerminalBackgroundError::NotFound,
        TerminalHostDispatchError::Resident(TerminalResidentError::Cancelled)
        | TerminalHostDispatchError::Catalog(TerminalCatalogViewError::Cancelled) => {
            NativeTerminalBackgroundError::Cancelled
        }
        TerminalHostDispatchError::Invalid | TerminalHostDispatchError::CommandAction => {
            NativeTerminalBackgroundError::Invalid
        }
        _ => NativeTerminalBackgroundError::Uncertain,
    }
}

#[cfg(test)]
#[path = "terminal_host_background/tests.rs"]
mod tests;
