//! Effect-free catalog and command-envelope routing for the five native slash categories.
//!
//! Payload validation and execution belong to the command handlers, not this catalog.

use std::fmt;
use std::iter::FusedIterator;

/// Inclusive UTF-8 byte bound checked before command/submission inspection.
pub const MAX_NATIVE_SLASH_INPUT_BYTES: usize = 65_536;
/// Inclusive UTF-8 byte bound checked before completion or help-query inspection.
pub const MAX_NATIVE_SLASH_QUERY_BYTES: usize = 4096;

/// The twenty pinned primary commands in the five native categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSlashCommand {
    Help,
    Clear,
    New,
    Reset,
    Resume,
    Continue,
    Rename,
    Status,
    Model,
    Models,
    Permissions,
    Allowlist,
    Undo,
    Copy,
    Compact,
    Fast,
    Sandbox,
    Workspace,
    Version,
    Quit,
}

/// Help presentation categories, in pinned display order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSlashCategory {
    General,
    Session,
    Model,
    Security,
    Workspace,
}

impl NativeSlashCategory {
    /// Returns the pinned human-readable category label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Session => "Session",
            Self::Model => "Model",
            Self::Security => "Security",
            Self::Workspace => "Workspace",
        }
    }
}

const CATEGORIES: [NativeSlashCategory; 5] = [
    NativeSlashCategory::General,
    NativeSlashCategory::Session,
    NativeSlashCategory::Model,
    NativeSlashCategory::Security,
    NativeSlashCategory::Workspace,
];

/// Static catalog metadata, not a declaration that runtime effects are implemented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSlashSpec {
    pub command: NativeSlashCommand,
    pub token: &'static str,
    pub aliases: &'static [&'static str],
    /// Pinned help label. More detailed handler grammar may have additional forms.
    pub grammar: &'static str,
    pub description: &'static str,
    pub category: NativeSlashCategory,
    pub accepts_payload: bool,
    pub show_in_welcome: bool,
    /// `/model` only needs a credential when its payload is nonempty.
    pub requires_prompt_credential: bool,
}

macro_rules! spec {
    ($kind:ident, $token:literal, $grammar:literal, $description:literal,
     $category:ident, $payload:literal, $welcome:literal, $credential:literal) => {
        NativeSlashSpec {
            command: NativeSlashCommand::$kind,
            token: $token,
            aliases: &[],
            grammar: $grammar,
            description: $description,
            category: NativeSlashCategory::$category,
            accepts_payload: $payload,
            show_in_welcome: $welcome,
            requires_prompt_credential: $credential,
        }
    };
}

// Preserve the relative order of the pinned complete registry, not category order.
static REGISTRY: [NativeSlashSpec; 20] = [
    spec!(
        Help,
        "/help",
        "/help",
        "show available slash commands",
        General,
        false,
        true,
        false
    ),
    spec!(
        Clear,
        "/clear",
        "/clear",
        "start a fresh session and keep background processes",
        General,
        false,
        true,
        false
    ),
    spec!(
        New,
        "/new",
        "/new",
        "start a fresh session",
        Session,
        false,
        true,
        false
    ),
    spec!(
        Reset,
        "/reset",
        "/reset",
        "reset the current session context",
        Session,
        false,
        false,
        false
    ),
    spec!(
        Resume,
        "/resume",
        "/resume",
        "resume a saved session",
        Session,
        false,
        false,
        false
    ),
    spec!(
        Continue,
        "/continue",
        "/continue",
        "continue a paused model response",
        Session,
        false,
        false,
        true
    ),
    spec!(
        Rename,
        "/rename",
        "/rename <title>",
        "rename the current session",
        Session,
        true,
        false,
        false
    ),
    spec!(
        Status,
        "/status",
        "/status",
        "show runtime configuration",
        General,
        false,
        true,
        false
    ),
    spec!(
        Model,
        "/model",
        "/model <id-or-query>",
        "choose what model and reasoning effort to use",
        Model,
        true,
        false,
        true
    ),
    spec!(
        Models,
        "/models",
        "/models",
        "browse available models",
        Model,
        false,
        false,
        false
    ),
    spec!(
        Permissions,
        "/permissions",
        "/permissions [ask|auto|yolo|reset]",
        "choose what fx is allowed to do",
        Security,
        true,
        true,
        false
    ),
    spec!(
        Allowlist,
        "/allowlist",
        "/allowlist [view [effective|local|user]|[local|user] add|remove|reset ...]",
        "manage trusted commands, tools, and URLs",
        Security,
        true,
        true,
        false
    ),
    spec!(
        Undo,
        "/undo",
        "/undo",
        "undo the latest tracked file operation",
        Session,
        false,
        false,
        false
    ),
    spec!(
        Copy,
        "/copy",
        "/copy",
        "copy the last assistant response",
        Session,
        false,
        false,
        false
    ),
    spec!(
        Compact,
        "/compact",
        "/compact",
        "compact older conversation turns",
        Session,
        false,
        false,
        false
    ),
    spec!(
        Fast,
        "/fast",
        "/fast",
        "toggle Fast mode when supported",
        Model,
        false,
        false,
        false
    ),
    spec!(
        Sandbox,
        "/sandbox",
        "/sandbox [os|none]",
        "choose command sandbox behavior",
        Security,
        true,
        false,
        false
    ),
    spec!(
        Workspace,
        "/workspace",
        "/workspace [list|add PATH|remove PATH|clear]",
        "manage additional workspace directories",
        Workspace,
        true,
        true,
        false
    ),
    spec!(
        Version,
        "/version",
        "/version",
        "show the fx version",
        General,
        false,
        false,
        false
    ),
    NativeSlashSpec {
        aliases: &["/exit"],
        ..spec!(
            Quit,
            "/quit",
            "/quit",
            "exit the interactive shell",
            General,
            false,
            true,
            false
        )
    },
];

