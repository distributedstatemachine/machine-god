//! Concrete shared-engine child construction; no parent runtime ownership edge.
mod history;
mod preparation;
mod resources;
mod staged;
pub(super) use staged::StagedParentMcp;
#[cfg(test)]
mod tests;

use super::{mcp::ManagedMcpInstance, services::NativeHostServices};
use crate::managed::{
    manager::factory::{
        ManagedPreparation, ManagedRuntimeError, ManagedRuntimeFactory,
        ManagedRuntimePreparationKind, ManagedRuntimeRequest, PreparedManagedRuntime,
    },
    mcp::NativePrincipalMcpRegistry,
    notices::{ManagedNotices, NoticePrincipal},
    principal::NativePrincipalRegistry,
    prompt_context::ParentNoticeContext,
    scheduler::ManagedScheduler,
    store::{JournalOwner, JournalTranscript},
};
use crate::{
    NativeConversation, NativeConversationRuntime, NativeModelPreferences,
    NativePermissionPolicySnapshot, NativeReasoningEffort, NativeSessionMetadata,
    NativeSessionOrigin, NativeWorkspaceAuthority, NativeWorkspaceContexts,
    NativeWorkspaceScopeSnapshot, PermissionMode,
};
use machine_god_core::{BoxFuture, CancellationToken, ManagedPermissionMode, Session, ToolName};
use std::{fmt, num::NonZeroU64, sync::Arc, time::Duration};

pub(super) struct ManagedRestorationAuthority {
    pub workspace: NativeWorkspaceScopeSnapshot,
    pub policy: NativePermissionPolicySnapshot,
    pub preferences: NativeModelPreferences,
}
pub(super) struct SharedManagedRuntimeFactoryOptions {
    pub services: Arc<NativeHostServices>,
    pub skills: Option<Arc<crate::NativeSkillCatalog>>,
    pub principals: Arc<NativePrincipalRegistry>,
    pub scheduler: ManagedScheduler,
    pub mcp: Arc<NativePrincipalMcpRegistry>,
    pub notices: Arc<ManagedNotices>,
    pub workspace_contexts: Arc<NativeWorkspaceContexts>,
    pub restoration: ManagedRestorationAuthority,
    pub reserved_tool_names: Vec<ToolName>,
    pub origin: NativeSessionOrigin,
    pub cleanup_timeout: Duration,
    pub prompts: Option<crate::interactive_prompts::NativeInteractivePromptRegistrar>,
}
pub(super) struct SharedManagedRuntimeFactory(Arc<SharedManagedRuntimeFactoryOptions>);
struct RuntimeSelection {
    principal: NoticePrincipal,
    authority: ManagedRestorationAuthority,
    parent_mcp: Option<Arc<resources::McpLifetime>>,
}
impl SharedManagedRuntimeFactory {
    pub(super) fn manages_prompt_inbox(
        &self,
        inbox: &crate::NativeInteractivePromptInbox,
    ) -> Result<bool, super::NativeManagedAgentsError> {
        match &self.0.prompts {
            None => Ok(false),
            Some(registrar) if registrar.matches(inbox) => Ok(true),
            Some(_) => Err(super::NativeManagedAgentsError::Configuration),
        }
    }

    pub(super) fn belongs_to(&self, services: &Arc<NativeHostServices>) -> bool {
        Arc::ptr_eq(&self.0.services, services)
    }

    pub(super) fn new(
        options: SharedManagedRuntimeFactoryOptions,
    ) -> Result<Self, ManagedRuntimeError> {
        let services = &options.services;
        if services.control_workers.is_none()
            || services.permissions.is_none()
            || services.permission_contexts.is_none()
            || services.permission_preparation.is_none()
            || services.managed_mcp_seed.is_none()
            || services.model_routes.is_none()
            || services.observations.is_none()
            || options.cleanup_timeout.is_zero()
            || options.cleanup_timeout > Duration::from_secs(300)
            || options.reserved_tool_names.len() > 4096
        {
            return Err(ManagedRuntimeError::Invalid);
        }
        let mut names = std::collections::BTreeSet::new();
        if options
            .reserved_tool_names
            .iter()
            .any(|name| !names.insert(name.as_str()))
        {
            return Err(ManagedRuntimeError::Invalid);
        }
        Ok(Self(Arc::new(options)))
    }

