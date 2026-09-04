#![doc = "Explicit native capabilities for machine-god hosts."]

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

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
mod background_inspection;
mod background_process;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod background_store;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod background_supervisor;
mod config;
mod copy_file;
mod create_folder;
mod delete_file;
mod doctor;
mod edit_file;
mod file_info;
mod glob_files;
mod grep_files;
mod install_skill;
mod list_files;
mod mcp_features;
mod mcp_search_tools;
mod mcp_select_tool;
mod memory;
mod open_file;
mod read_file;
mod read_tool_result;
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
mod reference_host;
mod rename_file;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod root_selection;
mod runtime_status;
mod semantic_search;
mod session_inspection;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_lifecycle;
mod session_listing;
mod session_store;
mod skill;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod state_environment;
mod terminal;
mod terminal_display_width;
mod terminal_grid;
mod terminal_tape_replay;
mod terminal_unicode_data;
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
mod tokio_web_search_deadline;
mod tool_output_serializer;
mod tool_result_projection;
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
    AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION,
    AI_GATEWAY_MAX_MODEL_BYTES, AI_GATEWAY_PROTOCOL_VERSION, AI_GATEWAY_PROVIDER_NAME,
    AiGatewayByteStream, AiGatewayConfigError, AiGatewayConfigErrorKind, AiGatewayHeader,
    AiGatewayLimits, AiGatewayProvider, AiGatewayTransport, AiGatewayTransportRequest,
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
    AiGatewayModelCatalogTransportResponse,
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
    AiGatewayVisionConfigError, AiGatewayVisionConfigErrorKind, AiGatewayVisionTransport,
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
    NativeConfigError, NativeConfigErrorKind, NativeCredentialSourceKind, NativeProviderKind,
    NativeTransportKind, load_native_config, load_process_config,
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
pub use terminal::{
    MAX_TERMINAL_COMMAND_BYTES, MAX_TERMINAL_CWD_BYTES, MAX_TERMINAL_CWD_COMPONENT_BYTES,
    MAX_TERMINAL_CWD_COMPONENTS, MAX_TERMINAL_ENVIRONMENT_BYTES, MAX_TERMINAL_ENVIRONMENT_ENTRIES,
    MAX_TERMINAL_ENVIRONMENT_KEY_BYTES, MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES,
    MAX_TERMINAL_PRODUCED_OUTPUT_BYTES, MAX_TERMINAL_RETAINED_OUTPUT_BYTES,
    MAX_TERMINAL_SERIALIZED_ARGUMENT_BYTES, MAX_TERMINAL_SERIALIZED_RESULT_BYTES,
    TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE, TERMINAL_DEFAULT_MAX_ACTIVE_EXECUTIONS,
    TERMINAL_DEFAULT_TIMEOUT, TERMINAL_ENVIRONMENT_PROFILE, TERMINAL_MAX_ACTIVE_EXECUTIONS,
    TERMINAL_MAX_ACTIVE_WAITS, TERMINAL_MAX_TIMEOUT, TERMINAL_MAX_WAIT_CEILING_MS,
    TERMINAL_MAX_WAIT_OBSERVATIONS, TERMINAL_PROGRAM, TERMINAL_TOOL_NAME,
    TerminalBackgroundInspector, TerminalBackgroundOutcome, TerminalBackgroundStarter,
    TerminalBackgroundWaitDelay, TerminalBackgroundWaitDelayError, TerminalCapturedOutput,
    TerminalConfigError, TerminalConfigErrorKind, TerminalExecution, TerminalExecutionOutcome,
    TerminalExecutionRequest, TerminalExecutionStatus, TerminalExecutor, TerminalExecutorError,
    TerminalExecutorErrorKind, TerminalLimits, TerminalTool,
};
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

/// Permission behavior used by the native host.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PermissionMode {
    /// Ask before exercising a permission-gated native capability.
    #[default]
    Ask,
}

impl PermissionMode {
    /// Returns the stable, machine-readable name of this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
        }
    }
}

/// Environment inputs used to locate native configuration and state.
///
/// Owned values make inspection deterministic when callers inject a snapshot.
/// The debug representation intentionally reports only whether each input was
/// present because environment values can contain sensitive path information.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeEnvironment {
    xdg_config_home: Option<OsString>,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
}

impl NativeEnvironment {
    /// Creates an environment snapshot from injected values.
    #[must_use]
    pub const fn new(
        xdg_config_home: Option<OsString>,
        xdg_state_home: Option<OsString>,
        home: Option<OsString>,
    ) -> Self {
        Self {
            xdg_config_home,
            xdg_state_home,
            home,
        }
    }