/// Returns the twenty static entries in pinned registry order, without allocation.
#[must_use]
pub fn native_slash_registry() -> &'static [NativeSlashSpec; 20] {
    &REGISTRY
}

impl NativeSlashCommand {
    /// Returns metadata for this primary command.
    #[must_use]
    pub fn spec(self) -> &'static NativeSlashSpec {
        // The exhaustive enum and registry have identical order, tested below.
        &REGISTRY[self as usize]
    }
}

/// A command-envelope match whose payload is borrowed, not parsed or executed.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeSlashInvocation<'a> {
    pub command: NativeSlashCommand,
    /// Original payload bytes with surrounding SP/TAB removed; no unquoting.
    pub payload: &'a str,
}

impl NativeSlashInvocation<'_> {
    /// Whether the pinned submission preflight requests a prompt credential.
    #[must_use]
    pub fn requires_prompt_credential(self) -> bool {
        self.command.spec().requires_prompt_credential
            && (self.command != NativeSlashCommand::Model
                || !self.payload.trim_matches(is_submit_space).is_empty())
    }
}

impl fmt::Debug for NativeSlashInvocation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSlashInvocation")
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// Raw routing result; `Valid` validates only the envelope, never action arguments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSlashRoute<'a> {
    /// Not a registered command in this catalog (including leading whitespace).
    NotLocal,
    /// Registered command token, but its separator or no-payload envelope is invalid.
    KnownInvalid {
        command: NativeSlashCommand,
    },
    Valid(NativeSlashInvocation<'a>),
}

/// Fixed redacted resource-limit error. No input bytes are retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSlashInputError {
    InputTooLong,
    QueryTooLong,
}

impl fmt::Display for NativeSlashInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputTooLong => "slash input exceeds byte limit",
            Self::QueryTooLong => "slash query exceeds byte limit",
        })
    }
}

impl std::error::Error for NativeSlashInputError {}

/// Parses the pinned raw command envelope without trimming leading whitespace.
///
/// # Errors
/// Returns `InputTooLong` before scanning input above the inclusive input bound.
pub fn route_native_slash(input: &str) -> Result<NativeSlashRoute<'_>, NativeSlashInputError> {
    check_input(input)?;
    Ok(route_unchecked(input))
}

fn route_unchecked(input: &str) -> NativeSlashRoute<'_> {
    let Some(spec) = known_command(input) else {
        return NativeSlashRoute::NotLocal;
    };
    let token_end = input
        .bytes()
        .position(is_ascii_space)
        .unwrap_or(input.len());
    let tail = &input[token_end..];
    if (!tail.is_empty() && !tail.starts_with([' ', '\t']))
        || (!spec.accepts_payload && !tail.trim_matches(is_horizontal_space).is_empty())
    {
        return NativeSlashRoute::KnownInvalid {
            command: spec.command,
        };
    }
    NativeSlashRoute::Valid(NativeSlashInvocation {
        command: spec.command,
        payload: tail.trim_matches(is_horizontal_space),
    })
}

fn known_command(input: &str) -> Option<&'static NativeSlashSpec> {
    if !input.starts_with('/') {
        return None;
    }
    let end = input
        .bytes()
        .position(is_ascii_space)
        .unwrap_or(input.len());
    let token = &input[..end];
    REGISTRY
        .iter()
        .find(|spec| spec.token == token || spec.aliases.contains(&token))
}

fn check_input(input: &str) -> Result<(), NativeSlashInputError> {
    if input.len() > MAX_NATIVE_SLASH_INPUT_BYTES {
        return Err(NativeSlashInputError::InputTooLong);
    }
    Ok(())
}

fn check_query(query: &str) -> Result<(), NativeSlashInputError> {
    if query.len() > MAX_NATIVE_SLASH_QUERY_BYTES {
        return Err(NativeSlashInputError::QueryTooLong);
    }
    Ok(())
}

fn is_horizontal_space(c: char) -> bool {
    matches!(c, ' ' | '\t')
}

fn is_submit_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn is_ascii_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// Explicit UI facts; this module never reads or owns picker state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeSlashSubmissionContext {
    pub slash_picker_active: bool,
    pub slash_picker_dismissed: bool,
    /// `Some(index)` means the host has a visible slash completion menu.
    pub visible_completion_index: Option<usize>,
}

/// Local submission decision after left trimming and optional completion resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSlashSubmission<'a> {
    NotLocal,
    /// Bare unknown slash word under the explicitly active, non-dismissed picker.
    UnknownLocal,
    KnownInvalid {
        command: NativeSlashCommand,
    },
    Valid(NativeSlashInvocation<'a>),
}