    pub(super) fn prepare_parent(
        &self,
        conversation: NativeConversation,
        authority: ManagedRestorationAuthority,
        principal: NoticePrincipal,
        journal_owner: JournalOwner,
        mcp: Arc<super::mcp::ManagedParentMcpSeed>,
    ) -> BoxFuture<'static, Result<PreparedManagedRuntime, ManagedRuntimeError>> {
        let factory = Arc::downgrade(&self.0);
        Box::pin(async move {
            let factory = factory.upgrade().ok_or(ManagedRuntimeError::Unavailable)?;
            let cohort = preparation::begin(&factory, &journal_owner)?;
            let completion = cohort.completion();
            let prepared_completion = completion.clone();
            let result = preparation::Attributed::new(
                cohort,
                Box::pin(async move {
                    let mcp = mcp
                        .compose(
                            factory
                                .services
                                .control_workers
                                .as_ref()
                                .ok_or(ManagedRuntimeError::Invalid)?,
                            &factory.reserved_tool_names,
                            factory
                                .services
                                .permission_preparation
                                .as_ref()
                                .ok_or(ManagedRuntimeError::Invalid)?,
                        )
                        .map_err(|_| ManagedRuntimeError::Invalid)?;
                    factory.compose_selected(
                        conversation,
                        &journal_owner,
                        RuntimeSelection {
                            principal,
                            authority,
                            parent_mcp: Some(Arc::new(resources::McpLifetime::new(mcp, None))),
                        },
                        prepared_completion,
                    )
                }),
            )
            .await;
            completion.wait().await;
            result
        })
    }
}
impl ManagedRuntimeFactory for SharedManagedRuntimeFactory {
    fn allocate_identity(
        &self,
    ) -> BoxFuture<'static, Result<JournalTranscript, ManagedRuntimeError>> {
        let future = self.0.services.session_lifecycle.allocate_identity();
        Box::pin(async move {
            let (session_id, incarnation_id) =
                future.await.map_err(|_| ManagedRuntimeError::Unavailable)?;
            Ok(JournalTranscript {
                session_id,
                incarnation: incarnation_id,
            })
        })
    }
    fn prepare(
        &self,
        request: ManagedRuntimeRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<ManagedPreparation, ManagedRuntimeError>> {
        preparation::prepare(Arc::downgrade(&self.0), request, cancellation)
    }
}

impl SharedManagedRuntimeFactoryOptions {
    fn selection(
        &self,
        request: &ManagedRuntimeRequest,
    ) -> Result<ManagedRestorationAuthority, ManagedRuntimeError> {
        if request.generation == 0
            || request.child_id.is_empty()
            || request.child_id.len() > 255
            || !request
                .child_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || (request.kind == ManagedRuntimePreparationKind::Create && request.origin.is_none())
        {
            return Err(ManagedRuntimeError::Invalid);
        }
        let (workspace, policy, mut preferences) = request.origin.as_ref().map_or_else(
            || {
                (
                    self.restoration.workspace.clone(),
                    self.restoration.policy.clone(),
                    self.restoration.preferences.clone(),
                )
            },
            |origin| {
                (
                    origin.workspace.clone(),
                    origin.policy.clone(),
                    origin.preferences.clone(),
                )
            },
        );
        let mode = match request.configuration.permission_mode {
            ManagedPermissionMode::Ask => PermissionMode::Ask,
            ManagedPermissionMode::Auto => PermissionMode::Auto,
            ManagedPermissionMode::Yolo => PermissionMode::Yolo,
        };
        if permission_rank(mode) > permission_rank(policy.mode()) {
            return Err(ManagedRuntimeError::Invalid);
        }
        let policy =
            NativePermissionPolicySnapshot::new(mode, Arc::new(policy.configured_rules().clone()))
                .with_sandbox_mode(policy.sandbox_mode());
        if let Some(model) = &request.configuration.model {
            preferences
                .set_model(model)
                .map_err(|_| ManagedRuntimeError::Invalid)?;
        }
        if let Some(effort) = &request.configuration.effort {
            preferences.set_effort(
                NativeReasoningEffort::parse(effort).map_err(|_| ManagedRuntimeError::Invalid)?,
            );
        }
        Ok(ManagedRestorationAuthority {
            workspace,
            policy,
            preferences,
        })
    }
    fn conversation(
        &self,
        mut conversation: NativeConversation,
        policy: NativePermissionPolicySnapshot,
    ) -> Result<NativeConversation, ManagedRuntimeError> {
        conversation = conversation
            .with_permission_controller(
                self.services
                    .permissions
                    .as_ref()
                    .ok_or(ManagedRuntimeError::Invalid)?,
                policy,
            )
            .map_err(|_| ManagedRuntimeError::Capacity)?;
        conversation = conversation
            .with_permission_contexts(
                self.services
                    .permission_contexts
                    .as_ref()
                    .ok_or(ManagedRuntimeError::Invalid)?,
            )
            .map_err(|_| ManagedRuntimeError::Capacity)?;
        conversation = conversation
            .with_observations(
                self.services
                    .observations
                    .as_ref()
                    .ok_or(ManagedRuntimeError::Invalid)?,
            )
            .map_err(|_| ManagedRuntimeError::Capacity)?;
        Ok(conversation)
    }

