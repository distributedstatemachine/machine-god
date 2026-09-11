#![doc = "Explicit native capabilities for machine-god hosts."]

mod native_environment;
pub use native_environment::{
    ConfigFileState, NativeEnvironment, NativeSandboxMode, NativeStatus, PermissionMode,
    StateDirectoryState, inspect_native_status, inspect_process_status,
};

mod ai_gateway;
#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
mod ai_gateway_credential;
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
mod ai_gateway_http;
#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
mod ai_gateway_http_shared;
mod ai_gateway_model_catalog;
#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
mod ai_gateway_model_catalog_http;
#[cfg(all(feature = "vision", not(target_family = "wasm")))]
mod ai_gateway_vision;
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
mod ai_gateway_web_search;
mod ask_permission;
mod ask_user_question;
mod background_commands;
#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub use background_commands::service::{
    NativeBackgroundControlError, NativeBackgroundControlReceipt, NativeBackgroundLogSummary,
    NativeBackgroundLogWindow,
};
pub use background_commands::{
    MAX_NATIVE_BACKGROUND_COMMAND_BYTES, NativeBackgroundCommand, NativeBackgroundCommandError,
    NativeBackgroundTarget,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod background_control;
mod background_history;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod background_input;
mod background_inspection;
pub use background_history::{
    MAX_BACKGROUND_HISTORY_RECORDS, NativeBackgroundHistoryDetail, NativeBackgroundHistoryId,
    NativeBackgroundHistoryInspection, NativeBackgroundHistoryList, NativeBackgroundHistoryQuery,
    NativeBackgroundHistorySummary, NativeBackgroundTerminalDetail,
    inspect_native_background_history, inspect_process_background_history,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod background_output;
mod background_process;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod background_store;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod background_supervisor;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
mod background_terminal_inspection;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
mod background_url_opener;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
pub use background_url_opener::{
    NativeBackgroundOpenError, NativeBackgroundOpenOutcome, NativeBackgroundUrlExecutable,
    NativeBackgroundUrlOpener,
};
mod model_catalog_cache;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod owned_worker;
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use owned_worker::NativeOwnedWorkerScopeIdentity;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use owned_worker::{
    NativeOwnedWorkerCleanup, NativeOwnedWorkerCompletion, NativeOwnedWorkerScope,
    NativeOwnedWorkerSpawnError, NativeOwnedWorkerSpawner,
};
#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
mod allowlist;
#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub use allowlist::{
    MAX_NATIVE_ALLOWLIST_REQUEST_BYTES, NativeAllowlistCommand, NativeAllowlistError,
    NativeAllowlistParseError, NativeAllowlistReceipt, NativeAllowlistReloadError,
    NativeAllowlistRequest, NativeAllowlistSources, NativeAllowlistView,
};
mod config;
mod permission_inspection;
pub use permission_inspection::{
    NativePermissionInspection, NativePermissionInspectionError, NativePermissionInspectionRule,
    inspect_native_permissions, inspect_process_permissions,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod user_config_store;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use user_config_store::{
    NativeUserConfigError, NativeUserConfigSnapshot, NativeUserConfigStore,
    NativeUserPermissionCommit, NativeUserWorkspaceCommit, NativeWorkspaceCommitDurability,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation_context;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace_authority;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use workspace_authority::{
    NativeWorkspaceAuthority, NativeWorkspaceAuthorityError, NativeWorkspaceEntry,
    NativeWorkspaceEntrySpec, NativeWorkspacePreparedInstall, NativeWorkspaceRoute,
    NativeWorkspaceScopeSnapshot, NativeWorkspaceSource,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace_context;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use workspace_context::{
    MAX_NATIVE_WORKSPACE_CONTEXT_SESSIONS, NativeWorkspaceContextError, NativeWorkspaceContexts,
    NativeWorkspaceTurnScope,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace_mutation;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace_path_tools;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace_service;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use workspace_service::{
    NativeWorkspaceAction, NativeWorkspaceReceipt, NativeWorkspaceReconciliation,
    NativeWorkspaceService, NativeWorkspaceServiceError,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace_startup;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use workspace_startup::{
    MAX_WORKSPACE_LAUNCH_ARGUMENTS, prepare_native_workspace,
    prepare_native_workspace_without_settings,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation_history;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use conversation_history::{
    NATIVE_CONVERSATION_HISTORY_KEY, NativeConversationHistory, NativeConversationHistoryError,
    NativeHistoryBackground, NativeHistoryFileAction, NativeHistoryFileEvidence,
    NativeHistoryFileSource, NativeHistoryFileStatus, NativeHistoryGroup, NativeHistoryState,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation_model_routes;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod conversation_observation_tests;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation_observations;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod file_history_tool;
mod permission_controller;
mod permission_tool;
pub use permission_tool::NativePermissionGovernedTool;
mod permission_rules;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use conversation_observations::{NativeConversationObservations, NativeObservationError};
pub use permission_rules::{
    MAX_NATIVE_PERMISSION_IDENTITY_BYTES, MAX_NATIVE_PERMISSION_RULES,
    NATIVE_SESSION_PERMISSION_RULES_KEY, NativePermissionRule, NativePermissionRuleDecision,
    NativePermissionRuleError, NativePermissionRuleKey, NativePermissionRuleKind,
    NativeSessionPermissionRules,
};
mod conversation_lifecycle;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation_runtime;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod interactive_input;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod interactive_terminal;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod interactive_terminal_size;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use interactive_input::{
    INTERACTIVE_INPUT_HELPER_ARGUMENT, NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES,
    NativeInteractiveInput, NativeInteractiveInputChunk, NativeInteractiveInputError,
    NativeInteractiveInputHelper, NativeInteractiveInputSource, run_interactive_input_helper,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use interactive_terminal::{
    NativeInteractiveTerminal, NativeInteractiveTerminalError,
    NativeInteractiveTerminalRestoreReceipt,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use interactive_terminal_size::{
    NativeInteractiveTerminalDimensions, NativeInteractiveTerminalSizeError,
    NativeInteractiveTerminalSizeReader,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod clipboard;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use clipboard::{NativeClipboard, NativeClipboardError, NativeClipboardExecutable};
mod clipboard_reply;
pub use clipboard_reply::{
    NativeClipboardReplyError, NativeClipboardReplySelection, NativeClipboardReplyStep,
};
mod interactive_prompts;
pub use interactive_prompts::{
    MAX_NATIVE_INTERACTIVE_PROMPT_PAYLOAD_BYTES, MAX_NATIVE_INTERACTIVE_PROMPTS,
    NativeInteractivePromptBridge, NativeInteractivePromptError, NativeInteractivePromptInbox,
    NativeInteractivePromptLimits, NativeInteractivePromptResponse, NativeInteractivePromptScope,
    NativeInteractivePromptToken, NativeInteractivePromptView,
};
mod copy_file;
mod create_folder;
mod delete_file;
mod doctor;
mod edit_file;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod file_approval;
mod file_info;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod os_sandbox;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod terminal_permission_policy;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use file_approval::{
    MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES, MAX_NATIVE_FILE_APPROVAL_RETAINED_BYTES,
    MAX_NATIVE_FILE_APPROVALS, NativeFileApprovalAdmission, NativeFileApprovalAuthority,
    NativeFileApprovalError, NativeFileApprovalKind, NativeFileApprovalPolicy,
    NativeFileApprovalPreimage, NativeFileApprovalRegistry, PreparedFileApproval,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use os_sandbox::{
    MAX_NATIVE_SANDBOX_PROFILE_BYTES, MAX_NATIVE_SANDBOX_ROOT_PATH_BYTES, MAX_NATIVE_SANDBOX_ROOTS,
    NATIVE_SANDBOX_EXECUTABLE, NativeSandboxError, NativeSandboxLaunch, NativeSandboxRoot,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use terminal_permission_policy::NativeTerminalPermissionPolicy;
mod file_undo;
mod glob_files;
mod grep_files;
mod install_skill;
mod list_files;
#[cfg(target_os = "macos")]
mod macos_directory;
mod mcp_features;
mod mcp_search_tools;
mod mcp_select_tool;
mod memory;
mod model_preferences;
mod model_selection;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod native_tool_result_archive;
mod open_file;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod permission_context;
mod permission_patterns;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod permission_preparer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use permission_preparer::NativeToolPermissionPreparer;
mod permission_reviewer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod permission_targets;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use permission_context::{
    MAX_NATIVE_PERMISSION_CONTEXT_SESSIONS, MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES,
    NATIVE_PERMISSION_CONTEXT_KEY, NativePermissionContextError, NativePermissionContexts,
    NativePermissionReviewContext,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use permission_targets::{
    NativePermissionOwnedTarget, NativePermissionTargetAuthority, NativePermissionTargetTool,
    NativePermissionTerminalResolution, NativePermissionTerminalResolver,
    NativePreparedPermissionTargets,
};
#[cfg(target_os = "macos")]
mod process_inventory_helper;
#[cfg(target_os = "macos")]
mod process_inventory_protocol;
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use process_inventory_helper::{
    PROCESS_INVENTORY_HELPER_ARGUMENT, run_process_inventory_helper,
};
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use process_inventory_protocol::{
    PROCESS_INVENTORY_SERVICE_ARGUMENT, run_process_inventory_service,
};
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
mod interactive_session;
mod read_file;
mod read_tool_result;
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
mod reference_host;
#[cfg(target_os = "macos")]
mod retained_root;
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
pub use interactive_session::{
    NativeInteractiveControl, NativeInteractiveControlError, NativeInteractiveControlId,
    NativeInteractiveControlOutcome, NativeInteractiveControlReceipt, NativeInteractiveCopyError,
    NativeInteractiveCopyId, NativeInteractiveCopyOutcome, NativeInteractiveCopyReceipt,
    NativeInteractiveError, NativeInteractiveInitialSession, NativeInteractiveOutcome,
    NativeInteractiveRequestId, NativeInteractiveRequestReceipt, NativeInteractiveSession,
    NativeInteractiveSessionOptions, NativeInteractiveTransition,
    NativeInteractiveTransitionReceipt,
};
mod rename_file;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod root_selection;
mod runtime_status;
mod semantic_search;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_catalog;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_catalog_reader;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_catalog_reader::{
    NativeSessionCatalogReadError, NativeSessionCatalogReader, NativeSessionCatalogScope,
};
mod session_catalog_cursor;
mod session_inspection;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_lifecycle;
mod session_listing;
mod session_maintenance;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_maintenance::NativeSessionMaintenance;
pub use session_maintenance::{
    NativeSessionCleanupMode, NativeSessionCleanupReport, NativeSessionCleanupStatus,
    NativeSessionMaintenanceError, NativeSessionMaintenanceReceipt,
    NativeSessionMaintenanceRequest, NativeSessionMigration, NativeSessionRecovery,
    execute_process_session_maintenance,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_metadata;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_metadata_commands;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_resume;
mod session_store;
mod skill;
mod skills_catalog;
mod skills_commands;
mod skills_invocation;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod skills_managed;
mod skills_metadata;
mod skills_picker;
mod skills_prompt_context;
mod skills_roots;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod skills_service;
pub use skills_catalog::{
    MAX_NATIVE_SKILL_CANDIDATES, MAX_NATIVE_SKILL_DIAGNOSTICS, MAX_NATIVE_SKILL_DIRECTORY_BYTES,
    MAX_NATIVE_SKILL_DISCOVERY_BYTES, MAX_NATIVE_SKILL_IO_ATTEMPTS, MAX_NATIVE_SKILL_LINK_HOPS,
    MAX_NATIVE_SKILL_MATERIALIZED_BYTES, MAX_NATIVE_SKILL_PATH_BYTES,
    MAX_NATIVE_SKILL_PATH_COMPONENTS, MAX_NATIVE_SKILL_QUERY_BYTES, MAX_NATIVE_SKILL_QUERY_ROWS,
    MAX_NATIVE_SKILL_ROOTS, MAX_NATIVE_SKILL_SNAPSHOT_BYTES, MAX_NATIVE_SKILL_VISITED_ENTRIES,
    NativeSkillCatalog, NativeSkillCatalogError, NativeSkillDiagnostic, NativeSkillEntry,
    NativeSkillLinkPolicy, NativeSkillMaterialized, NativeSkillRoot, NativeSkillSelection,
    NativeSkillSnapshot, NativeSkillSource,
};
pub use skills_commands::{
    MAX_NATIVE_SKILLS_COMMAND_BYTES, MAX_NATIVE_SKILLS_SELECTOR_BYTES, NativeSkillsCommand,
    NativeSkillsCommandError,
};
pub use skills_invocation::{
    MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES, MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES,
    MAX_NATIVE_SKILL_INVOCATION_SELECTIONS, NativeSkillInvocationError, NativeSkillInvocationPlan,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use skills_managed::{
    MAX_MANAGED_SKILL_ENTRIES, MAX_MANAGED_SKILL_FILE_BYTES, MAX_MANAGED_SKILL_ITEMS,
    MAX_MANAGED_SKILL_OPERATIONS, MAX_MANAGED_SKILL_TOTAL_BYTES, NativeManagedSkills,
    NativeSkillBatchReceipt, NativeSkillDestinationRevision, NativeSkillGitLease,
    NativeSkillGitRequest, NativeSkillGitRunner, NativeSkillInstallItem, NativeSkillInstallPlan,
    NativeSkillInstallSource, NativeSkillItemOutcome, NativeSkillItemReceipt,
    NativeSkillManagedError, NativeSkillManagedErrorKind, NativeSkillReplacementConsent,
    NativeSkillSourceKind, parse_skill_create_command, parse_skill_install_command,
};
pub use skills_metadata::{
    MAX_NATIVE_SKILL_DESCRIPTION_BYTES, MAX_NATIVE_SKILL_HEADER_BYTES,
    MAX_NATIVE_SKILL_METADATA_NAME_BYTES, NativeSkillMetadata, NativeSkillMetadataError,
};
pub use skills_picker::{
    NativeSkillBinding, NativeSkillDraftIdentity, NativeSkillFrameIdentity, NativeSkillInlineQuery,
    NativeSkillPicker, NativeSkillPickerError, NativeSkillPickerInsertion, NativeSkillPickerMode,
    NativeSkillPickerView,
};
pub use skills_prompt_context::{
    NATIVE_SKILL_PROMPT_CONTEXT_KEY, NativeSkillPromptContext, NativeSkillPromptContextError,
};
pub use skills_roots::{
    MAX_NATIVE_SKILL_ROOT_IO_ATTEMPTS, MAX_NATIVE_SKILL_WORKSPACE_LEVELS,
    NativeSkillDirectoryAuthority, NativeSkillRootsError, compose_native_skill_catalog,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use skills_service::{
    NativeSkillsCatalogView, NativeSkillsNotice, NativeSkillsService, NativeSkillsServiceError,
    NativeSkillsServiceResult,
};
mod slash_commands;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod state_environment;
mod terminal;
mod terminal_action_parse;
mod terminal_action_tool;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod terminal_captured_exec;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_monitor;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_probe_effects;
// Explicit module declarations keep the native component graph visible to rustfmt.
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_catalog;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_catalog_view;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_history;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_host;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_host_authority;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_host_catalog;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_host_dispatch;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_host_probes;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_input;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_journal;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_native_backend;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_native_launch;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_owner;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_profile;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_profile_store;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_pty;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_registry;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_resident_dispatch;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_runtime;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_session;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_session_record;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_staged_start;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_startup;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_tmux;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_tmux_startup;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_wait;
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
#[cfg_attr(
    test,
    allow(dead_code, reason = "unit tests exercise private component seams")
)]
mod terminal_write_completion;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use terminal_captured_exec::{
    TERMINAL_CAPTURED_HELPER_ARGUMENT, TerminalCapturedExec, TerminalCapturedExecError,
    run_terminal_captured_helper,
};
#[cfg(all(
    any(test, feature = "ai-gateway-http"),
    any(target_os = "linux", target_os = "macos")
))]
pub use terminal_host::{
    NativeTerminalBackgroundEntry, NativeTerminalBackgroundError,
    NativeTerminalBackgroundInspection, NativeTerminalBackgroundPage,
    NativeTerminalBackgroundRequester, NativeTerminalBackgroundSnapshot,
    NativeTerminalBackgroundStopReceipt, NativeTerminalBackgroundTarget,
    NativeTerminalHandoffReceipt, NativeTerminalLifecycleRequester, NativeTerminalResetEntry,
    NativeTerminalResetOutcome, NativeTerminalResetReceipt, NativeTerminalTransitionError,
};
mod terminal_display_width;
pub use terminal_display_width::{NativeTerminalDisplayUnit, native_terminal_display_unit_at};
mod terminal_grid;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod terminal_helper;
mod terminal_screen;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod terminal_shell;
mod terminal_tape_recording;
mod terminal_tape_replay;
pub use terminal_tape_recording::{
    MAX_TERMINAL_TAPE_RECORDING_BYTES, MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES,
    MAX_TERMINAL_TAPE_RECORDING_FRAMES, TerminalTapeRecordingDestination,
    TerminalTapeRecordingError, TerminalTapeRecordingFrame, TerminalTapeRecordingOptions,
    TerminalTapeRecordingRequest, TerminalTapeRecordingStatus,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use terminal_tape_recording::{TerminalTapeRecorder, TerminalTapeRecordingCompletion};
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod terminal_tmux_helper;
mod terminal_unicode_data;
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
mod tokio_web_search_deadline;
mod tool_output_serializer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod tool_result_archive;
mod tool_result_projection;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use native_tool_result_archive::NativeToolResultArchiveAdapter;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use tool_result_archive::{
    ArchivedToolResult, TOOL_RESULT_ARCHIVE_HANDLE_PREFIX, TOOL_RESULT_ARCHIVE_MAX_BYTES,
    TOOL_RESULT_ARCHIVE_MAX_ENTRIES, TOOL_RESULT_ARCHIVE_MAX_PAGE_BYTES,
    TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES, ToolResultArchive, ToolResultArchiveError,
    ToolResultArchiveHandle, ToolResultArchivePage,
};
mod utf8_boundary;
#[cfg(all(feature = "vision", not(target_family = "wasm")))]
mod vision;
mod vision_portable;
#[cfg(all(feature = "web-fetch-http", not(target_family = "wasm")))]
mod web_fetch;
mod web_search;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace;
mod workspace_inspection;
mod write_file;

pub use ai_gateway::{
    AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_INFERENCE_OPTIONS_KEY,
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_MAX_MODEL_BYTES,
    AI_GATEWAY_PROTOCOL_VERSION, AI_GATEWAY_PROVIDER_NAME, AiGatewayByteStream,
    AiGatewayConfigError, AiGatewayConfigErrorKind, AiGatewayHeader, AiGatewayInferenceOptions,
    AiGatewayLimits, AiGatewayProvider, AiGatewayToolInputLimits, AiGatewayTransport,
    AiGatewayTransportRequest,
};
#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
pub use ai_gateway_credential::{
    AI_GATEWAY_API_KEY_ENV, AiGatewayCredentialEnvironment, AiGatewayCredentialError,
    AiGatewayCredentialErrorKind, AiGatewayCredentialSource, DiscoveredAiGatewayCatalogCredential,
    DiscoveredAiGatewayCredential, VERCEL_OIDC_TOKEN_ENV, discover_ai_gateway_catalog_credential,
    discover_ai_gateway_credential, discover_process_ai_gateway_catalog_credential,
    discover_process_ai_gateway_credential,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use ai_gateway_http::{
    AI_GATEWAY_HTTP_DEFAULT_CONNECT_TIMEOUT, AI_GATEWAY_HTTP_DEFAULT_ENDPOINT,
    AI_GATEWAY_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS, AI_GATEWAY_HTTP_DEFAULT_REQUEST_TIMEOUT,
    AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES, AI_GATEWAY_HTTP_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_HTTP_MAX_CONNECT_TIMEOUT, AI_GATEWAY_HTTP_MAX_ENDPOINT_BYTES,
    AI_GATEWAY_HTTP_MAX_REQUEST_TIMEOUT, AI_GATEWAY_HTTP_MAX_RESPONSE_CHUNK_BYTES,
    AiGatewayHttpEndpoint, AiGatewayHttpLimits, AiGatewayHttpTransport,
};
#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
pub use ai_gateway_http_shared::{
    AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES, AiGatewayBearerToken, AiGatewayHttpConfigError,
    AiGatewayHttpConfigErrorKind,
};
pub use ai_gateway_model_catalog::{
    AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES, AI_GATEWAY_MODEL_CATALOG_MAX_JSON_DEPTH,
    AI_GATEWAY_MODEL_CATALOG_MAX_JSON_NODES, AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES,
    AI_GATEWAY_MODEL_CATALOG_MAX_MODELS, AI_GATEWAY_MODEL_CATALOG_MAX_RAW_ENTRIES,
    AI_GATEWAY_MODEL_CATALOG_PROVIDER_NAME, AI_GATEWAY_MODEL_CATALOG_REQUEST_TIMEOUT,
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogProvider,
    AiGatewayModelCatalogRequestAccess, AiGatewayModelCatalogTransport,
    AiGatewayModelCatalogTransportError, AiGatewayModelCatalogTransportErrorKind,
    AiGatewayModelCatalogTransportResponse, NativeModelCatalog, NativeModelCatalogEntry,
};
#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
pub use ai_gateway_model_catalog_http::{
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT,
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_ENDPOINT,
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT,
    AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ENDPOINT_BYTES,
    AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_RESPONSE_CHUNK_BYTES, AiGatewayModelCatalogHttpConfigError,
    AiGatewayModelCatalogHttpConfigErrorKind, AiGatewayModelCatalogHttpEndpoint,
    AiGatewayModelCatalogHttpLimits, AiGatewayModelCatalogHttpTransport,
};
#[cfg(all(feature = "vision", not(target_family = "wasm")))]
pub use ai_gateway_vision::{
    AI_GATEWAY_VISION_MODEL, AiGatewayVisionConfigError, AiGatewayVisionConfigErrorKind,
    AiGatewayVisionTransport,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use ai_gateway_web_search::AiGatewayWebSearchTransport;
pub use ask_permission::{
    ASK_PERMISSION_DENIED_REASON, ASK_PERMISSION_PROMPT_ERROR_CODE,
    ASK_PERMISSION_PROMPT_ERROR_MESSAGE, AskPermissionHandler, PermissionPromptDecision,
    PermissionPromptError, PermissionPrompter,
};
pub use ask_user_question::{
    ASK_USER_QUESTION_CANCELLED_SENTINEL, ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS,
    ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS, ASK_USER_QUESTION_TOOL_NAME,
    ASK_USER_QUESTION_UNAVAILABLE_SENTINEL, AskUserQuestionConfigError, AskUserQuestionTool,
    MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION, MAX_ASK_USER_QUESTION_QUESTIONS,
    MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES, MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES,
    MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES, MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES, MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES,
    MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
    MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES, MAX_ASK_USER_QUESTION_TOTAL_OPTIONS,
    MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, QuestionPrompt, QuestionPromptAnswers,
    QuestionPromptError, QuestionPromptOption, QuestionPromptOutcome, QuestionPromptRequest,
    QuestionPrompter,
};
pub use background_inspection::{
    MAX_BACKGROUND_COMMAND_BYTES, MAX_BACKGROUND_COMMAND_PREVIEW_BYTES,
    MAX_BACKGROUND_DIAGNOSTIC_BYTES, MAX_BACKGROUND_DIRECTORY_ENTRIES, MAX_BACKGROUND_JSON_DEPTH,
    MAX_BACKGROUND_JSON_NODES, MAX_BACKGROUND_PATH_BYTES, MAX_BACKGROUND_RECORD_BYTES,
    MAX_BACKGROUND_RECORDS, MAX_BACKGROUND_SERVER_URL_BYTES, MAX_BACKGROUND_STATE_BASE_BYTES,
    MAX_BACKGROUND_TOTAL_RECORD_BYTES, NativeBackgroundDetail, NativeBackgroundInspection,
    NativeBackgroundInspectionError, NativeBackgroundInspectionErrorKind, NativeBackgroundList,
    NativeBackgroundQuery, NativeBackgroundRecordSummary, NativeBackgroundState,
    inspect_native_background, inspect_process_background,
};
pub use background_process::run_background_process_helper;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use background_supervisor::{
    BACKGROUND_PROCESS_HELPER_ARGUMENT, NATIVE_BACKGROUND_DEFAULT_MAX_ACTIVE,
    NATIVE_BACKGROUND_HARD_MAX_ACTIVE, NativeBackgroundLimits, NativeBackgroundReconciliation,
    NativeBackgroundSupervisor, NativeBackgroundSupervisorError,
    NativeBackgroundSupervisorErrorKind,
};
pub use model_catalog_cache::{
    NATIVE_MODEL_CATALOG_MAX_WAITERS, NATIVE_MODEL_CATALOG_RETRY_MS, NativeModelCatalogCache,
    NativeModelCatalogCacheError, NativeModelCatalogCacheFailure, NativeModelCatalogCacheSnapshot,
    NativeModelCatalogCacheState,
};
pub use model_preferences::{
    MAX_NATIVE_REASONING_EFFORT_BYTES, MAX_NATIVE_REASONING_EFFORT_OPTIONS,
    NATIVE_MODEL_PREFERENCES_KEY, NativeEffectiveModelPreferences, NativeFastModeChange,
    NativeModelCapabilities, NativeModelPreferences, NativeModelPreferencesError,
    NativeModelSnapshot, NativeReasoningEffort,
};
pub use model_selection::{
    MAX_NATIVE_MODEL_QUERY_BYTES, NativeModelSelectionError, resolve_model_query,
};
pub use permission_controller::{
    NativePermissionActionPreparer, NativePermissionAutomaticOutcome,
    NativePermissionConfiguredOutcome, NativePermissionController, NativePermissionExecutionProof,
    NativePermissionPolicySnapshot, NativePermissionRuleChange, NativePermissionRulePrompt,
    NativePermissionRuleProposal, NativePermissionSession, NativePermissionTurn,
    NativePreparedPermissionAction,
};
pub use permission_patterns::{
    MAX_CONFIGURED_PERMISSION_MATCH_STEPS, MAX_CONFIGURED_PERMISSION_TARGET_BYTES,
    NativeConfiguredPermissionDecision, NativeConfiguredPermissionError,
    NativeConfiguredPermissionRule, NativeConfiguredPermissionRules, NativePermissionTargetKind,
    NativePreparedPermissionTarget,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use permission_reviewer::TokioPermissionReviewClock;
pub use permission_reviewer::{
    AiGatewayPermissionReviewer, MAX_NATIVE_PERMISSION_REVIEW_PACKET_BYTES,
    MAX_NATIVE_PERMISSION_REVIEW_RATIONALE_BYTES, NATIVE_PERMISSION_REVIEW_MODEL,
    NATIVE_PERMISSION_REVIEW_TIMEOUT, NativeAutoPermissionAction, NativeAutoPermissionAssessment,
    NativeAutoPermissionAuthorization, NativeAutoPermissionDecision,
    NativeAutoPermissionFilePreimage, NativeAutoPermissionOrigin, NativeAutoPermissionPhase,
    NativeAutoPermissionReview, NativeAutoPermissionReviewError, NativeAutoPermissionRisk,
    NativeAutoPermissionRootContext, NativeAutoPermissionSandboxScope, NativeAutoPermissionTarget,
    NativePermissionReviewClock, NativePermissionReviewer,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[doc(hidden)]
pub use terminal_helper::{
    TERMINAL_PTY_HELPER_ARGUMENT, TERMINAL_STARTUP_MARKER_ARGUMENT, TerminalHelperError,
    run_terminal_pty_helper, run_terminal_startup_marker,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[doc(hidden)]
pub use terminal_tmux_helper::{
    TERMINAL_TMUX_HELPER_ARGUMENT, TerminalTmuxLaunchError, run_terminal_tmux_helper,
};
#[cfg(all(feature = "vision", not(target_family = "wasm")))]
pub use vision::{
    MAX_VISION_BATCH_BYTES, MAX_VISION_IMAGE_BYTES, MAX_VISION_IMAGES, MAX_VISION_PATH_BYTES,
    MAX_VISION_PATH_COMPONENT_BYTES, MAX_VISION_PATH_COMPONENTS,
    MAX_VISION_SERIALIZED_RESULT_BYTES, MAX_VISION_TOTAL_IMAGE_BYTES,
    VISION_DEFAULT_MAX_ACTIVE_REQUESTS, VISION_DEFAULT_REQUEST_TIMEOUT, VISION_MAX_ACTIVE_REQUESTS,
    VISION_TOOL_NAME, VisionConfigError, VisionConfigErrorKind, VisionLimits, VisionTool,
};
pub use vision_portable::{
    MAX_VISION_ATTEMPT_EVIDENCE_BYTES, MAX_VISION_BATCH_IMAGES, MAX_VISION_BATCH_RAW_BYTES,
    MAX_VISION_EVIDENCE_LIST_ITEMS, MAX_VISION_EVIDENCE_STRING_BYTES, MAX_VISION_FOCUS_BYTES,
    MAX_VISION_REQUEST_BYTES, MAX_VISION_RESPONSE_BYTES, MAX_VISION_RESPONSE_JSON_NODES,
    MAX_VISION_RESPONSE_RECORD_BYTES, MAX_VISION_RESPONSE_RECORDS, VisionBatchRequest,
    VisionBatchResponse, VisionDeadline, VisionImage, VisionImageOutcome, VisionImageResult,
    VisionMediaType, VisionProviderFailure, VisionProviderFailureCode, VisionTransport,
    VisionTransportError, VisionTransportErrorKind,
};

pub use config::{
    CONFIG_SCHEMA_VERSION, ConfigOrigin, LoadedNativeConfig, MAX_CONFIG_BYTES, NativeConfig,
    NativeConfigError, NativeConfigErrorKind, NativeConfiguredPermissionMutation,
    NativeConfiguredPermissionMutationOutcome, NativeConfiguredPermissionReset,
    NativeConfiguredPermissionScope, NativeConfiguredPermissionSources, NativeCredentialSourceKind,
    NativeProviderKind, NativeSavedWorkspaceDirectory, NativeTransportKind,
    NativeWorkspaceDirectoryMutation, load_native_config, load_process_config,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use conversation::{
    NATIVE_CONVERSATION_CHECKPOINT_KEY, NativeConversation, NativeConversationError,
    NativeConversationTurn, NativePausedTurn,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use conversation_context::{
    NATIVE_CONTEXT_PREFERENCES_KEY, NativeContextError, NativeContextPreferences,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use conversation_model_routes::{
    MAX_NATIVE_CONVERSATION_MODEL_ROUTES, NativeConversationModelRouteError,
    NativeConversationModelRoutes,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use conversation_runtime::{
    MAX_NATIVE_QUEUED_INPUT_BYTES, MAX_NATIVE_QUEUED_JOBS, MAX_NATIVE_QUEUED_OPTIONS_BYTES,
    MAX_NATIVE_QUEUED_PROMPT_BYTES, NativeConversationRuntime, NativeConversationRuntimeError,
    NativeConversationRuntimePhase, NativeConversationRuntimeStatus, NativeConversationRuntimeTurn,
    NativeModelPreferenceCommit, NativeModelPreferencePersistence, NativeQueuedJobId,
    NativeQuiescentSelectionSnapshot, NativeRuntimeQuiescence,
};
pub use copy_file::{
    COPY_FILE_TOOL_NAME, CopyFileTool, CopyFileToolOpenError, CopyFileToolOpenErrorKind,
    MAX_COPY_FILE_CHUNK_BYTES, MAX_COPY_FILE_IO_CALLS, MAX_COPY_FILE_PATH_BYTES,
    MAX_COPY_FILE_PATH_COMPONENTS, MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_COPY_FILE_SERIALIZED_RESULT_BYTES, MAX_COPY_FILE_SOURCE_BYTES, MAX_COPY_FILE_TEMP_ATTEMPTS,
};
pub use create_folder::{
    CREATE_FOLDER_TOOL_NAME, CreateFolderTool, CreateFolderToolOpenError,
    CreateFolderToolOpenErrorKind, MAX_CREATE_FOLDER_MKDIR_CALLS, MAX_CREATE_FOLDER_PATH_BYTES,
    MAX_CREATE_FOLDER_PATH_COMPONENTS, MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES,
    MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES, MAX_CREATE_FOLDER_SYNC_CALLS,
};
pub use delete_file::{
    DELETE_FILE_TOOL_NAME, DeleteFileTool, DeleteFileToolOpenError, DeleteFileToolOpenErrorKind,
    MAX_DELETE_FILE_PATH_BYTES, MAX_DELETE_FILE_PATH_COMPONENTS,
    MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES,
};
pub use doctor::{
    NATIVE_DOCTOR_CHECK_COUNT, NativeDoctorCheck, NativeDoctorCheckStatus,
    NativeDoctorCredentialStatus, NativeDoctorReport, inspect_native_doctor,
    inspect_process_doctor,
};
pub use edit_file::{
    EDIT_FILE_TOOL_NAME, EditFileTool, EditFileToolOpenError, EditFileToolOpenErrorKind,
    MAX_EDIT_FILE_CHUNK_BYTES, MAX_EDIT_FILE_EXISTING_BYTES, MAX_EDIT_FILE_MATCH_WORK_STEPS,
    MAX_EDIT_FILE_NEW_STRING_BYTES, MAX_EDIT_FILE_OLD_STRING_BYTES, MAX_EDIT_FILE_PATH_BYTES,
    MAX_EDIT_FILE_PATH_COMPONENTS, MAX_EDIT_FILE_RESULTING_BYTES,
    MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES,
    MAX_EDIT_FILE_TEMP_ATTEMPTS,
};
pub use file_info::{
    FILE_INFO_TOOL_NAME, FileInfoTool, FileInfoToolOpenError, FileInfoToolOpenErrorKind,
    MAX_FILE_INFO_PATH_BYTES,
};
pub use file_undo::{
    FileUndoClearReservation, FileUndoError, FileUndoOutcome, FileUndoTracker,
    FileUndoUnavailableReason, MAX_FILE_UNDO_ENTRIES, MAX_FILE_UNDO_OBSERVATION_BYTES,
    MAX_FILE_UNDO_PREIMAGE_BYTES, MAX_FILE_UNDO_RETAINED_BYTES,
};
pub use glob_files::{
    GLOB_FILES_TOOL_NAME, GlobFilesTool, GlobFilesToolOpenError, GlobFilesToolOpenErrorKind,
    MAX_GLOB_FILES_DEPTH, MAX_GLOB_FILES_MATCH_STEPS, MAX_GLOB_FILES_MATCHES,
    MAX_GLOB_FILES_PATH_BYTES, MAX_GLOB_FILES_PATTERN_BYTES, MAX_GLOB_FILES_RESULT_PATH_BYTES,
    MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES, MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES,
    MAX_GLOB_FILES_VISITED_ENTRIES,
};
pub use grep_files::{
    GREP_FILES_TOOL_NAME, GrepFilesTool, GrepFilesToolOpenError, GrepFilesToolOpenErrorKind,
    MAX_GREP_FILES_CANDIDATE_FILES, MAX_GREP_FILES_CONTENT_MATCH_STEPS,
    MAX_GREP_FILES_CONTEXT_LINES, MAX_GREP_FILES_DEPTH, MAX_GREP_FILES_FILE_BYTES,
    MAX_GREP_FILES_HEAD_LIMIT, MAX_GREP_FILES_INCLUDE_BYTES, MAX_GREP_FILES_INCLUDE_MATCH_STEPS,
    MAX_GREP_FILES_OFFSET, MAX_GREP_FILES_PATH_BYTES, MAX_GREP_FILES_PATTERN_BYTES,
    MAX_GREP_FILES_RESULT_LINE_BYTES, MAX_GREP_FILES_RESULT_PATH_BYTES,
    MAX_GREP_FILES_SERIALIZED_RESULT_BYTES, MAX_GREP_FILES_TOTAL_CONTENT_BYTES,
    MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES, MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES,
    MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES, MAX_GREP_FILES_VISITED_ENTRIES,
};
pub use install_skill::{
    INSTALL_SKILL_TOOL_NAME, InstallSkillTool, InstallSkillToolOpenError,
    InstallSkillToolOpenErrorKind, MAX_INSTALL_SKILL_CHUNK_BYTES,
    MAX_INSTALL_SKILL_COMPONENT_BYTES, MAX_INSTALL_SKILL_ENTRIES,
    MAX_INSTALL_SKILL_ENTRY_NAME_BYTES, MAX_INSTALL_SKILL_FILE_BYTES,
    MAX_INSTALL_SKILL_IO_ATTEMPTS, MAX_INSTALL_SKILL_NAME_BYTES, MAX_INSTALL_SKILL_PATH_BYTES,
    MAX_INSTALL_SKILL_PATH_COMPONENTS, MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES,
    MAX_INSTALL_SKILL_SERIALIZED_RESULT_BYTES, MAX_INSTALL_SKILL_SOURCE_BYTES,
    MAX_INSTALL_SKILL_STAGE_ATTEMPTS, MAX_INSTALL_SKILL_TOTAL_BYTES,
};
pub use list_files::{
    LIST_FILES_TOOL_NAME, ListFilesTool, ListFilesToolOpenError, ListFilesToolOpenErrorKind,
    MAX_LIST_FILES_ENTRIES, MAX_LIST_FILES_PATH_BYTES, MAX_LIST_FILES_TOTAL_NAME_BYTES,
};
pub use mcp_features::{
    MAX_MCP_FEATURE_ARGUMENTS_BYTES, MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES,
    MAX_MCP_FEATURE_COMPLETION_VALUES, MAX_MCP_FEATURE_CONTENT_FIELD_BYTES,
    MAX_MCP_FEATURE_CONTENT_ITEMS, MAX_MCP_FEATURE_CONTEXT_BYTES, MAX_MCP_FEATURE_CONTEXT_PAIRS,
    MAX_MCP_FEATURE_DESCRIPTION_BYTES, MAX_MCP_FEATURE_ICON_SIZES, MAX_MCP_FEATURE_ICONS,
    MAX_MCP_FEATURE_JSON_DEPTH, MAX_MCP_FEATURE_JSON_NODES, MAX_MCP_FEATURE_NAME_BYTES,
    MAX_MCP_FEATURE_PAYLOAD_BYTES, MAX_MCP_FEATURE_PROMPT_ARGUMENTS,
    MAX_MCP_FEATURE_SERIALIZED_ARGUMENT_BYTES, MAX_MCP_FEATURE_SERIALIZED_RESULT_BYTES,
    MAX_MCP_FEATURE_SERVER_BYTES, MAX_MCP_FEATURE_TITLE_BYTES, MAX_MCP_FEATURE_URI_BYTES,
    MCP_FEATURES_TOOL_NAME, McpFeatureAction, McpFeatureAuthority, McpFeatureError,
    McpFeatureErrorKind, McpFeaturePayload, McpFeatureRequest, McpFeaturesTool,
};
pub use mcp_search_tools::{
    MAX_MCP_SEARCH_DESCRIPTION_BYTES, MAX_MCP_SEARCH_MATCH_STEPS, MAX_MCP_SEARCH_QUERY_BYTES,
    MAX_MCP_SEARCH_QUERY_TOKENS, MAX_MCP_SEARCH_SERIALIZED_ARGUMENT_BYTES,
    MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES, MAX_MCP_SELECTED_TOOL_SPEC_BYTES,
    MAX_MCP_TOOL_CATALOG_BYTES, MAX_MCP_TOOL_CATALOG_ENTRIES, MAX_MCP_TOOL_DESCRIPTION_BYTES,
    MAX_MCP_TOOL_SCHEMA_DEPTH, MAX_MCP_TOOL_SCHEMA_NODES, MAX_MCP_TOOL_SEARCH_TEXT_BYTES,
    MAX_MCP_TOOL_SERVER_BYTES, MAX_MCP_TOOL_TAG_BYTES, MAX_MCP_TOOL_TAGS,
    MCP_SEARCH_TOOLS_DEFAULT_LIMIT, MCP_SEARCH_TOOLS_MAX_LIMIT, MCP_SEARCH_TOOLS_TOOL_NAME,
    McpSearchToolsTool, McpToolCatalog, McpToolCatalogBuildError, McpToolCatalogBuildErrorKind,
    McpToolCatalogError, McpToolCatalogErrorKind, McpToolCatalogSnapshot, McpToolCatalogState,
    McpToolMetadata,
};
pub use mcp_select_tool::{
    MAX_MCP_SELECT_SERIALIZED_ARGUMENT_BYTES, MAX_MCP_SELECT_SERIALIZED_RESULT_BYTES,
    MCP_SELECT_TOOL_NAME, McpSelectTool,
};
pub use memory::{
    MAX_MEMORY_FACT_BYTES, MAX_MEMORY_FACTS, MAX_MEMORY_FILE_BYTES, MAX_MEMORY_IO_ATTEMPTS,
    MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES, MAX_MEMORY_SERIALIZED_RESULT_BYTES,
    MAX_MEMORY_TOTAL_FACT_BYTES, MEMORY_SCHEMA_VERSION, MEMORY_TOOL_NAME, MemoryTool,
    MemoryToolOpenError, MemoryToolOpenErrorKind,
};
pub use open_file::{
    MAX_CONCURRENT_OPEN_FILE_LAUNCHES, MAX_OPEN_FILE_PATH_BYTES,
    MAX_OPEN_FILE_PATH_COMPONENT_BYTES, MAX_OPEN_FILE_PATH_COMPONENTS,
    MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES,
    OPEN_FILE_LAUNCH_TIMEOUT, OPEN_FILE_TOOL_NAME, OpenFileTool, OpenFileToolOpenError,
    OpenFileToolOpenErrorKind,
};
#[cfg(target_os = "linux")]
pub use open_file::{
    OpenFileLaunch, OpenFileLaunchOutcome, OpenFileLaunchRequest, OpenFileLauncher,
};
pub use read_file::{
    MAX_READ_FILE_BYTES, MAX_READ_FILE_PATH_BYTES, READ_FILE_TOOL_NAME, ReadFileTool,
    ReadFileToolOpenError, ReadFileToolOpenErrorKind,
};
pub use read_tool_result::{
    READ_TOOL_RESULT_MAX_SOURCE_BYTES, READ_TOOL_RESULT_TOOL_NAME, ReadToolResultConfigError,
    ReadToolResultConfigErrorKind, ReadToolResultLimits, ReadToolResultTool,
};
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
pub use reference_host::{
    NativeReferenceHost, NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
    NativeReferenceHostConversationOptions, NativeReferenceHostPermissionOptions,
    NativeReferenceHostTerminalOptions,
};
pub use rename_file::{
    MAX_RENAME_FILE_PATH_BYTES, MAX_RENAME_FILE_PATH_COMPONENTS,
    MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES,
    RENAME_FILE_TOOL_NAME, RenameFileTool, RenameFileToolOpenError, RenameFileToolOpenErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use root_selection::{
    NativeRootSelection, NativeRootSelectionError, NativeRootSelectionErrorKind,
    PreparedNativeRoots, PreparedNativeRootsError, PreparedNativeRootsErrorKind,
};
pub use runtime_status::{
    MAX_NATIVE_RUNTIME_BUILD_REVISION_BYTES, MAX_NATIVE_RUNTIME_WORKSPACE_PATH_BYTES,
    NATIVE_RUNTIME_BUILD_CHANNEL, NATIVE_RUNTIME_MISSING_AUTH_HELP, NATIVE_RUNTIME_SANDBOX,
    NATIVE_RUNTIME_UPDATE_CHANNEL, NativeRuntimeCredentialEnvironment,
    NativeRuntimeCredentialSource, NativeRuntimeStatus, NativeRuntimeStatusError,
    NativeRuntimeStatusErrorKind, NativeRuntimeStatusInput, inspect_native_runtime_status,
    inspect_process_runtime_status,
};
pub use semantic_search::{
    MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS, MAX_SEMANTIC_SEARCH_DEPTH,
    MAX_SEMANTIC_SEARCH_DIRECTORY_READ_ATTEMPTS, MAX_SEMANTIC_SEARCH_FILE_BYTES,
    MAX_SEMANTIC_SEARCH_KEYWORDS, MAX_SEMANTIC_SEARCH_MATCH_STEPS, MAX_SEMANTIC_SEARCH_PATH_BYTES,
    MAX_SEMANTIC_SEARCH_QUERY_BYTES, MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES,
    MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES, MAX_SEMANTIC_SEARCH_RETAINED_RESULTS,
    MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES, MAX_SEMANTIC_SEARCH_SHOWN_RESULTS,
    MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES, MAX_SEMANTIC_SEARCH_TOTAL_ENTRY_NAME_BYTES,
    MAX_SEMANTIC_SEARCH_TOTAL_RESULT_LINE_BYTES, MAX_SEMANTIC_SEARCH_TOTAL_RESULT_PATH_BYTES,
    MAX_SEMANTIC_SEARCH_VISITED_ENTRIES, SEMANTIC_SEARCH_TOOL_NAME, SemanticSearchTool,
    SemanticSearchToolOpenError, SemanticSearchToolOpenErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_catalog::{
    MAX_NATIVE_SESSION_CATALOG_QUERY_BYTES, MAX_NATIVE_SESSION_PREVIEW_BYTES, NativeSessionCatalog,
    NativeSessionCatalogEntry, NativeSessionCatalogError, NativeSessionCatalogErrorKind,
    NativeSessionCatalogInvalidRecords, NativeSessionCatalogPage, NativeSessionCatalogQuery,
    NativeSessionSelectionIncomplete, inspect_native_session_catalog_entry,
    inspect_process_session_catalog_entry, list_native_session_catalog,
    list_process_current_workspace_session_catalog, list_process_session_catalog,
};
pub use session_catalog_cursor::{
    MAX_NATIVE_SESSION_CATALOG_CURSOR_BYTES, NativeSessionCatalogCursor,
    NativeSessionCatalogCursorError,
};
pub use session_inspection::{
    NativeSessionInspection, NativeSessionInspectionError, NativeSessionInspectionErrorKind,
    inspect_native_session, inspect_process_session,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_lifecycle::{
    MAX_SESSION_ID_ATTEMPTS, MAX_SESSION_INCARNATION_ATTEMPTS, NativeSessionLifecycle,
    NativeSessionLifecycleBuildError, NativeSessionLifecycleBuildErrorKind,
    NativeSessionLifecycleError, NativeSessionLifecycleErrorKind, SessionIdSource,
    SessionIdSourceError, SessionIncarnationSource, SessionIncarnationSourceError,
};
pub use session_listing::{
    NativeSessionList, NativeSessionListingError, NativeSessionListingErrorKind,
    list_native_sessions, list_process_sessions,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_metadata::{
    MAX_NATIVE_SESSION_LANGUAGE_BYTES, MAX_NATIVE_SESSION_TITLE_BYTES,
    MAX_NATIVE_SESSION_WORKSPACE_BYTES, NATIVE_SESSION_METADATA_KEY, NativeSessionMetadata,
    NativeSessionMetadataError, NativeSessionOrigin,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_metadata_commands::{
    MAX_NATIVE_WORKSPACE_REBINDINGS, NATIVE_WORKSPACE_REBINDINGS_KEY,
    NativeSessionMetadataMutationError, NativeWorkspaceRebinding, NativeWorkspaceRebindingHistory,
    rebind_native_session_workspace, rename_native_session,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_resume::{
    NativeObservedSession, NativePreparedResume, NativeResumeTarget, NativeSessionResumeError,
    NativeSessionResumeErrorKind, prepare_native_session_resume,
};
pub use session_store::{
    FILE_SESSION_SCHEMA_VERSION, FileSessionStore, FileSessionStoreOpenError,
    FileSessionStoreOpenErrorKind, MAX_FILE_SESSION_BYTES, MAX_LIST_SESSION_DIRECTORY_ENTRIES,
    MAX_LIST_SESSION_TOTAL_RECORD_BYTES, MAX_LIST_SESSIONS,
};
pub use skill::{
    MAX_SKILL_CHUNK_BYTES, MAX_SKILL_FILE_BYTES, MAX_SKILL_IO_ATTEMPTS, MAX_SKILL_NAME_BYTES,
    MAX_SKILL_PATH_BYTES, MAX_SKILL_PATH_COMPONENT_BYTES, MAX_SKILL_PATH_COMPONENTS,
    MAX_SKILL_RESOURCE_BYTES, MAX_SKILL_SERIALIZED_ARGUMENT_BYTES,
    MAX_SKILL_SERIALIZED_RESULT_BYTES, SKILL_TOOL_NAME, SkillTool, SkillToolOpenError,
    SkillToolOpenErrorKind,
};
pub use slash_commands::{
    MAX_NATIVE_SLASH_INPUT_BYTES, MAX_NATIVE_SLASH_QUERY_BYTES, NativeSlashCategory,
    NativeSlashCommand, NativeSlashCompletion, NativeSlashCompletions, NativeSlashHelp,
    NativeSlashInputError, NativeSlashInvocation, NativeSlashRoute, NativeSlashSpec,
    NativeSlashSubmission, NativeSlashSubmissionContext, native_slash_completion_prefix,
    native_slash_completions, native_slash_help, native_slash_registry,
    resolve_native_slash_submission, route_native_slash,
};
pub use terminal::{
    MAX_TERMINAL_BACKGROUND_READ_BYTES, MAX_TERMINAL_BACKGROUND_WRITE_BYTES,
    MAX_TERMINAL_COMMAND_BYTES, MAX_TERMINAL_CWD_BYTES, MAX_TERMINAL_CWD_COMPONENT_BYTES,
    MAX_TERMINAL_CWD_COMPONENTS, MAX_TERMINAL_ENVIRONMENT_BYTES, MAX_TERMINAL_ENVIRONMENT_ENTRIES,
    MAX_TERMINAL_ENVIRONMENT_KEY_BYTES, MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES,
    MAX_TERMINAL_PRODUCED_OUTPUT_BYTES, MAX_TERMINAL_RETAINED_OUTPUT_BYTES,
    MAX_TERMINAL_SERIALIZED_ARGUMENT_BYTES, MAX_TERMINAL_SERIALIZED_RESULT_BYTES,
    TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE, TERMINAL_DEFAULT_MAX_ACTIVE_EXECUTIONS,
    TERMINAL_DEFAULT_TIMEOUT, TERMINAL_ENVIRONMENT_PROFILE, TERMINAL_MAX_ACTIVE_EXECUTIONS,
    TERMINAL_MAX_ACTIVE_LISTS, TERMINAL_MAX_ACTIVE_SIGNALS, TERMINAL_MAX_ACTIVE_WAITS,
    TERMINAL_MAX_ACTIVE_WRITES, TERMINAL_MAX_TIMEOUT, TERMINAL_MAX_WAIT_CEILING_MS,
    TERMINAL_MAX_WAIT_OBSERVATIONS, TERMINAL_PROGRAM, TERMINAL_TOOL_NAME,
    TerminalBackgroundCatalog, TerminalBackgroundInspector, TerminalBackgroundOutcome,
    TerminalBackgroundOutputReader, TerminalBackgroundReadError, TerminalBackgroundReadErrorKind,
    TerminalBackgroundReadSnapshot, TerminalBackgroundSignal, TerminalBackgroundSignalCompletion,
    TerminalBackgroundSignalError, TerminalBackgroundSignalErrorKind,
    TerminalBackgroundSignalOutcome, TerminalBackgroundSignaler, TerminalBackgroundStarter,
    TerminalBackgroundWaitDelay, TerminalBackgroundWaitDelayError,
    TerminalBackgroundWriteCompletion, TerminalBackgroundWriteError,
    TerminalBackgroundWriteErrorKind, TerminalBackgroundWriteOutcome,
    TerminalBackgroundWriteStatus, TerminalBackgroundWriter, TerminalCapturedOutput,
    TerminalConfigError, TerminalConfigErrorKind, TerminalExecution, TerminalExecutionOutcome,
    TerminalExecutionRequest, TerminalExecutionStatus, TerminalExecutor, TerminalExecutorError,
    TerminalExecutorErrorKind, TerminalLimits, TerminalTool,
};
pub use terminal_action_parse::{
    MAX_TERMINAL_ACTION_ARGUMENT_BYTES, MAX_TERMINAL_ACTION_ARGUMENT_NODES,
    TerminalActionParseError, decode_terminal_action, terminal_action_input_schema,
    terminal_action_requested_cwd,
};
pub use terminal_action_tool::{
    MAX_TERMINAL_ACTION_RESULT_BYTES, MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES,
    MAX_TERMINAL_PREPARED_ARGUMENT_BYTES, TerminalActionExecutor, TerminalActionHostIdentity,
    TerminalActionInputPublisher, TerminalActionInvocation, TerminalActionResultPublisher,
    TerminalActionTool,
};
pub use terminal_screen::{
    MAX_TERMINAL_SCREEN_FEED_BYTES, TerminalScreenEngine, TerminalScreenError, TerminalScreenMode,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use terminal_shell::{TerminalShell, TerminalShellError};
pub use terminal_tape_replay::{
    MAX_TERMINAL_TAPE_ARTIFACT_BYTES, MAX_TERMINAL_TAPE_BYTES, MAX_TERMINAL_TAPE_FRAMES_DIR_FRAMES,
    MAX_TERMINAL_TAPE_RENDERED_OUTPUT_BYTES, TerminalTapeReplayError, TerminalTapeReplayErrorKind,
    TerminalTapeReplayOutput, TerminalTapeReplayRequest, replay_terminal_tape,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use tokio_web_search_deadline::{TokioWebSearchDeadline, TokioWebSearchRuntime};
#[cfg(all(feature = "web-fetch-http", not(target_family = "wasm")))]
pub use web_fetch::{
    MAX_WEB_FETCH_BODY_BYTES, MAX_WEB_FETCH_DNS_ADDRESSES, MAX_WEB_FETCH_MIME_TYPE_BYTES,
    MAX_WEB_FETCH_SERIALIZED_RESULT_BYTES, MAX_WEB_FETCH_URL_BYTES,
    WEB_FETCH_DEFAULT_CONNECT_TIMEOUT, WEB_FETCH_DEFAULT_MAX_ACTIVE_REQUESTS,
    WEB_FETCH_DEFAULT_REQUEST_TIMEOUT, WEB_FETCH_MAX_ACTIVE_REQUESTS, WEB_FETCH_TOOL_NAME,
    WebFetchConfigError, WebFetchConfigErrorKind, WebFetchLimits, WebFetchRequest,
    WebFetchResponse, WebFetchTool, WebFetchTransport, WebFetchTransportError,
    WebFetchTransportErrorKind,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use web_search::WebSearchTool;
pub use web_search::{
    MAX_WEB_SEARCH_DOMAIN_BYTES, MAX_WEB_SEARCH_DOMAIN_FILTERS, MAX_WEB_SEARCH_JSON_NODES,
    MAX_WEB_SEARCH_QUERY_BYTES, MAX_WEB_SEARCH_REQUEST_BYTES, MAX_WEB_SEARCH_RESPONSE_BYTES,
    MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES, MAX_WEB_SEARCH_RESPONSE_RECORDS,
    MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES, MAX_WEB_SEARCH_SOURCE_TITLE_BYTES,
    MAX_WEB_SEARCH_SOURCE_URL_BYTES, MAX_WEB_SEARCH_SOURCES, MAX_WEB_SEARCH_TOTAL_DOMAIN_BYTES,
    WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS, WEB_SEARCH_DEFAULT_REQUEST_TIMEOUT,
    WEB_SEARCH_MAX_ACTIVE_REQUESTS, WEB_SEARCH_TOOL_NAME, WebSearchConfigError,
    WebSearchConfigErrorKind, WebSearchDeadline, WebSearchLimits, WebSearchRequest,
    WebSearchResponse, WebSearchSource, WebSearchTransport, WebSearchTransportError,
    WebSearchTransportErrorKind,
};
pub use workspace_inspection::{
    MAX_WORKSPACE_PATH_BYTES, NativeWorkspaceInspection, NativeWorkspaceInspectionError,
    NativeWorkspaceInspectionErrorKind, inspect_process_workspace,
};
pub use write_file::{
    MAX_WRITE_FILE_CHUNK_BYTES, MAX_WRITE_FILE_CONTENT_BYTES, MAX_WRITE_FILE_PATH_BYTES,
    MAX_WRITE_FILE_PATH_COMPONENTS, MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES, MAX_WRITE_FILE_TEMP_ATTEMPTS, WRITE_FILE_TOOL_NAME,
    WriteFileTool, WriteFileToolOpenError, WriteFileToolOpenErrorKind,
};

/// Core API version intentionally supported by this native host.
pub const SUPPORTED_CORE_API_VERSION: u32 = 1;

/// Namespace used for machine-god's native state and configuration.
pub const STATE_NAMESPACE: &str = "machine-god";

/// File name used for machine-god's native configuration.
pub const CONFIG_FILE_NAME: &str = "config.json";

/// Returns the core API version supported by this native host.
#[must_use]
pub const fn supported_core_api_version() -> u32 {
    SUPPORTED_CORE_API_VERSION
}