/// Resolves only slash submission policy; does not submit prompts, expand pastes,
/// inspect attachment entities, or decide whether a UI picker is visible.
///
/// # Errors
/// Returns `InputTooLong` before inspecting an oversized submission.
pub fn resolve_native_slash_submission(
    input: &str,
    context: NativeSlashSubmissionContext,
) -> Result<NativeSlashSubmission<'_>, NativeSlashInputError> {
    check_input(input)?;
    let mut text = input.trim_start_matches(is_submit_space);
    if let Some(index) = context.visible_completion_index {
        // Submission is already bounded independently of explicit query APIs.
        // Preserve argument-completion whitespace normalization even for a long
        // submission instead of imposing the smaller help/query bound here.
        let mut completions = completions_unchecked(text);
        let count = completions.clone().count();
        if count != 0
            && let Some(selected) = completions.nth(index % count)
        {
            text = selected.replacement;
        }
    }
    Ok(match route_unchecked(text) {
        NativeSlashRoute::Valid(invocation) => NativeSlashSubmission::Valid(invocation),
        NativeSlashRoute::KnownInvalid { command } => {
            NativeSlashSubmission::KnownInvalid { command }
        }
        NativeSlashRoute::NotLocal => {
            let bare = text.starts_with('/')
                && !text.chars().any(is_submit_space)
                && !text[1..].contains('/');
            if bare
                && context.slash_picker_active
                && !context.slash_picker_dismissed
                && completions_unchecked(text).next().is_none()
            {
                NativeSlashSubmission::UnknownLocal
            } else {
                NativeSlashSubmission::NotLocal
            }
        }
    })
}

/// One static completion. Argument rows have no category or description.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSlashCompletion {
    pub command: NativeSlashCommand,
    pub replacement: &'static str,
    pub label: &'static str,
    pub description: Option<&'static str>,
    pub category: Option<NativeSlashCategory>,
    /// Whether Tab should keep the command open for further arguments.
    pub has_args: bool,
}

#[derive(Clone)]
enum CompletionState<'a> {
    Commands {
        query: &'a str,
        rank: u8,
        index: usize,
    },
    Arguments {
        query: &'a str,
        options: &'static [&'static str],
        offset: usize,
        index: usize,
        command: NativeSlashCommand,
    },
    Done,
}

/// Allocation-free completion iterator; at most twenty rows are returned.
#[derive(Clone)]
pub struct NativeSlashCompletions<'a> {
    state: CompletionState<'a>,
}

impl fmt::Debug for NativeSlashCompletions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSlashCompletions")
            .finish_non_exhaustive()
    }
}

/// Returns completions for a raw prefix (does not left-trim it).
///
/// # Errors
/// Returns `QueryTooLong` before scanning an oversized query.
pub fn native_slash_completions(
    prefix: &str,
) -> Result<NativeSlashCompletions<'_>, NativeSlashInputError> {
    check_query(prefix)?;
    Ok(completions_unchecked(prefix))
}

/// Extracts a composer completion prefix, separately from raw parsing.
/// No-argument commands followed by whitespace yield `None`, even if malformed.
///
/// # Errors
/// Returns `QueryTooLong` before scanning an oversized composer input.
pub fn native_slash_completion_prefix(input: &str) -> Result<Option<&str>, NativeSlashInputError> {
    check_query(input)?;
    let prefix = input.trim_start_matches(is_submit_space);
    if !prefix.starts_with('/') {
        return Ok(None);
    }
    if let Some(end) = prefix.find(is_submit_space)
        && let Some(spec) = REGISTRY
            .iter()
            .find(|spec| spec.token == &prefix[..end] || spec.aliases.contains(&&prefix[..end]))
        && !spec.accepts_payload
    {
        return Ok(None);
    }
    Ok(Some(prefix))
}

fn completions_unchecked(prefix: &str) -> NativeSlashCompletions<'_> {
    let state = if let Some(query) = argument_prefix(prefix, "/allowlist", false) {
        let args = allowlist_arguments(query);
        CompletionState::Arguments {
            query: args.query,
            options: args.options,
            offset: args.offset,
            index: 0,
            command: NativeSlashCommand::Allowlist,
        }
    } else if let Some(query) = argument_prefix(prefix, "/sandbox", true) {
        argument_state(
            query,
            &["/sandbox os", "/sandbox none"],
            "/sandbox ".len(),
            NativeSlashCommand::Sandbox,
        )
    } else if let Some(query) = argument_prefix(prefix, "/permissions", true) {
        argument_state(
            query,
            &[
                "/permissions ask",
                "/permissions auto",
                "/permissions remember",
                "/permissions revoke",
                "/permissions yolo",
                "/permissions reset",
            ],
            "/permissions ".len(),
            NativeSlashCommand::Permissions,
        )
    } else if let Some(query) = argument_prefix(prefix, "/workspace", true) {
        argument_state(
            query,
            &[
                "/workspace list",
                "/workspace add",
                "/workspace remove",
                "/workspace clear",
            ],
            "/workspace ".len(),
            NativeSlashCommand::Workspace,
        )
    } else if prefix.starts_with('/') {
        CompletionState::Commands {
            query: prefix,
            rank: 0,
            index: 0,
        }
    } else {
        CompletionState::Done
    };
    NativeSlashCompletions { state }
}

fn argument_state<'a>(
    query: &'a str,
    options: &'static [&'static str],
    offset: usize,
    command: NativeSlashCommand,
) -> CompletionState<'a> {
    CompletionState::Arguments {
        query,
        options,
        offset,
        index: 0,
        command,
    }
}

fn argument_prefix<'a>(prefix: &'a str, command: &str, trim_end: bool) -> Option<&'a str> {
    let rest = prefix.strip_prefix(command)?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(if trim_end {
        rest.trim_matches(is_horizontal_space)
    } else {
        rest.trim_start_matches(is_horizontal_space)
    })
}

fn command_rank(command: &str, query: &str) -> Option<u8> {
    if command == query {
        Some(0)
    } else if command.starts_with(query) {
        Some(1)
    } else if query.len() > 1 && command[1..].contains(&query[1..]) {
        Some(2)
    } else {
        None
    }
}