    /// Captures the relevant values from the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self::new(
            env::var_os("XDG_CONFIG_HOME"),
            env::var_os("XDG_STATE_HOME"),
            env::var_os("HOME"),
        )
    }
}

impl fmt::Debug for NativeEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeEnvironment")
            .field("has_xdg_config_home", &self.xdg_config_home.is_some())
            .field("has_xdg_state_home", &self.xdg_state_home.is_some())
            .field("has_home", &self.home.is_some())
            .finish()
    }
}

/// Observed state of the resolved native configuration file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFileState {
    /// The path is a regular file.
    File,
    /// Nothing exists at the path.
    Missing,
    /// The path exists but is not a regular file.
    NotFile,
    /// Metadata for the path could not be inspected.
    Inaccessible,
    /// No path could be resolved because no applicable environment input exists.
    Unavailable,
    /// The selected environment input is invalid.
    InvalidEnvironment,
}

impl ConfigFileState {
    /// Returns the stable, machine-readable name of this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Missing => "missing",
            Self::NotFile => "not_file",
            Self::Inaccessible => "inaccessible",
            Self::Unavailable => "unavailable",
            Self::InvalidEnvironment => "invalid_environment",
        }
    }
}

/// Observed state of the resolved native state directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateDirectoryState {
    /// The path is a directory.
    Directory,
    /// Nothing exists at the path.
    Missing,
    /// The path exists but is not a directory.
    NotDirectory,
    /// Metadata for the path could not be inspected.
    Inaccessible,
    /// No path could be resolved because no applicable environment input exists.
    Unavailable,
    /// The selected environment input is invalid.
    InvalidEnvironment,
}

impl StateDirectoryState {
    /// Returns the stable, machine-readable name of this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Missing => "missing",
            Self::NotDirectory => "not_directory",
            Self::Inaccessible => "inaccessible",
            Self::Unavailable => "unavailable",
            Self::InvalidEnvironment => "invalid_environment",
        }
    }
}

/// Read-only native host status derived from an environment snapshot.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeStatus {
    permission_mode: PermissionMode,
    config_file_path: Option<PathBuf>,
    config_file_state: ConfigFileState,
    state_directory_path: Option<PathBuf>,
    state_directory_state: StateDirectoryState,
}

impl NativeStatus {
    /// Returns the permission behavior of the native host.
    #[must_use]
    pub const fn permission_mode(&self) -> PermissionMode {
        self.permission_mode
    }

    /// Returns the resolved configuration file path, when available.
    #[must_use]
    pub fn config_file_path(&self) -> Option<&Path> {
        self.config_file_path.as_deref()
    }

    /// Returns the observed configuration file state.
    #[must_use]
    pub const fn config_file_state(&self) -> ConfigFileState {
        self.config_file_state
    }

    /// Returns the resolved state directory path, when available.
    #[must_use]
    pub fn state_directory_path(&self) -> Option<&Path> {
        self.state_directory_path.as_deref()
    }

    /// Returns the observed state directory state.
    #[must_use]
    pub const fn state_directory_state(&self) -> StateDirectoryState {
        self.state_directory_state
    }
}

impl fmt::Debug for NativeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStatus")
            .field("permission_mode", &self.permission_mode)
            .field("has_config_file_path", &self.config_file_path.is_some())
            .field("config_file_state", &self.config_file_state)
            .field(
                "has_state_directory_path",
                &self.state_directory_path.is_some(),
            )
            .field("state_directory_state", &self.state_directory_state)
            .finish()
    }
}

/// Resolves and inspects native configuration and state without modifying them.
#[must_use]
pub fn inspect_native_status(environment: &NativeEnvironment) -> NativeStatus {
    let config_file_path = resolve_config_file(environment);
    let state_directory_path = resolve_state_directory(environment);

    let (config_file_path, config_file_state) = match config_file_path {
        ResolvedPath::Path(path) => {
            let state = inspect_config_file(&path);
            (Some(path), state)
        }
        ResolvedPath::Unavailable => (None, ConfigFileState::Unavailable),
        ResolvedPath::InvalidEnvironment => (None, ConfigFileState::InvalidEnvironment),
    };
    let (state_directory_path, state_directory_state) = match state_directory_path {
        ResolvedPath::Path(path) => {
            let state = inspect_state_directory(&path);
            (Some(path), state)
        }
        ResolvedPath::Unavailable => (None, StateDirectoryState::Unavailable),
        ResolvedPath::InvalidEnvironment => (None, StateDirectoryState::InvalidEnvironment),
    };

    NativeStatus {
        permission_mode: PermissionMode::Ask,
        config_file_path,
        config_file_state,
        state_directory_path,
        state_directory_state,
    }
}

