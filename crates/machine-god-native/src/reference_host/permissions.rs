//! Explicit authority and exact tool allocations for native permission composition.

use super::{
    AiGatewayTransport, AskUserQuestionTool, EngineLimits, NativeFileHistoryKind,
    NativeFileHistoryTool, NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
    PermissionPrompter, Tool, WorkspaceTools,
};
use crate::{
    AiGatewayPermissionReviewer, NativeFileApprovalAuthority, NativeFileApprovalRegistry,
    NativePermissionContexts, NativePermissionController, NativePermissionGovernedTool,
    NativePermissionReviewClock, NativePermissionTargetAuthority, NativePermissionTargetTool,
    NativeSandboxRoot, NativeTerminalPermissionPolicy, NativeToolPermissionPreparer,
};
use rustix::fd::OwnedFd;
use std::fs::File;
use std::{fmt, path::PathBuf, sync::Arc};

/// Explicit selected-file read and permission-review authority for this host.
/// Construction only retains the caller's allocations; no file is observed.
#[derive(Clone)]
pub struct NativeReferenceHostPermissionOptions {
    contexts: Arc<NativePermissionContexts>,
    clock: Arc<dyn NativePermissionReviewClock>,
    sandbox_executable: Option<Arc<File>>,
}
impl NativeReferenceHostPermissionOptions {
    #[must_use]
    pub fn new(
        contexts: Arc<NativePermissionContexts>,
        clock: Arc<dyn NativePermissionReviewClock>,
    ) -> Self {
        Self {
            contexts,
            clock,
            sandbox_executable: None,
        }
    }

    /// Supplies the retained system launcher; configuration alone cannot open it.
    #[must_use]
    pub fn with_sandbox_executable(mut self, executable: File) -> Self {
        self.sandbox_executable = Some(Arc::new(executable));
        self
    }
}
impl fmt::Debug for NativeReferenceHostPermissionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeReferenceHostPermissionOptions { .. }")
    }
}

pub(super) fn error() -> NativeReferenceHostBuildError {
    NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::PermissionConfig)
}

pub(super) struct PermissionComposition {
    root: File,
    workspace: String,
    files: Arc<NativeFileApprovalAuthority>,
    pub(super) registry: Arc<NativeFileApprovalRegistry>,
    pub(super) sandbox: Arc<NativeTerminalPermissionPolicy>,
    pub(super) contexts: Arc<NativePermissionContexts>,
    clock: Arc<dyn NativePermissionReviewClock>,
}
impl PermissionComposition {
    pub(super) fn install_files(&self, mut tools: WorkspaceTools) -> WorkspaceTools {
        tools.write_file = tools
            .write_file
            .with_file_approvals(Arc::clone(&self.registry));
        tools.edit_file = tools
            .edit_file
            .with_file_approvals(Arc::clone(&self.registry));
        tools.copy_file = tools
            .copy_file
            .with_file_approvals(Arc::clone(&self.registry));
        tools.rename_file = tools
            .rename_file
            .with_file_approvals(Arc::clone(&self.registry));
        tools.delete_file = tools
            .delete_file
            .with_file_approvals(Arc::clone(&self.registry));
        tools
    }
    pub(super) fn new(
        options: NativeReferenceHostPermissionOptions,
        tools: &WorkspaceTools,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let clone_root = || {
            tools
                .terminal_root
                .try_clone()
                .map(File::from)
                .map_err(|_| error())
        };
        let workspace = tools
            .canonical_workspace
            .to_str()
            .ok_or_else(error)?
            .to_owned();
        let root = NativeSandboxRoot::new(clone_root()?, tools.canonical_workspace.clone())
            .map_err(|_| error())?;
        let executable = options
            .sandbox_executable
            .as_ref()
            .map(|file| file.try_clone().map_err(|_| error()))
            .transpose()?;
        Ok(Self {
            root: clone_root()?,
            workspace,
            files: Arc::new(
                NativeFileApprovalAuthority::from_directory(clone_root()?).map_err(|_| error())?,
            ),
            registry: Arc::new(NativeFileApprovalRegistry::new()),
            sandbox: Arc::new(
                NativeTerminalPermissionPolicy::new(vec![root], executable).map_err(|_| error())?,
            ),
            contexts: options.contexts,
            clock: options.clock,
        })
    }

    pub(super) fn finish(
        self,
        registrations: Vec<NativePermissionTargetTool>,
        workers: crate::NativeOwnedWorkerScope,
        transport: Arc<dyn AiGatewayTransport>,
        prompter: Arc<dyn PermissionPrompter>,
    ) -> Result<Arc<NativePermissionController>, NativeReferenceHostBuildError> {
        let targets = Arc::new(
            NativePermissionTargetAuthority::new(self.root, self.workspace, registrations)
                .map_err(|_| error())?,
        );
        let reviewer = Arc::new(AiGatewayPermissionReviewer::new(transport, self.clock));
        let preparer = Arc::new(NativeToolPermissionPreparer::new(
            targets,
            self.files,
            self.registry,
            self.contexts,
            reviewer,
            workers,
        ));
        let controller = Arc::new(NativePermissionController::new(preparer.clone(), prompter));
        preparer.bind_controller(&controller).map_err(|_| error())?;
        self.sandbox
            .bind_controller(&controller)
            .map_err(|_| error())?;
        Ok(controller)
    }
}