fn best_command_match(spec: &NativeSlashSpec, query: &str) -> Option<(&'static str, u8)> {
    let mut best = command_rank(spec.token, query).map(|rank| (spec.token, rank));
    for alias in spec.aliases {
        if let Some(rank) = command_rank(alias, query)
            && best.is_none_or(|(_, current)| rank < current)
        {
            best = Some((alias, rank));
        }
    }
    best
}

impl Iterator for NativeSlashCompletions<'_> {
    type Item = NativeSlashCompletion;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.state {
            CompletionState::Commands { query, rank, index } => {
                while *rank < 3 {
                    while let Some(spec) = REGISTRY.get(*index) {
                        *index += 1;
                        if let Some((replacement, found_rank)) = best_command_match(spec, query)
                            && found_rank == *rank
                        {
                            return Some(NativeSlashCompletion {
                                command: spec.command,
                                replacement,
                                label: replacement,
                                description: Some(spec.description),
                                category: Some(spec.category),
                                has_args: spec.accepts_payload,
                            });
                        }
                    }
                    *index = 0;
                    *rank += 1;
                }
            }
            CompletionState::Arguments {
                query,
                options,
                offset,
                index,
                command,
            } => {
                while let Some(replacement) = options.get(*index) {
                    *index += 1;
                    let label = &replacement[*offset..];
                    if starts_ignore_ascii_case(label, query) {
                        return Some(NativeSlashCompletion {
                            command: *command,
                            replacement,
                            label,
                            description: None,
                            category: None,
                            has_args: completion_has_args(replacement),
                        });
                    }
                }
            }
            CompletionState::Done => {}
        }
        self.state = CompletionState::Done;
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            0,
            Some(if matches!(self.state, CompletionState::Done) {
                0
            } else {
                20
            }),
        )
    }
}

impl FusedIterator for NativeSlashCompletions<'_> {}

fn starts_ignore_ascii_case(value: &str, query: &str) -> bool {
    value
        .as_bytes()
        .get(..query.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(query.as_bytes()))
}

struct AllowlistTables {
    actions: &'static [&'static str],
    add: &'static [&'static str],
    remove: &'static [&'static str],
    reset: &'static [&'static str],
    add_tool: &'static [&'static str],
    remove_tool: &'static [&'static str],
    base_len: usize,
}

macro_rules! tool_rows {
    ($prefix:literal) => {
        &[
            concat!($prefix, "read_file"),
            concat!($prefix, "write_file"),
            concat!($prefix, "edit_file"),
            concat!($prefix, "list_files"),
            concat!($prefix, "glob_files"),
            concat!($prefix, "grep_files"),
            concat!($prefix, "open_file"),
            concat!($prefix, "create_folder"),
            concat!($prefix, "rename_file"),
            concat!($prefix, "copy_file"),
            concat!($prefix, "skill"),
            concat!($prefix, "install_skill"),
            concat!($prefix, "subagent"),
        ]
    };
}

macro_rules! allowlist_tables {
    ($prefix:literal, $add_prefix:literal, $remove_prefix:literal) => {
        AllowlistTables {
            actions: &[
                concat!($prefix, "add"),
                concat!($prefix, "remove"),
                concat!($prefix, "reset"),
            ],
            add: &[
                concat!($prefix, "add command"),
                concat!($prefix, "add tool"),
                concat!($prefix, "add url"),
                concat!($prefix, "add web-fetch-domain"),
            ],
            remove: &[
                concat!($prefix, "remove command"),
                concat!($prefix, "remove tool"),
                concat!($prefix, "remove url"),
                concat!($prefix, "remove web-fetch-domain"),
            ],
            reset: &[
                concat!($prefix, "reset commands"),
                concat!($prefix, "reset tools"),
                concat!($prefix, "reset urls"),
                concat!($prefix, "reset web-fetch-domains"),
                concat!($prefix, "reset all"),
            ],
            add_tool: tool_rows!($add_prefix),
            remove_tool: tool_rows!($remove_prefix),
            base_len: $prefix.len(),
        }
    };
}

static ALLOWLIST_DEFAULT: AllowlistTables = allowlist_tables!(
    "/allowlist ",
    "/allowlist add tool ",
    "/allowlist remove tool "
);
static ALLOWLIST_LOCAL: AllowlistTables = allowlist_tables!(
    "/allowlist local ",
    "/allowlist local add tool ",
    "/allowlist local remove tool "
);
static ALLOWLIST_USER: AllowlistTables = allowlist_tables!(
    "/allowlist user ",
    "/allowlist user add tool ",
    "/allowlist user remove tool "
);
const ALLOWLIST_ACTIONS: &[&str] = &[
    "/allowlist view",
    "/allowlist add",
    "/allowlist remove",
    "/allowlist reset",
    "/allowlist local",
    "/allowlist user",
];
const ALLOWLIST_VIEWS: &[&str] = &[
    "/allowlist view effective",
    "/allowlist view local",
    "/allowlist view user",
];

struct ArgumentRows<'a> {
    options: &'static [&'static str],
    offset: usize,
    query: &'a str,
}

fn split_argument(query: &str) -> (&str, Option<&str>) {
    let query = query.trim_start_matches(is_horizontal_space);
    query
        .find(is_horizontal_space)
        .map_or((query, None), |end| {
            (
                &query[..end],
                Some(query[end..].trim_start_matches(is_horizontal_space)),
            )
        })
}