/// Captures the process environment and inspects native configuration and state.
#[must_use]
pub fn inspect_process_status() -> NativeStatus {
    inspect_native_status(&NativeEnvironment::from_process())
}

enum ResolvedPath {
    Path(PathBuf),
    Unavailable,
    InvalidEnvironment,
}

fn resolve_config_file(environment: &NativeEnvironment) -> ResolvedPath {
    resolve_root(
        environment.xdg_config_home.as_deref(),
        environment.home.as_deref(),
        &[".config"],
    )
    .map(|root| root.join(STATE_NAMESPACE).join(CONFIG_FILE_NAME))
}

fn resolve_state_directory(environment: &NativeEnvironment) -> ResolvedPath {
    resolve_root(
        environment.xdg_state_home.as_deref(),
        environment.home.as_deref(),
        &[".local", "state"],
    )
    .map(|root| root.join(STATE_NAMESPACE))
}

fn resolve_root(
    selected_xdg: Option<&OsStr>,
    home: Option<&OsStr>,
    home_suffix: &[&str],
) -> ResolvedPath {
    if let Some(root) = nonempty(selected_xdg) {
        return validate_root(root);
    }

    let Some(home) = nonempty(home) else {
        return ResolvedPath::Unavailable;
    };
    validate_root(home).map(|root| home_suffix.iter().fold(root, |path, part| path.join(part)))
}

fn nonempty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

fn validate_root(root: &OsStr) -> ResolvedPath {
    let path = Path::new(root);
    if root.to_str().is_none() || !path.is_absolute() {
        ResolvedPath::InvalidEnvironment
    } else {
        ResolvedPath::Path(path.to_path_buf())
    }
}

impl ResolvedPath {
    fn map(self, operation: impl FnOnce(PathBuf) -> PathBuf) -> Self {
        match self {
            Self::Path(path) => Self::Path(operation(path)),
            Self::Unavailable => Self::Unavailable,
            Self::InvalidEnvironment => Self::InvalidEnvironment,
        }
    }
}

fn inspect_config_file(path: &Path) -> ConfigFileState {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_file() => ConfigFileState::File,
        Ok(_) => ConfigFileState::NotFile,
        Err(error) if error.kind() == io::ErrorKind::NotFound => ConfigFileState::Missing,
        Err(_) => ConfigFileState::Inaccessible,
    }
}