    fn compose(
        &self,
        request: &ManagedRuntimeRequest,
        session: Session,
        preparation: crate::NativeOwnedWorkerCompletion,
    ) -> Result<PreparedManagedRuntime, ManagedRuntimeError> {
        if session.id() != request.transcript.session_id
            || session.incarnation_id() != request.transcript.incarnation
            || session.has_active_turn()
        {
            return Err(ManagedRuntimeError::Invalid);
        }
        let selected = self.selection(request)?;
        self.compose_selected(
            NativeConversation::from_session(session).map_err(|_| ManagedRuntimeError::Invalid)?,
            &request.journal_owner,
            RuntimeSelection {
                principal: NoticePrincipal {
                    id: request.child_id.clone(),
                    generation: NonZeroU64::new(request.generation)
                        .ok_or(ManagedRuntimeError::Invalid)?,
                },
                authority: selected,
                parent_mcp: None,
            },
            preparation,
        )
    }

    fn compose_selected(
        &self,
        conversation: NativeConversation,
        journal_owner: &JournalOwner,
        selection: RuntimeSelection,
        preparation: crate::NativeOwnedWorkerCompletion,
    ) -> Result<PreparedManagedRuntime, ManagedRuntimeError> {
        let RuntimeSelection {
            principal,
            authority: selected,
            parent_mcp,
        } = selection;
        let foreground = parent_mcp.is_some();
        if conversation.core_session().has_active_turn()
            || principal.id.is_empty()
            || principal.id.len() > 255
            || !principal
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(ManagedRuntimeError::Invalid);
        }
        let workspace = NativeWorkspaceAuthority::from_admitted_scope(selected.workspace);
        let generation = principal.generation.get();
        let notice_context = Arc::new(ParentNoticeContext::new(
            conversation.core_session(),
            principal,
            &self.notices,
        ));
        let conversation = if foreground {
            conversation
                .with_staged_routes()
                .map_err(|_| ManagedRuntimeError::Invalid)?
        } else {
            conversation
        };
        let conversation = self.conversation(conversation, selected.policy)?;
        let (conversation, owner) = conversation
            .with_managed_execution(
                &self.principals,
                self.scheduler.clone(),
                generation,
                &workspace,
                &self.workspace_contexts,
            )
            .map_err(|_| ManagedRuntimeError::Capacity)?;
        let workers = self
            .services
            .control_workers
            .as_ref()
            .ok_or(ManagedRuntimeError::Invalid)?;
        owner
            .configure_workers(workers.clone(), journal_owner.clone())
            .map_err(|_| ManagedRuntimeError::Unavailable)?;
        let mcp = match parent_mcp {
            Some(mcp) => mcp,
            None => Arc::new(resources::McpLifetime::new(
                self.compose_mcp(workers)?,
                None,
            )),
        };
        let mcp_owner = self
            .mcp
            .register(
                owner.principal(),
                &mcp.instance.runtime,
                None,
                Some(&mcp.instance.permissions),
            )
            .map_err(|_| ManagedRuntimeError::Capacity)?;
        owner
            .configure_mcp(&mcp_owner)
            .map_err(|_| ManagedRuntimeError::Invalid)?;
        mcp.bind(mcp_owner)?;
        let conversation = bind_mcp(conversation, &mcp.instance)?
            .with_notice_context(&notice_context)
            .map_err(|_| ManagedRuntimeError::Invalid)?;
        let runtime = self.runtime(conversation, selected.preferences, foreground)?;
        let prompt = self.register_prompt(&runtime, foreground)?;
        let resources = resources::Resources::new(
            owner.binding(),
            mcp,
            preparation,
            prompt,
            resources::CloseAuthority {
                workers: workers.clone(),
                journal_owner: journal_owner.clone(),
                timeout: self.cleanup_timeout,
            },
        );
        Ok(PreparedManagedRuntime {
            runtime: Arc::new(runtime),
            owner,
            resources: Box::new(resources),
            notice_context: Some(notice_context),
            skills: self.skills.as_ref().map(|catalog| {
                crate::managed::manager::factory::ManagedRuntimeSkills {
                    catalog: catalog.clone(),
                    workers: workers.clone(),
                }
            }),
        })
    }

    fn register_prompt(
        &self,
        runtime: &NativeConversationRuntime,
        foreground: bool,
    ) -> Result<Option<resources::Prompt>, ManagedRuntimeError> {
        self.prompts
            .as_ref()
            .map(|prompts| {
                let owner = machine_god_core::BackgroundOutputOwner::new(
                    runtime.id(),
                    runtime.incarnation_id(),
                );
                if foreground {
                    prompts.reserve(owner).map(resources::Prompt::Reserved)
                } else {
                    prompts.register(owner).map(resources::Prompt::Active)
                }
                .map_err(prompt_error)
            })
            .transpose()
    }

    fn compose_mcp(
        &self,
        workers: &crate::NativeOwnedWorkerScope,
    ) -> Result<ManagedMcpInstance, ManagedRuntimeError> {
        self.services
            .managed_mcp_seed
            .as_ref()
            .ok_or(ManagedRuntimeError::Invalid)?
            .compose(
                workers,
                &self.reserved_tool_names,
                self.services
                    .permission_preparation
                    .as_ref()
                    .ok_or(ManagedRuntimeError::Invalid)?,
            )
            .map_err(|_| ManagedRuntimeError::Invalid)
    }

    fn runtime(
        &self,
        conversation: NativeConversation,
        preferences: NativeModelPreferences,
        foreground: bool,
    ) -> Result<NativeConversationRuntime, ManagedRuntimeError> {
        let runtime = NativeConversationRuntime::new_with_model_routes(
            conversation,
            preferences.clone(),
            None,
            self.services
                .model_routes
                .as_ref()
                .ok_or(ManagedRuntimeError::Invalid)?,
        )
        .map_err(|_| ManagedRuntimeError::Capacity)?;
        // A child keeps its captured work selection; a foreground honors saved metadata.
        if !foreground {
            runtime
                .set_model_preferences(preferences)
                .map_err(|_| ManagedRuntimeError::Invalid)?;
        }
        Ok(runtime)
    }
}