fn allowlist_arguments(query: &str) -> ArgumentRows<'_> {
    let (word, rest) = split_argument(query);
    if let Some(rest) = rest {
        if word.eq_ignore_ascii_case("view") {
            return ArgumentRows {
                options: ALLOWLIST_VIEWS,
                offset: "/allowlist view ".len(),
                query: rest,
            };
        }
        if word.eq_ignore_ascii_case("local") {
            return scoped_allowlist_arguments(rest, &ALLOWLIST_LOCAL);
        }
        if word.eq_ignore_ascii_case("user") {
            return scoped_allowlist_arguments(rest, &ALLOWLIST_USER);
        }
        return action_arguments(word, rest, &ALLOWLIST_DEFAULT);
    }
    ArgumentRows {
        options: ALLOWLIST_ACTIONS,
        offset: "/allowlist ".len(),
        query: word,
    }
}

fn scoped_allowlist_arguments<'a>(query: &'a str, tables: &AllowlistTables) -> ArgumentRows<'a> {
    let (word, rest) = split_argument(query);
    rest.map_or(
        ArgumentRows {
            options: tables.actions,
            offset: tables.base_len,
            query: word,
        },
        |rest| action_arguments(word, rest, tables),
    )
}

fn action_arguments<'a>(
    action: &str,
    query: &'a str,
    tables: &AllowlistTables,
) -> ArgumentRows<'a> {
    let (word, rest) = split_argument(query);
    let add = action.eq_ignore_ascii_case("add");
    let remove = action.eq_ignore_ascii_case("remove");
    if add || remove {
        let offset = tables.base_len + if add { "add ".len() } else { "remove ".len() };
        if word.eq_ignore_ascii_case("tool")
            && let Some(rest) = rest
        {
            return ArgumentRows {
                options: if add {
                    tables.add_tool
                } else {
                    tables.remove_tool
                },
                offset: offset + "tool ".len(),
                query: rest,
            };
        }
        return ArgumentRows {
            options: if add { tables.add } else { tables.remove },
            offset,
            query,
        };
    }
    if action.eq_ignore_ascii_case("reset") {
        return ArgumentRows {
            options: tables.reset,
            offset: tables.base_len + "reset ".len(),
            query,
        };
    }
    ArgumentRows {
        options: &[],
        offset: 0,
        query: "",
    }
}

fn completion_has_args(completion: &str) -> bool {
    if matches!(completion, "/workspace add" | "/workspace remove") {
        return true;
    }
    if ALLOWLIST_ACTIONS.contains(&completion) {
        return true;
    }
    [&ALLOWLIST_DEFAULT, &ALLOWLIST_LOCAL, &ALLOWLIST_USER]
        .iter()
        .any(|tables| {
            tables.actions.contains(&completion)
                || tables.add.contains(&completion)
                || tables.remove.contains(&completion)
        })
}

/// Allocation-free categorized help-search iterator. Query bytes are redacted in Debug.
#[derive(Clone)]
pub struct NativeSlashHelp<'a> {
    query: &'a str,
    category: usize,
    index: usize,
}

impl fmt::Debug for NativeSlashHelp<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSlashHelp").finish_non_exhaustive()
    }
}

/// Searches tokens with AND semantics across command, alias, grammar, description,
/// and category. Matching is ASCII-case-insensitive; no query allocation occurs.
///
/// # Errors
/// Returns `QueryTooLong` before scanning an oversized query.
pub fn native_slash_help(query: &str) -> Result<NativeSlashHelp<'_>, NativeSlashInputError> {
    check_query(query)?;
    Ok(NativeSlashHelp {
        query,
        category: 0,
        index: 0,
    })
}

impl Iterator for NativeSlashHelp<'_> {
    type Item = &'static NativeSlashSpec;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(category) = CATEGORIES.get(self.category) {
            while let Some(spec) = REGISTRY.get(self.index) {
                self.index += 1;
                if spec.category == *category && help_matches(spec, self.query) {
                    return Some(spec);
                }
            }
            self.category += 1;
            self.index = 0;
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            0,
            Some(if self.category == CATEGORIES.len() {
                0
            } else {
                20
            }),
        )
    }
}

impl FusedIterator for NativeSlashHelp<'_> {}