/// Registers the exact underlying allocations before optional instrumentation.
pub(super) struct ReferenceHostToolCatalog {
    pub(super) tools: Vec<Arc<dyn Tool>>,
    pub(super) registrations: Vec<NativePermissionTargetTool>,
    observations: Option<Arc<crate::NativeConversationObservations>>,
    limits: EngineLimits,
    governed: bool,
}

pub(super) struct ReferenceHostWorkspaceAuthority {
    pub(super) vision_root: OwnedFd,
    pub(super) terminal_root: OwnedFd,
    pub(super) background_root: OwnedFd,
    pub(super) canonical_workspace: PathBuf,
}
impl ReferenceHostToolCatalog {
    pub(super) fn extensions(
        &mut self,
        catalog: Arc<dyn super::McpToolCatalog>,
        features: Arc<dyn super::McpFeatureAuthority>,
        subagents: Arc<dyn super::SubagentAuthority>,
    ) {
        self.add(
            super::McpSearchToolsTool::shared_catalog(Arc::clone(&catalog)),
            None,
        );
        self.add(super::McpSelectTool::shared_catalog(catalog), None);
        self.add(super::McpFeaturesTool::shared_authority(features), None);
        self.add(super::SubagentTool::shared_authority(subagents), None);
    }

    pub(super) fn workspace(&mut self, tools: WorkspaceTools) -> ReferenceHostWorkspaceAuthority {
        self.add(tools.copy_file, Some(NativeFileHistoryKind::Copy));
        self.add(tools.create_folder, None);
        self.add(tools.delete_file, Some(NativeFileHistoryKind::Delete));
        self.add(tools.edit_file, Some(NativeFileHistoryKind::Edit));
        self.add(tools.file_info, None);
        self.add(tools.glob_files, Some(NativeFileHistoryKind::Glob));
        self.grep(tools.grep_files);
        self.add(tools.install_skill, None);
        self.add(tools.list_files, Some(NativeFileHistoryKind::List));
        self.add(tools.open_file, None);
        self.add(tools.read_file, Some(NativeFileHistoryKind::Read));
        self.add(tools.rename_file, Some(NativeFileHistoryKind::Rename));
        self.add(tools.semantic_search, None);
        self.add(tools.skill, None);
        self.add(tools.write_file, Some(NativeFileHistoryKind::Write));
        ReferenceHostWorkspaceAuthority {
            vision_root: tools.vision_root,
            terminal_root: tools.terminal_root,
            background_root: tools.background_root,
            canonical_workspace: tools.canonical_workspace,
        }
    }
    pub(super) fn new(
        observations: Option<Arc<crate::NativeConversationObservations>>,
        limits: EngineLimits,
        governed: bool,
    ) -> Self {
        Self {
            tools: Vec::with_capacity(26),
            registrations: Vec::new(),
            observations,
            limits,
            governed,
        }
    }

    pub(super) fn add(&mut self, tool: impl Tool, history: Option<NativeFileHistoryKind>) {
        let tool: Arc<dyn Tool> = Arc::new(tool);
        if self.governed {
            self.registrations
                .push(NativePermissionTargetTool::Ordinary(Arc::clone(&tool)));
        }
        self.push(tool, history);
    }

    pub(super) fn question(&mut self, tool: AskUserQuestionTool) {
        let tool = Arc::new(tool);
        if self.governed {
            self.registrations
                .push(NativePermissionTargetTool::Question(Arc::clone(&tool)));
        }
        self.push(tool, None);
    }

    pub(super) fn grep(&mut self, tool: crate::GrepFilesTool) {
        let tool = Arc::new(tool);
        if self.governed {
            self.registrations
                .push(NativePermissionTargetTool::Grep(Arc::clone(&tool)));
        }
        self.push(tool, Some(NativeFileHistoryKind::Grep));
    }

    pub(super) fn terminal(
        &mut self,
        tool: Arc<dyn Tool>,
        concrete: Option<Arc<crate::TerminalActionTool>>,
    ) -> Result<(), NativeReferenceHostBuildError> {
        if self.governed {
            self.registrations
                .push(NativePermissionTargetTool::Terminal(
                    concrete.ok_or_else(error)?,
                ));
        }
        self.push(tool, None);
        Ok(())
    }

    fn push(&mut self, mut tool: Arc<dyn Tool>, history: Option<NativeFileHistoryKind>) {
        if let (Some(registry), Some(kind)) = (&self.observations, history) {
            tool = Arc::new(NativeFileHistoryTool::shared(
                tool,
                kind,
                Arc::clone(registry),
            ));
        }
        if self.governed {
            tool = Arc::new(NativePermissionGovernedTool::new(tool, self.limits));
        }
        self.tools.push(tool);
    }
}