fn prompt_error(error: crate::NativeInteractivePromptError) -> ManagedRuntimeError {
    match error {
        crate::NativeInteractivePromptError::Busy
        | crate::NativeInteractivePromptError::Limit
        | crate::NativeInteractivePromptError::Exhausted => ManagedRuntimeError::Capacity,
        _ => ManagedRuntimeError::Unavailable,
    }
}
fn bind_mcp(
    mut conversation: NativeConversation,
    mcp: &ManagedMcpInstance,
) -> Result<NativeConversation, ManagedRuntimeError> {
    conversation = conversation
        .with_mcp_contexts(&mcp.contexts)
        .map_err(|_| ManagedRuntimeError::Capacity)?;
    if let Some(controller) = &mcp.controller {
        conversation = conversation
            .with_managed_mcp_readiness(controller)
            .map_err(|_| ManagedRuntimeError::Invalid)?;
    }
    if let Some(ephemeral) = &mcp.ephemeral {
        conversation = conversation
            .with_mcp_ephemeral_readiness(ephemeral)
            .map_err(|_| ManagedRuntimeError::Invalid)?;
    }
    Ok(conversation)
}
fn permission_rank(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Ask => 0,
        PermissionMode::Auto => 1,
        PermissionMode::Yolo => 2,
    }
}
impl fmt::Debug for SharedManagedRuntimeFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedManagedRuntimeFactory(..)")
    }
}