fn help_matches(spec: &NativeSlashSpec, query: &str) -> bool {
    query
        .split(is_submit_space)
        .filter(|token| !token.is_empty())
        .all(|token| {
            [
                spec.token,
                spec.grammar,
                spec.description,
                spec.category.label(),
            ]
            .into_iter()
            .chain(spec.aliases.iter().copied())
            .any(|field| {
                field
                    .as_bytes()
                    .windows(token.len())
                    .any(|window| window.eq_ignore_ascii_case(token.as_bytes()))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(input: &str) -> NativeSlashInvocation<'_> {
        match route_native_slash(input).unwrap() {
            NativeSlashRoute::Valid(invocation) => invocation,
            other => panic!("expected valid envelope, got {other:?}"),
        }
    }

    fn rows(prefix: &str) -> Vec<&'static str> {
        native_slash_completions(prefix)
            .unwrap()
            .map(|row| row.replacement)
            .collect()
    }

    #[test]
    fn slash_commands_registry_preserves_complete_pinned_category_scope_and_order() {
        let expected = [
            "/help",
            "/clear",
            "/new",
            "/reset",
            "/resume",
            "/continue",
            "/rename",
            "/status",
            "/model",
            "/models",
            "/permissions",
            "/allowlist",
            "/undo",
            "/copy",
            "/compact",
            "/fast",
            "/sandbox",
            "/workspace",
            "/version",
            "/quit",
        ];
        assert_eq!(native_slash_registry().map(|spec| spec.token), expected);
        let mut aliases = 0;
        for spec in native_slash_registry() {
            assert_eq!(spec.command.spec(), spec);
            assert_eq!(valid(spec.token).command, spec.command);
            assert_eq!(valid(spec.token).payload, "");
            for alias in spec.aliases {
                aliases += 1;
                assert_eq!(*alias, "/exit");
                assert_eq!(valid(alias).command, NativeSlashCommand::Quit);
            }
        }
        assert_eq!(aliases, 1);
        assert_eq!(
            CATEGORIES.map(|category| REGISTRY
                .iter()
                .filter(|spec| spec.category == category)
                .count()),
            [5, 8, 3, 3, 1]
        );
    }

    #[test]
    fn slash_commands_envelope_payload_acceptance_follows_every_spec() {
        for spec in &REGISTRY {
            for token in std::iter::once(spec.token).chain(spec.aliases.iter().copied()) {
                let with_payload = format!("{token} sample");
                let parsed = route_native_slash(&with_payload).unwrap();
                if spec.accepts_payload {
                    assert_eq!(
                        parsed,
                        NativeSlashRoute::Valid(NativeSlashInvocation {
                            command: spec.command,
                            payload: "sample"
                        })
                    );
                } else {
                    assert_eq!(
                        parsed,
                        NativeSlashRoute::KnownInvalid {
                            command: spec.command
                        }
                    );
                }
                assert_eq!(valid(&format!("{token}\t ")).command, spec.command);
            }
        }
    }

    #[test]
    fn slash_commands_raw_router_preserves_pinned_whitespace_case_and_borrowing() {
        let input = "/model \t claude-opus \t";
        let invocation = valid(input);
        assert_eq!(invocation.payload, "claude-opus");
        assert_eq!(invocation.payload.as_ptr(), input[9..].as_ptr());
        for input in [
            "/model\nopus",
            "/help\n",
            "/help\r",
            "/help\u{b}",
            "/help\u{c}",
            "/copy extra",
            "/help search",
        ] {
            assert!(matches!(
                route_native_slash(input).unwrap(),
                NativeSlashRoute::KnownInvalid { .. }
            ));
        }
        for input in [
            " /model opus",
            "\n/help",
            "/HELP",
            "/Model",
            "/model-helper",
            "/tmp/file",
            "/helpé",
            "hello",
            "/wat",
        ] {
            assert_eq!(
                route_native_slash(input).unwrap(),
                NativeSlashRoute::NotLocal
            );
        }
        assert_eq!(valid("/model \nopus\n").payload, "\nopus\n");
        assert_eq!(valid("/rename one\ttwo").payload, "one\ttwo");
        assert_eq!(
            valid("/allowlist add command \"git *\"").payload,
            "add command \"git *\""
        );
    }

    #[test]
    fn slash_commands_handler_grammar_is_not_mistaken_for_envelope_validation() {
        for input in [
            "/rename",
            "/workspace add",
            "/workspace ADD thing",
            "/sandbox vercel",
            "/permissions remember allow write_file {}",
            "/permissions revoke nope",
            "/allowlist reset",
        ] {
            assert!(matches!(
                route_native_slash(input).unwrap(),
                NativeSlashRoute::Valid(_)
            ));
        }
        assert!(!valid("/model").requires_prompt_credential());
        assert!(!valid("/model \n").requires_prompt_credential());
        assert!(valid("/model opus").requires_prompt_credential());
        assert!(valid("/continue").requires_prompt_credential());
        for spec in &REGISTRY {
            if !matches!(
                spec.command,
                NativeSlashCommand::Model | NativeSlashCommand::Continue
            ) {
                assert!(!valid(spec.token).requires_prompt_credential());
            }
        }
    }

    #[test]
    fn slash_commands_composer_prefix_does_not_change_raw_router() {
        assert_eq!(
            native_slash_completion_prefix("\n   /he").unwrap(),
            Some("/he")
        );
        for input in [
            "/sandbox ",
            "/sandbox\nos",
            "/resume-helper",
            "/not-a-command ",
            "/help\u{b}ignored ",
        ] {
            assert_eq!(native_slash_completion_prefix(input).unwrap(), Some(input));
        }
        for input in ["/resume ", "\n\t/resume\nignored", "/exit\t", "ordinary"] {
            assert_eq!(native_slash_completion_prefix(input).unwrap(), None);
        }
        assert_eq!(
            native_slash_completion_prefix("/resume").unwrap(),
            Some("/resume")
        );
        assert_eq!(
            route_native_slash("\n   /help").unwrap(),
            NativeSlashRoute::NotLocal
        );
        assert_eq!(
            resolve_native_slash_submission("\n   /help", NativeSlashSubmissionContext::default())
                .unwrap(),
            NativeSlashSubmission::Valid(NativeSlashInvocation {
                command: NativeSlashCommand::Help,
                payload: ""
            })
        );
    }

    #[test]
    fn slash_commands_submission_unknown_path_and_picker_rules_are_explicit() {
        let active = NativeSlashSubmissionContext {
            slash_picker_active: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_native_slash_submission("/wat", active).unwrap(),
            NativeSlashSubmission::UnknownLocal
        );
        for input in [
            "/wat payload",
            "/wat ",
            "/tmp/file",
            "/wat\n",
            "/he",
            "/",
            "ordinary",
        ] {
            assert_eq!(
                resolve_native_slash_submission(input, active).unwrap(),
                NativeSlashSubmission::NotLocal
            );
        }
        for context in [
            NativeSlashSubmissionContext::default(),
            NativeSlashSubmissionContext {
                slash_picker_dismissed: true,
                ..active
            },
        ] {
            assert_eq!(
                resolve_native_slash_submission("/wat", context).unwrap(),
                NativeSlashSubmission::NotLocal
            );
        }
        let visible = NativeSlashSubmissionContext {
            visible_completion_index: Some(0),
            ..active
        };
        assert_eq!(
            resolve_native_slash_submission(" /he", visible).unwrap(),
            NativeSlashSubmission::Valid(NativeSlashInvocation {
                command: NativeSlashCommand::Help,
                payload: ""
            })
        );
        assert_eq!(
            resolve_native_slash_submission("/workspace a", visible).unwrap(),
            NativeSlashSubmission::Valid(NativeSlashInvocation {
                command: NativeSlashCommand::Workspace,
                payload: "add"
            })
        );
        assert_eq!(
            resolve_native_slash_submission("/help extra", visible).unwrap(),
            NativeSlashSubmission::KnownInvalid {
                command: NativeSlashCommand::Help
            }
        );
        let wrap = NativeSlashSubmissionContext {
            visible_completion_index: Some(usize::MAX),
            ..active
        };
        let expected = rows("/re")[usize::MAX % rows("/re").len()];
        assert_eq!(
            resolve_native_slash_submission("/re", wrap).unwrap(),
            NativeSlashSubmission::Valid(valid(expected))
        );
    }

    #[test]
    fn slash_commands_completion_rank_alias_dedup_and_pinned_order() {
        assert_eq!(rows("/"), REGISTRY.map(|spec| spec.token));
        assert_eq!(rows("/model"), ["/model", "/models"]);
        assert_eq!(rows("/exit"), ["/exit"]);
        assert_eq!(rows("/quit"), ["/quit"]);
        assert_eq!(rows("/it"), ["/quit"]);
        assert_eq!(rows("/ex"), ["/exit"]);
        assert_eq!(rows("/res"), ["/reset", "/resume"]);
        assert_eq!(
            rows("/e"),
            [
                "/exit",
                "/help",
                "/clear",
                "/new",
                "/reset",
                "/resume",
                "/continue",
                "/rename",
                "/model",
                "/models",
                "/permissions",
                "/workspace",
                "/version"
            ]
        );
        for input in ["", "help", " /help", "/HELP", "/model x", "/resume "] {
            assert!(rows(input).is_empty());
        }
        let model = native_slash_completions("/model").unwrap().next().unwrap();
        assert_eq!(model.category, Some(NativeSlashCategory::Model));
        assert!(model.description.is_some());
        assert!(model.has_args);
    }

    #[test]
    fn slash_commands_public_argument_completions_are_exact() {
        assert_eq!(rows("/sandbox "), ["/sandbox os", "/sandbox none"]);
        assert_eq!(rows("/sandbox O"), ["/sandbox os"]);
        assert!(rows("/sandbox m").is_empty());
        assert!(rows("/sandbox vercel").is_empty());
        assert_eq!(
            rows("/permissions "),
            [
                "/permissions ask",
                "/permissions auto",
                "/permissions remember",
                "/permissions revoke",
                "/permissions yolo",
                "/permissions reset"
            ]
        );
        assert_eq!(
            rows("/workspace "),
            [
                "/workspace list",
                "/workspace add",
                "/workspace remove",
                "/workspace clear"
            ]
        );
        let row = native_slash_completions("/workspace a")
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(row.label, "add");
        assert!(row.has_args);
        assert_eq!(row.category, None);
        assert_eq!(row.description, None);
        assert!(
            !native_slash_completions("/workspace clear")
                .unwrap()
                .next()
                .unwrap()
                .has_args
        );
        // Pinned metadata does not mark the remember/revoke argument rows open.
        assert!(
            !native_slash_completions("/permissions remember")
                .unwrap()
                .next()
                .unwrap()
                .has_args
        );
    }

    #[test]
    fn slash_commands_allowlist_stages_preserve_scope_order_and_tool_catalog() {
        assert_eq!(rows("/allowlist "), ALLOWLIST_ACTIONS);
        assert_eq!(
            rows("/allowlist re"),
            ["/allowlist remove", "/allowlist reset"]
        );
        assert_eq!(rows("/allowlist view "), ALLOWLIST_VIEWS);
        assert!(rows("/allowlist nope ").is_empty());
        for (prefix, tables) in [
            ("/allowlist", &ALLOWLIST_DEFAULT),
            ("/allowlist local", &ALLOWLIST_LOCAL),
            ("/allowlist user", &ALLOWLIST_USER),
        ] {
            assert_eq!(rows(&format!("{prefix} add ")), tables.add);
            assert_eq!(rows(&format!("{prefix} remove ")), tables.remove);
            assert_eq!(rows(&format!("{prefix} reset ")), tables.reset);
            assert_eq!(rows(&format!("{prefix} add tool ")), tables.add_tool);
            assert_eq!(rows(&format!("{prefix} remove tool ")), tables.remove_tool);
            assert_eq!(
                rows(&format!("{prefix} add tool write")),
                [format!("{prefix} add tool write_file")]
            );
            assert_eq!(
                rows(&format!("{prefix} ADD\tTOOL\tWRITE")),
                [format!("{prefix} add tool write_file")]
            );
            for action in ["add", "remove"] {
                let row = native_slash_completions(&format!("{prefix} {action} web-"))
                    .unwrap()
                    .next()
                    .unwrap();
                assert_eq!(row.label, "web-fetch-domain");
                assert!(row.has_args);
            }
            assert!(rows(&format!("{prefix} add command git")).is_empty());
        }
        assert_eq!(rows("/allowlist user "), ALLOWLIST_USER.actions);
        assert_eq!(rows("/allowlist local "), ALLOWLIST_LOCAL.actions);
        assert!(rows("/allowlist user view ").is_empty());
        assert!(rows("/allowlist add tool read ").is_empty());
        assert_eq!(rows("/allowlist add tool"), ["/allowlist add tool"]);
        assert_eq!(rows("/allowlist add tool ").len(), 13);
        for query in [
            "/allowlist view user",
            "/allowlist reset all",
            "/allowlist user reset web-fetch-domains",
        ] {
            assert!(
                !native_slash_completions(query)
                    .unwrap()
                    .next()
                    .unwrap()
                    .has_args
            );
        }
    }

    #[test]
    fn slash_commands_help_is_categorized_and_token_search_is_ascii_case_insensitive() {
        let all: Vec<_> = native_slash_help("")
            .unwrap()
            .map(|spec| spec.token)
            .collect();
        assert_eq!(
            all,
            [
                "/help",
                "/clear",
                "/status",
                "/version",
                "/quit",
                "/new",
                "/reset",
                "/resume",
                "/continue",
                "/rename",
                "/undo",
                "/copy",
                "/compact",
                "/model",
                "/models",
                "/fast",
                "/permissions",
                "/allowlist",
                "/sandbox",
                "/workspace"
            ]
        );
        let search = |query: &str| {
            native_slash_help(query)
                .unwrap()
                .map(|spec| spec.token)
                .collect::<Vec<_>>()
        };
        assert_eq!(search("SeSsIoN\tCoPy"), ["/copy"]);
        assert_eq!(search("GENERAL EXIT"), ["/quit"]);
        assert_eq!(search("security URLs"), ["/allowlist"]);
        assert_eq!(search("saved\nSESSION"), ["/resume"]);
        assert!(search("GENERAL COPY").is_empty());
        assert!(search("\u{b}").is_empty());
        assert!(search("é").is_empty());
        assert_eq!(search(" \t\r\n"), all);
    }

    #[test]
    fn slash_commands_limits_are_inclusive_and_iterators_are_fused() {
        let exact = "x".repeat(MAX_NATIVE_SLASH_INPUT_BYTES);
        assert_eq!(
            route_native_slash(&exact).unwrap(),
            NativeSlashRoute::NotLocal
        );
        assert_eq!(
            route_native_slash(&(exact.clone() + "x")),
            Err(NativeSlashInputError::InputTooLong)
        );
        assert_eq!(
            resolve_native_slash_submission(
                &(exact + "x"),
                NativeSlashSubmissionContext::default()
            ),
            Err(NativeSlashInputError::InputTooLong)
        );
        let query = "é".repeat(MAX_NATIVE_SLASH_QUERY_BYTES / 2);
        assert_eq!(native_slash_help(&query).unwrap().count(), 0);
        assert_eq!(native_slash_completions(&query).unwrap().count(), 0);
        let excess = query + "x";
        assert!(matches!(
            native_slash_help(&excess),
            Err(NativeSlashInputError::QueryTooLong)
        ));
        assert!(matches!(
            native_slash_completions(&excess),
            Err(NativeSlashInputError::QueryTooLong)
        ));
        assert_eq!(
            native_slash_completion_prefix(&excess),
            Err(NativeSlashInputError::QueryTooLong)
        );
        let long_command = format!("/model {}", "x".repeat(MAX_NATIVE_SLASH_QUERY_BYTES));
        assert!(matches!(
            resolve_native_slash_submission(
                &long_command,
                NativeSlashSubmissionContext {
                    visible_completion_index: Some(0),
                    ..Default::default()
                }
            )
            .unwrap(),
            NativeSlashSubmission::Valid(_)
        ));
        let long_whitespace = format!("/sandbox {}o", " ".repeat(MAX_NATIVE_SLASH_QUERY_BYTES));
        assert_eq!(
            resolve_native_slash_submission(
                &long_whitespace,
                NativeSlashSubmissionContext {
                    visible_completion_index: Some(0),
                    ..Default::default()
                }
            )
            .unwrap(),
            NativeSlashSubmission::Valid(NativeSlashInvocation {
                command: NativeSlashCommand::Sandbox,
                payload: "os"
            })
        );
        let mut completions = native_slash_completions("/").unwrap();
        assert_eq!(completions.by_ref().count(), 20);
        assert_eq!(completions.next(), None);
        assert_eq!(completions.next(), None);
        let mut help = native_slash_help("").unwrap();
        assert_eq!(help.by_ref().count(), 20);
        assert_eq!(help.next(), None);
        assert_eq!(help.next(), None);
    }

    #[test]
    fn slash_commands_debug_output_redacts_payload_and_queries() {
        let marker = "private-value-54321";
        let input = format!("/model {marker}");
        assert!(!format!("{:?}", route_native_slash(&input)).contains(marker));
        assert!(
            !format!(
                "{:?}",
                resolve_native_slash_submission(&input, NativeSlashSubmissionContext::default())
            )
            .contains(marker)
        );
        assert!(!format!("{:?}", native_slash_completions(marker)).contains(marker));
        assert!(!format!("{:?}", native_slash_help(marker)).contains(marker));
    }

    #[test]
    fn slash_commands_catalog_routing_and_iteration_allocate_nothing() {
        let measured = allocation_counter::measure(|| {
            std::hint::black_box(native_slash_registry());
            std::hint::black_box(route_native_slash("/model opus").unwrap());
            std::hint::black_box(
                native_slash_completions("/allowlist user add tool ")
                    .unwrap()
                    .count(),
            );
            std::hint::black_box(native_slash_completions("/").unwrap().count());
            std::hint::black_box(native_slash_help("session copy").unwrap().count());
            std::hint::black_box(
                resolve_native_slash_submission(
                    "/he",
                    NativeSlashSubmissionContext {
                        visible_completion_index: Some(0),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
        });
        assert_eq!(measured.count_total, 0);
    }
}
