//! Pre-engine managed routes; actual queue ownership stays outside shared services.
pub(super) mod relationship;

use super::{NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind};
use crate::managed::{
    mailbox::{MailboxLimits, ManagedMailbox},
    mcp::NativePrincipalMcpRegistry,
    notices::{ManagedNotices, NoticeLimits},
    principal::NativePrincipalRegistry,
    scheduler::{ManagedScheduler, SchedulerLimits},
};
use crate::mcp::runtime::NativeMcpRuntimeClock;
use crate::{NativeToolResultArchiveAdapter, NativeUndoBudget};
use std::{fmt, sync::Arc};

/// Explicit native clock selection for managed tools and their outer driver.
/// Construction opens no journal, starts no worker and does not execute tools.
#[derive(Clone)]
pub struct NativeReferenceHostManagedOptions {
    pub(super) clock: Arc<dyn NativeMcpRuntimeClock>,
}

impl NativeReferenceHostManagedOptions {
    #[must_use]
    pub fn new(clock: Arc<dyn NativeMcpRuntimeClock>) -> Self {
        Self { clock }
    }
}

impl fmt::Debug for NativeReferenceHostManagedOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeReferenceHostManagedOptions")
            .finish_non_exhaustive()
    }
}

pub(super) struct Selection {
    pub options: NativeReferenceHostManagedOptions,
    pub budget: Arc<NativeUndoBudget>,
}

/// Never put this owner in `NativeHostServices`: its mailbox routes are retained
/// weakly by the shared engine, and the eventual manager owns the actual queue.
pub(super) struct ManagedHostAssembly {
    pub principals: Arc<NativePrincipalRegistry>,
    pub scheduler: ManagedScheduler,
    pub mcp: Arc<NativePrincipalMcpRegistry>,
    pub notices: Arc<ManagedNotices>,
    pub mailbox: Option<ManagedMailbox>,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    pub relationships: Arc<dyn crate::managed::manager::factory::ManagedRelationshipAuthorizer>,
}

impl ManagedHostAssembly {
    pub(super) fn authority(
        &self,
    ) -> Result<Arc<dyn machine_god_core::ManagedSubagentAuthority>, NativeReferenceHostBuildError>
    {
        Ok(Arc::new(
            self.mailbox.as_ref().ok_or_else(error)?.requester(),
        ))
    }

    pub(super) fn new(
        selection: Selection,
        archive: Arc<NativeToolResultArchiveAdapter>,
        prompter: Arc<dyn crate::PermissionPrompter>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let principals =
            Arc::new(NativePrincipalRegistry::new(64, selection.budget).map_err(|_| error())?);
        let mailbox = ManagedMailbox::new(principals.requester(), MailboxLimits::default())
            .map_err(|_| error())?;
        let mcp = Arc::new(
            NativePrincipalMcpRegistry::new(64, principals.requester(), archive)
                .map_err(|_| error())?,
        );
        let clock = selection.options.clock;
        let notices = Arc::new(
            ManagedNotices::new(NoticeLimits::default(), clock.clone()).map_err(|_| error())?,
        );
        let relationships = Arc::new(relationship::RelationshipConsent::new(
            principals.requester(),
            prompter,
        ));
        Ok(Self {
            principals,
            scheduler: ManagedScheduler::new(SchedulerLimits::default()),
            mcp,
            notices,
            mailbox: Some(mailbox),
            clock,
            relationships,
        })
    }
}

pub(super) fn validate(
    options: &super::PreparedCompositionOptions,
) -> Result<(), NativeReferenceHostBuildError> {
    if options.managed.is_some()
        && (options.terminal.is_none()
            || options.permissions.is_none()
            || options.workspace_binding.is_none()
            || options.model_routes.is_none()
            || options.observations.is_none()
            || options.undo_tracker.is_none())
    {
        return Err(error());
    }
    Ok(())
}

pub(super) fn select(
    options: &super::PreparedCompositionOptions,
) -> Result<Option<Selection>, NativeReferenceHostBuildError> {
    options
        .managed
        .as_ref()
        .map(|selected| {
            Ok(Selection {
                options: selected.clone(),
                budget: options
                    .undo_tracker
                    .as_ref()
                    .ok_or_else(error)?
                    .shared_budget(),
            })
        })
        .transpose()
}

pub(super) fn error() -> NativeReferenceHostBuildError {
    NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::ManagedConfig)
}