fn inspect_state_directory(path: &Path) -> StateDirectoryState {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_dir() => StateDirectoryState::Directory,
        Ok(_) => StateDirectoryState::NotDirectory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => StateDirectoryState::Missing,
        Err(_) => StateDirectoryState::Inaccessible,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIG_FILE_NAME, ConfigFileState, NativeEnvironment, PermissionMode, STATE_NAMESPACE,
        SUPPORTED_CORE_API_VERSION, StateDirectoryState, inspect_native_status,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[derive(Debug)]
    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(test_name: &str) -> Self {
            let base = std::env::temp_dir().join("machine-god-native-tests");
            fs::create_dir_all(&base).expect("failed to create native test base directory");
            loop {
                let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!("{}-{test_name}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("failed to create test directory {path:?}: {error}"),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("failed to remove test directory {:?}: {error}", self.path);
            }
        }
    }

    fn environment(
        xdg_config_home: Option<&Path>,
        xdg_state_home: Option<&Path>,
        home: Option<&Path>,
    ) -> NativeEnvironment {
        NativeEnvironment::new(
            xdg_config_home.map(Path::as_os_str).map(OsString::from),
            xdg_state_home.map(Path::as_os_str).map(OsString::from),
            home.map(Path::as_os_str).map(OsString::from),
        )
    }

    #[test]
    fn compatibility_version_is_deliberately_current() {
        assert_eq!(SUPPORTED_CORE_API_VERSION, machine_god_core::API_VERSION);
    }

    #[test]
    fn public_names_and_stable_strings_are_deliberate() {
        assert_eq!(STATE_NAMESPACE, "machine-god");
        assert_eq!(CONFIG_FILE_NAME, "config.json");
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);
        assert_eq!(PermissionMode::Ask.as_str(), "ask");

        let config_states = [
            (ConfigFileState::File, "file"),
            (ConfigFileState::Missing, "missing"),
            (ConfigFileState::NotFile, "not_file"),
            (ConfigFileState::Inaccessible, "inaccessible"),
            (ConfigFileState::Unavailable, "unavailable"),
            (ConfigFileState::InvalidEnvironment, "invalid_environment"),
        ];
        for (state, expected) in config_states {
            assert_eq!(state.as_str(), expected);
        }

        let directory_states = [
            (StateDirectoryState::Directory, "directory"),
            (StateDirectoryState::Missing, "missing"),
            (StateDirectoryState::NotDirectory, "not_directory"),
            (StateDirectoryState::Inaccessible, "inaccessible"),
            (StateDirectoryState::Unavailable, "unavailable"),
            (
                StateDirectoryState::InvalidEnvironment,
                "invalid_environment",
            ),
        ];
        for (state, expected) in directory_states {
            assert_eq!(state.as_str(), expected);
        }
    }

    #[test]
    fn xdg_roots_take_precedence_and_paths_have_expected_shapes() {
        let temporary = TestDirectory::new("xdg-precedence");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let home = temporary.path().join("ignored-home");
        let status = inspect_native_status(&environment(
            Some(&config_root),
            Some(&state_root),
            Some(&home),
        ));

        assert_eq!(status.permission_mode(), PermissionMode::Ask);
        assert_eq!(
            status.config_file_path(),
            Some(config_root.join("machine-god/config.json").as_path())
        );
        assert_eq!(
            status.state_directory_path(),
            Some(state_root.join("machine-god").as_path())
        );
        assert_eq!(status.config_file_state(), ConfigFileState::Missing);
        assert_eq!(status.state_directory_state(), StateDirectoryState::Missing);
        assert!(!status.config_file_path().unwrap().starts_with(&home));
        assert!(!status.state_directory_path().unwrap().starts_with(&home));
    }

    #[test]
    fn config_and_state_resolve_their_roots_independently() {
        let temporary = TestDirectory::new("independent-roots");
        let state_root = temporary.path().join("state-root");
        let home = temporary.path().join("home");
        let status = inspect_native_status(&environment(None, Some(&state_root), Some(&home)));

        assert_eq!(
            status.config_file_path(),
            Some(home.join(".config/machine-god/config.json").as_path())
        );
        assert_eq!(
            status.state_directory_path(),
            Some(state_root.join("machine-god").as_path())
        );
    }

    #[test]
    fn empty_xdg_values_fall_back_to_home() {
        let temporary = TestDirectory::new("empty-xdg");
        let home = temporary.path().join("home");
        let status = inspect_native_status(&NativeEnvironment::new(
            Some(OsString::new()),
            Some(OsString::new()),
            Some(home.as_os_str().to_owned()),
        ));

        assert_eq!(
            status.config_file_path(),
            Some(home.join(".config/machine-god/config.json").as_path())
        );
        assert_eq!(
            status.state_directory_path(),
            Some(home.join(".local/state/machine-god").as_path())
        );
    }

    #[test]
    fn relative_xdg_values_are_invalid_without_home_fallback() {
        let temporary = TestDirectory::new("relative-xdg");
        let home = temporary.path().join("home");
        let status = inspect_native_status(&NativeEnvironment::new(
            Some(OsString::from("relative-config")),
            Some(OsString::from("relative-state")),
            Some(home.as_os_str().to_owned()),
        ));

        assert_eq!(status.config_file_path(), None);
        assert_eq!(
            status.config_file_state(),
            ConfigFileState::InvalidEnvironment
        );
        assert_eq!(status.state_directory_path(), None);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::InvalidEnvironment
        );
    }

    #[test]
    fn missing_or_empty_home_makes_both_paths_unavailable() {
        for home in [None, Some(OsString::new())] {
            let status = inspect_native_status(&NativeEnvironment::new(None, None, home));

            assert_eq!(status.config_file_path(), None);
            assert_eq!(status.config_file_state(), ConfigFileState::Unavailable);
            assert_eq!(status.state_directory_path(), None);
            assert_eq!(
                status.state_directory_state(),
                StateDirectoryState::Unavailable
            );
        }
    }

    #[test]
    fn relative_home_is_invalid() {
        let status = inspect_native_status(&NativeEnvironment::new(
            None,
            None,
            Some(OsString::from("relative-home")),
        ));

        assert_eq!(
            status.config_file_state(),
            ConfigFileState::InvalidEnvironment
        );
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::InvalidEnvironment
        );
    }

    #[test]
    fn regular_file_and_directory_are_recognized_without_parsing() {
        let temporary = TestDirectory::new("expected-kinds");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let config_file = config_root.join("machine-god/config.json");
        let state_directory = state_root.join("machine-god");
        fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        fs::create_dir_all(&state_directory).unwrap();
        fs::write(&config_file, b"this is deliberately not parsed as JSON").unwrap();

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::File);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::Directory
        );
    }

    #[test]
    fn wrong_path_kinds_are_reported() {
        let temporary = TestDirectory::new("wrong-kinds");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let config_file = config_root.join("machine-god/config.json");
        let state_directory = state_root.join("machine-god");
        fs::create_dir_all(&config_file).unwrap();
        fs::create_dir_all(state_directory.parent().unwrap()).unwrap();
        fs::write(&state_directory, b"not a directory").unwrap();

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::NotFile);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::NotDirectory
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_symlinks_are_not_followed() {
        use std::os::unix::fs::symlink;

        let temporary = TestDirectory::new("symlinks");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let targets = temporary.path().join("targets");
        let target_file = targets.join("config.json");
        let target_directory = targets.join("state");
        fs::create_dir_all(&target_directory).unwrap();
        fs::write(&target_file, b"{}").unwrap();

        let config_file = config_root.join("machine-god/config.json");
        let state_directory = state_root.join("machine-god");
        fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        fs::create_dir_all(state_directory.parent().unwrap()).unwrap();
        symlink(&target_file, &config_file).unwrap();
        symlink(&target_directory, &state_directory).unwrap();

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::NotFile);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::NotDirectory
        );
    }

    #[cfg(unix)]
    #[test]
    fn metadata_errors_other_than_not_found_are_inaccessible() {
        let temporary = TestDirectory::new("inaccessible");
        let too_long = "x".repeat(300);
        let config_root = temporary.path().join(&too_long);
        let state_root = temporary.path().join(too_long);

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::Inaccessible);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::Inaccessible
        );
    }

    #[test]
    fn inspection_does_not_create_resolved_paths_or_ancestors() {
        let temporary = TestDirectory::new("no-write");
        let config_root = temporary.path().join("absent-config-root");
        let state_root = temporary.path().join("absent-state-root");

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::Missing);
        assert_eq!(status.state_directory_state(), StateDirectoryState::Missing);
        assert!(!config_root.exists());
        assert!(!state_root.exists());
    }

    #[test]
    fn debug_output_redacts_environment_values_and_resolved_paths() {
        let temporary = TestDirectory::new("debug-redaction");
        let secret = temporary.path().join("do-not-print-this-secret");
        let environment = environment(Some(&secret), Some(&secret), Some(&secret));
        let environment_debug = format!("{environment:?}");
        assert!(environment_debug.contains("has_xdg_config_home: true"));
        assert!(environment_debug.contains("has_xdg_state_home: true"));
        assert!(environment_debug.contains("has_home: true"));
        assert!(!environment_debug.contains("do-not-print-this-secret"));

        let status = inspect_native_status(&environment);
        let status_debug = format!("{status:?}");
        assert!(status_debug.contains("has_config_file_path: true"));
        assert!(status_debug.contains("has_state_directory_path: true"));
        assert!(status_debug.contains("config_file_state: Missing"));
        assert!(!status_debug.contains("do-not-print-this-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_selected_roots_are_invalid_without_fallback() {
        use std::os::unix::ffi::OsStringExt;

        let temporary = TestDirectory::new("non-unicode");
        let mut invalid_bytes = temporary.path().as_os_str().as_encoded_bytes().to_vec();
        invalid_bytes.extend_from_slice(b"/invalid-");
        invalid_bytes.push(0xff);
        let invalid = OsString::from_vec(invalid_bytes);
        let home = temporary.path().join("valid-home");
        let status = inspect_native_status(&NativeEnvironment::new(
            Some(invalid.clone()),
            Some(invalid),
            Some(home.as_os_str().to_owned()),
        ));

        assert_eq!(status.config_file_path(), None);
        assert_eq!(
            status.config_file_state(),
            ConfigFileState::InvalidEnvironment
        );
        assert_eq!(status.state_directory_path(), None);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::InvalidEnvironment
        );
    }
}
