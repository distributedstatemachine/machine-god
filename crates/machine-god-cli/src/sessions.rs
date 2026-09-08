//! Thin, bounded presentation of native catalog observations.

use machine_god_core::{BoxFuture, SessionId};
use machine_god_native::NativeSessionCatalogCursor;
use std::{
    ffi::OsString,
    fmt, io,
    task::{Context, Poll, Waker},
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_native::{
    NativeSessionCatalogErrorKind, NativeSessionCatalogInvalidRecords, NativeSessionCatalogPage,
    NativeSessionCatalogQuery, list_process_current_workspace_session_catalog,
    list_process_session_catalog,
};

// Each UTF-8 byte expands by at most six bytes in our JSON/terminal encoder.
// 100 rows: ID128 + title240 + preview240 + two workspace4096 paths + language24;
// 512 bytes per row cover fixed keys and numbers; 4096 cover page/cursor/warnings.
const MAX_OUTPUT_BYTES: usize = 100 * (6 * (128 + 240 + 240 + 8192 + 24) + 16384 + 512) + 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SessionsOptions {
    pub(super) json: bool,
    pub(super) all: bool,
    pub(super) limit: usize,
    pub(super) cursor: Option<NativeSessionCatalogCursor>,
}
impl Default for SessionsOptions {
    fn default() -> Self {
        Self {
            json: false,
            all: false,
            limit: 100,
            cursor: None,
        }
    }
}

pub(super) fn parse_options(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<SessionsOptions, ()> {
    let mut options = SessionsOptions::default();
    let mut arguments = arguments.into_iter();
    let mut limit_seen = false;
    while let Some(argument) = arguments.next() {
        match argument.to_str().ok_or(())? {
            "--json" if !options.json => options.json = true,
            "--all" if !options.all => options.all = true,
            "--limit" if !limit_seen => {
                limit_seen = true;
                let value = arguments.next().ok_or(())?;
                let value = value.to_str().ok_or(())?;
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(());
                }
                options.limit = value.parse().map_err(|_| ())?;
                if !(1..=100).contains(&options.limit) {
                    return Err(());
                }
            }
            "--cursor" if options.cursor.is_none() => {
                let value = arguments.next().ok_or(())?;
                options.cursor = Some(
                    NativeSessionCatalogCursor::parse(value.to_str().ok_or(())?).map_err(|_| ())?,
                );
            }
            _ => return Err(()),
        }
    }
    Ok(options)
}

/// Ephemeral presentation values, never a second session state owner.
#[derive(Clone, Default)]
pub(super) struct SessionsSnapshot {
    entries: Vec<SessionRow>,
    incomplete: bool,
    next_cursor: Option<NativeSessionCatalogCursor>,
    skipped_invalid: usize,
}
#[derive(Clone)]
struct SessionRow {
    id: String,
    title: Option<String>,
    preview: Option<String>,
    workspace: Option<String>,
    workspace_hex: Option<String>,
    origin_workspace: Option<String>,
    origin_workspace_hex: Option<String>,
    created: Option<i64>,
    updated: Option<i64>,
    history_len: usize,
    language: Option<String>,
}
impl fmt::Debug for SessionsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionsSnapshot")
            .field("count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SessionsSnapshot {
    fn from_native(page: &NativeSessionCatalogPage) -> Self {
        Self {
            entries: page
                .entries()
                .iter()
                .map(|entry| {
                    let metadata = entry.native_metadata();
                    let (workspace, workspace_hex) = workspace_text(metadata.workspace());
                    let (origin_workspace, origin_workspace_hex) =
                        workspace_text(metadata.origin_workspace());
                    SessionRow {
                        id: entry.id().as_str().to_owned(),
                        title: metadata.title().map(str::to_owned),
                        preview: entry.preview().map(str::to_owned),
                        workspace,
                        workspace_hex,
                        origin_workspace,
                        origin_workspace_hex,
                        created: metadata.created_at_ms(),
                        updated: metadata.updated_at_ms(),
                        history_len: entry.history_len(),
                        language: metadata.language().map(str::to_owned),
                    }
                })
                .collect(),
            incomplete: !page.scan_complete(),
            next_cursor: page.next_cursor(),
            skipped_invalid: page.skipped_invalid(),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn workspace_text(path: Option<&std::path::Path>) -> (Option<String>, Option<String>) {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt as _;
    let Some(path) = path else {
        return (None, None);
    };
    if let Some(text) = path.to_str() {
        return (Some(text.to_owned()), None);
    }
    let mut hex = String::with_capacity(path.as_os_str().as_bytes().len() * 2);
    for byte in path.as_os_str().as_bytes() {
        write!(hex, "{byte:02x}").expect("String formatting cannot fail");
    }
    (None, Some(hex))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SessionsOperationalFailure {
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    Corrupt,
    ResourceLimit,
    Unavailable,
    #[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
    Unsupported,
}
impl SessionsOperationalFailure {
    const fn category(self) -> &'static str {
        match self {
            #[cfg(any(test, target_os = "linux", target_os = "macos"))]
            Self::Corrupt => "Corrupt",
            Self::ResourceLimit => "ResourceLimit",
            Self::Unavailable => "Unavailable",
            #[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
            Self::Unsupported => "Unsupported",
        }
    }
}
pub(super) trait SessionsCommandHost {
    fn list_sessions(
        &self,
        options: &SessionsOptions,
    ) -> BoxFuture<'static, Result<SessionsSnapshot, SessionsOperationalFailure>>;
}
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ProductionSessionsCommandHost;
impl SessionsCommandHost for ProductionSessionsCommandHost {
    fn list_sessions(
        &self,
        options: &SessionsOptions,
    ) -> BoxFuture<'static, Result<SessionsSnapshot, SessionsOperationalFailure>> {
        let options = options.clone();
        Box::pin(async move {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                let mut query = NativeSessionCatalogQuery::new(options.limit)
                    .map_err(|_| SessionsOperationalFailure::ResourceLimit)?
                    .with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport);
                if let Some(cursor) = options.cursor {
                    query = query.with_continuation(cursor);
                }
                let page = if options.all {
                    list_process_session_catalog(query).await
                } else {
                    list_process_current_workspace_session_catalog(query).await
                }
                .map_err(|error| classify_error(error.kind()))?;
                Ok(SessionsSnapshot::from_native(&page))
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = options;
                Err(SessionsOperationalFailure::Unsupported)
            }
        })
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_error(kind: NativeSessionCatalogErrorKind) -> SessionsOperationalFailure {
    match kind {
        NativeSessionCatalogErrorKind::Corrupt => SessionsOperationalFailure::Corrupt,
        NativeSessionCatalogErrorKind::InvalidQuery => SessionsOperationalFailure::ResourceLimit,
        _ => SessionsOperationalFailure::Unavailable,
    }
}

pub(super) fn run_sessions(
    host: &impl SessionsCommandHost,
    options: &SessionsOptions,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let result = {
        // Native catalog performs bounded synchronous I/O on its first poll.
        // An injected pending future is dropped before any output is written.
        let mut listing = host.list_sessions(options);
        match listing
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => result,
            Poll::Pending => Err(SessionsOperationalFailure::Unavailable),
        }
    };
    let rendered = result.and_then(|snapshot| render_sessions(&snapshot, options));
    match rendered {
        Ok(output) => {
            if stdout.write_all(output.as_bytes()).is_err() {
                let _ = stderr.write_all(super::OUTPUT_FAILURE.as_bytes());
                return 1;
            }
            0
        }
        Err(failure) => write_failure(failure, options.json, stdout, stderr),
    }
}
fn write_failure(
    failure: SessionsOperationalFailure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let category = failure.category();
    let written = if json {
        let output = format!(
            "{{\"kind\":\"sessions\",\"error\":\"could not list sessions: {category}\",\"code\":\"{category}\"}}\n"
        );
        stdout.write_all(output.as_bytes())
    } else {
        let output = format!("machine-god sessions: could not list sessions: {category}\n");
        stderr.write_all(output.as_bytes())
    };
    if written.is_err() {
        let _ = stderr.write_all(super::OUTPUT_FAILURE.as_bytes());
    }
    1
}

struct BoundedOutput(String);
impl fmt::Write for BoundedOutput {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self
            .0
            .len()
            .checked_add(text.len())
            .is_none_or(|len| len > MAX_OUTPUT_BYTES)
        {
            return Err(fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}
fn render_sessions(
    snapshot: &SessionsSnapshot,
    options: &SessionsOptions,
) -> Result<String, SessionsOperationalFailure> {
    validate(snapshot, options).map_err(|()| SessionsOperationalFailure::ResourceLimit)?;
    let mut output = BoundedOutput(String::with_capacity(1024));
    if options.json {
        write_json(&mut output, snapshot)
    } else {
        write_human(&mut output, snapshot, options)
    }
    .map_err(|_| SessionsOperationalFailure::ResourceLimit)?;
    Ok(output.0)
}
fn validate(snapshot: &SessionsSnapshot, options: &SessionsOptions) -> Result<(), ()> {
    if snapshot.entries.len() > options.limit
        || options.limit > 100
        || options.limit == 0
        || (snapshot.incomplete && snapshot.next_cursor.is_some())
    {
        return Err(());
    }
    let mut previous: Option<&SessionRow> = None;
    for row in &snapshot.entries {
        if SessionId::validate(&row.id).is_err()
            || row.title.as_ref().is_some_and(|value| value.len() > 240)
            || row.preview.as_ref().is_some_and(|value| value.len() > 240)
            || !valid_workspace_text(row.workspace.as_deref(), row.workspace_hex.as_deref())
            || !valid_workspace_text(
                row.origin_workspace.as_deref(),
                row.origin_workspace_hex.as_deref(),
            )
            || row.language.as_ref().is_some_and(|value| value.len() > 24)
            || previous.is_some_and(|prior| (prior.updated, &prior.id) <= (row.updated, &row.id))
        {
            return Err(());
        }
        previous = Some(row);
    }
    if let Some(cursor) = &snapshot.next_cursor {
        let last = snapshot.entries.last().ok_or(())?;
        if cursor.id().as_str() != last.id || cursor.updated_at_ms() != last.updated {
            return Err(());
        }
    }
    Ok(())
}
fn valid_workspace_text(text: Option<&str>, hex: Option<&str>) -> bool {
    text.is_none_or(|value| value.len() <= 4096)
        && !hex.is_some_and(|value| {
            text.is_some()
                || value.len() > 8192
                || value.len() % 2 != 0
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}
fn title(row: &SessionRow) -> &str {
    row.title
        .as_deref()
        .filter(|title| !title.is_empty())
        .unwrap_or("Untitled session")
}
fn optional_text(output: &mut impl fmt::Write, text: Option<&str>) -> fmt::Result {
    match text {
        Some(text) => super::write_json_string(output, text),
        None => output.write_str("null"),
    }
}
fn optional_time(output: &mut impl fmt::Write, time: Option<i64>) -> fmt::Result {
    match time {
        Some(time) => write!(output, "{time}"),
        None => output.write_str("null"),
    }
}
fn write_json(output: &mut impl fmt::Write, snapshot: &SessionsSnapshot) -> fmt::Result {
    write!(
        output,
        "{{\"kind\":\"sessions\",\"count\":{}",
        snapshot.entries.len()
    )?;
    if snapshot.skipped_invalid > 0 {
        write!(output, ",\"skipped_invalid\":{}", snapshot.skipped_invalid)?;
    }
    if let Some(cursor) = &snapshot.next_cursor {
        output.write_str(",\"has_more\":true,\"next_cursor\":")?;
        super::write_json_string(output, &cursor.to_string())?;
    }
    if snapshot.incomplete {
        output.write_str(",\"scan_complete\":false,\"truncated\":true")?;
    }
    output.write_str(",\"sessions\":[")?;
    for (index, row) in snapshot.entries.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        output.write_str("{\"id\":")?;
        super::write_json_string(output, &row.id)?;
        output.write_str(",\"title\":")?;
        super::write_json_string(output, title(row))?;
        output.write_str(",\"preview\":")?;
        optional_text(output, row.preview.as_deref())?;
        output.write_str(",\"workspace_root\":")?;
        optional_text(output, row.workspace.as_deref())?;
        output.write_str(",\"origin_workspace_root\":")?;
        optional_text(output, row.origin_workspace.as_deref())?;
        output.write_str(",\"created_at_ms\":")?;
        optional_time(output, row.created)?;
        output.write_str(",\"updated_at_ms\":")?;
        optional_time(output, row.updated)?;
        write!(
            output,
            ",\"history_len\":{},\"conversation_language\":",
            row.history_len
        )?;
        optional_text(output, row.language.as_deref())?;
        if let Some(hex) = &row.workspace_hex {
            output.write_str(",\"workspace_root_hex\":")?;
            super::write_json_string(output, hex)?;
        }
        if let Some(hex) = &row.origin_workspace_hex {
            output.write_str(",\"origin_workspace_root_hex\":")?;
            super::write_json_string(output, hex)?;
        }
        output.write_char('}')?;
    }
    output.write_str("]}\n")
}
fn write_human(
    output: &mut impl fmt::Write,
    snapshot: &SessionsSnapshot,
    options: &SessionsOptions,
) -> fmt::Result {
    if snapshot.entries.is_empty() {
        output.write_str(if snapshot.incomplete {
            "[sessions] 0 saved\n"
        } else if snapshot.skipped_invalid > 0 {
            "[sessions] no readable saved sessions\n"
        } else {
            "[sessions] no saved sessions\n"
        })?;
    } else {
        writeln!(output, "[sessions] {} saved", snapshot.entries.len())?;
        for row in &snapshot.entries {
            output.write_str(" - ")?;
            super::write_json_string_content(output, title(row))?;
            output.write_str("\n   id=")?;
            super::write_json_string_content(output, &row.id)?;
            write!(
                output,
                " | {} turn{}",
                row.history_len,
                if row.history_len == 1 { "" } else { "s" }
            )?;
            if let Some(label) = row.language.as_deref().and_then(language_label) {
                output.write_str(" | ")?;
                super::write_json_string_content(output, label)?;
            }
            output.write_str(" | updated ")?;
            write_timestamp(output, row.updated)?;
            output.write_char('\n')?;
        }
    }
    if let Some(cursor) = &snapshot.next_cursor {
        writeln!(
            output,
            "[sessions] more saved sessions; continue with `machine-god sessions {}--limit {} --cursor {cursor}`",
            if options.all { "--all " } else { "" },
            options.limit
        )?;
    }
    if snapshot.incomplete {
        output.write_str("[sessions] listing incomplete: a resource limit was reached\n")?;
    }
    if snapshot.skipped_invalid > 0 {
        writeln!(
            output,
            "[sessions] warning: skipped {} unreadable saved session{}; run `machine-god doctor` for recovery guidance",
            snapshot.skipped_invalid,
            if snapshot.skipped_invalid == 1 {
                ""
            } else {
                "s"
            }
        )?;
    }
    Ok(())
}
fn language_label(tag: &str) -> Option<&str> {
    if tag.eq_ignore_ascii_case("und") {
        return None;
    }
    if let Some(script) = tag.get(4..).filter(|_| {
        tag.get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("und-"))
    }) {
        for (code, label) in [
            ("Latn", "Latin script"),
            ("Hani", "Han script"),
            ("Arab", "Arabic script"),
            ("Hebr", "Hebrew script"),
            ("Cyrl", "Cyrillic script"),
            ("Grek", "Greek script"),
            ("Deva", "Devanagari script"),
            ("Thai", "Thai script"),
        ] {
            if script.eq_ignore_ascii_case(code) {
                return Some(label);
            }
        }
        return Some(tag);
    }
    let primary = tag.split('-').next().unwrap_or(tag);
    for (code, label) in [
        ("en", "English"),
        ("es", "Spanish"),
        ("fr", "French"),
        ("de", "German"),
        ("it", "Italian"),
        ("pt", "Portuguese"),
        ("ja", "Japanese"),
        ("ko", "Korean"),
        ("zh", "Chinese"),
        ("ar", "Arabic"),
        ("he", "Hebrew"),
        ("ru", "Russian"),
        ("el", "Greek"),
        ("hi", "Hindi"),
        ("th", "Thai"),
    ] {
        if primary.eq_ignore_ascii_case(code) {
            return Some(label);
        }
    }
    Some(tag)
}
fn write_timestamp(output: &mut impl fmt::Write, time: Option<i64>) -> fmt::Result {
    let Some(time) = time.filter(|time| (0..=253_402_300_799_999).contains(time)) else {
        return output.write_str("unknown");
    };
    let seconds = time / 1000;
    let mut days = seconds / 86400;
    let mut year = 1970_i64;
    // At most 8030 iterations, independent of input beyond the validated range.
    loop {
        let length = if leap_year(year) { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
    }
    let months = [
        31,
        if leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for length in months {
        if days < length {
            break;
        }
        days -= length;
        month += 1;
    }
    write!(
        output,
        "{year:04}-{month:02}-{:02} {:02}:{:02}:{:02}.{:03} UTC",
        days + 1,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        time % 1000
    )
}
const fn leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests;
