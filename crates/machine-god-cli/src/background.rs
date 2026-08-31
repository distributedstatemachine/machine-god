use std::ffi::OsStr;
#[cfg(test)]
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::{Component, Path};
use std::task::{Context, Poll, Waker};

use machine_god_core::BoxFuture;
use machine_god_native::{
    MAX_BACKGROUND_COMMAND_BYTES, MAX_BACKGROUND_COMMAND_PREVIEW_BYTES,
    MAX_BACKGROUND_DIAGNOSTIC_BYTES, MAX_BACKGROUND_PATH_BYTES, MAX_BACKGROUND_RECORDS,
    MAX_BACKGROUND_SERVER_URL_BYTES, NativeBackgroundInspection, NativeBackgroundInspectionError,
    NativeBackgroundInspectionErrorKind, NativeBackgroundQuery, inspect_process_background,
};

const MAX_BACKGROUND_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackgroundTarget {
    List,
    Last,
    Id(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackgroundArguments {
    target: BackgroundTarget,
    json: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BackgroundRecordSnapshot {
    id: u64,
    state: String,
    updated_at_ms: u64,
    command_preview: String,
    preview_truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BackgroundListSnapshot {
    records: Vec<BackgroundRecordSnapshot>,
    truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BackgroundDetailSnapshot {
    id: u64,
    state: String,
    started_at_ms: u64,
    updated_at_ms: u64,
    pid: Option<u32>,
    command: String,
    cwd: String,
    exit_code: Option<i32>,
    server_url: Option<String>,
    diagnostic: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BackgroundSnapshot {
    List(BackgroundListSnapshot),
    Detail(BackgroundDetailSnapshot),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BackgroundOperationalFailure {
    NotFound,
    Corrupt,
    ResourceLimit,
    Unavailable,
    Unsupported,
}

impl BackgroundOperationalFailure {
    const fn category(self) -> &'static str {
        match self {
            Self::NotFound => "NotFound",
            Self::Corrupt => "Corrupt",
            Self::ResourceLimit => "ResourceLimit",
            Self::Unavailable => "Unavailable",
            Self::Unsupported => "Unsupported",
        }
    }
}

pub(super) trait BackgroundCommandHost {
    fn inspect_background(
        &self,
        query: NativeBackgroundQuery,
    ) -> BoxFuture<'static, Result<BackgroundSnapshot, BackgroundOperationalFailure>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ProductionBackgroundCommandHost;

impl BackgroundCommandHost for ProductionBackgroundCommandHost {
    fn inspect_background(
        &self,
        query: NativeBackgroundQuery,
    ) -> BoxFuture<'static, Result<BackgroundSnapshot, BackgroundOperationalFailure>> {
        Box::pin(async move {
            let inspection = inspect_process_background(query)
                .await
                .map_err(classify_inspection_error)?;
            BackgroundSnapshot::from_native(inspection)
        })
    }
}

impl BackgroundSnapshot {
    fn from_native(
        inspection: NativeBackgroundInspection,
    ) -> Result<Self, BackgroundOperationalFailure> {
        let snapshot = match inspection {
            NativeBackgroundInspection::List(list) => Self::List(BackgroundListSnapshot {
                records: list
                    .records()
                    .iter()
                    .map(|record| BackgroundRecordSnapshot {
                        id: record.id(),
                        state: record.state().as_str().to_owned(),
                        updated_at_ms: record.updated_at_ms(),
                        command_preview: record.command_preview().to_owned(),
                        preview_truncated: record.preview_truncated(),
                    })
                    .collect(),
                truncated: list.truncated(),
            }),
            NativeBackgroundInspection::Detail(detail) => Self::Detail(BackgroundDetailSnapshot {
                id: detail.id(),
                state: detail.state().as_str().to_owned(),
                started_at_ms: detail.started_at_ms(),
                updated_at_ms: detail.updated_at_ms(),
                pid: detail.pid(),
                command: detail.command().to_owned(),
                cwd: detail.cwd().to_owned(),
                exit_code: detail.exit_code(),
                server_url: detail.server_url().map(str::to_owned),
                diagnostic: detail.diagnostic().map(str::to_owned),
            }),
        };
        validate_snapshot(&snapshot)?;
        Ok(snapshot)
    }
}

fn classify_inspection_error(
    error: NativeBackgroundInspectionError,
) -> BackgroundOperationalFailure {
    match error.kind() {
        NativeBackgroundInspectionErrorKind::NotFound => BackgroundOperationalFailure::NotFound,
        NativeBackgroundInspectionErrorKind::Corrupt => BackgroundOperationalFailure::Corrupt,
        NativeBackgroundInspectionErrorKind::ResourceLimit => {
            BackgroundOperationalFailure::ResourceLimit
        }
        NativeBackgroundInspectionErrorKind::Unavailable => {
            BackgroundOperationalFailure::Unavailable
        }
        NativeBackgroundInspectionErrorKind::UnsupportedPlatform => {
            BackgroundOperationalFailure::Unsupported
        }
        _ => BackgroundOperationalFailure::Unavailable,
    }
}

pub(super) fn is_background_command(argument: &OsStr) -> bool {
    argument == "background"
}

pub(super) fn run_background<I, S>(
    host: &impl BackgroundCommandHost,
    arguments: I,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    invalid_arguments: &str,
    output_failure: &str,
) -> u8
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let Ok(arguments) = parse_arguments(arguments) else {
        let _ = stderr.write_all(invalid_arguments.as_bytes());
        return 2;
    };
    let query = match arguments.target {
        BackgroundTarget::List => NativeBackgroundQuery::List,
        BackgroundTarget::Last => NativeBackgroundQuery::Last,
        BackgroundTarget::Id(id) => NativeBackgroundQuery::Id(id),
    };
    let mut future = host.inspect_background(query);
    let mut context = Context::from_waker(Waker::noop());
    let result = match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => Err(BackgroundOperationalFailure::Unavailable),
    };
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(failure) => {
            return write_failure(failure, arguments.json, stdout, stderr, output_failure);
        }
    };
    if validate_target_snapshot(arguments.target, &snapshot).is_err() {
        return write_failure(
            BackgroundOperationalFailure::ResourceLimit,
            arguments.json,
            stdout,
            stderr,
            output_failure,
        );
    }
    let output = match render_snapshot(&snapshot, arguments.json) {
        Ok(output) => output,
        Err(failure) => {
            return write_failure(failure, arguments.json, stdout, stderr, output_failure);
        }
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(output_failure.as_bytes());
        return 1;
    }
    0
}

fn validate_target_snapshot(
    target: BackgroundTarget,
    snapshot: &BackgroundSnapshot,
) -> Result<(), BackgroundOperationalFailure> {
    match (target, snapshot) {
        (BackgroundTarget::List, BackgroundSnapshot::List(_))
        | (BackgroundTarget::Last, BackgroundSnapshot::Detail(_)) => Ok(()),
        (BackgroundTarget::Id(requested), BackgroundSnapshot::Detail(detail))
            if requested == detail.id =>
        {
            Ok(())
        }
        _ => Err(BackgroundOperationalFailure::ResourceLimit),
    }
}

fn parse_arguments<I, S>(arguments: I) -> Result<BackgroundArguments, ()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut json = false;
    let mut target = None;
    for argument in arguments {
        let argument = argument.as_ref().to_str().ok_or(())?;
        if argument == "--json" {
            if json {
                return Err(());
            }
            json = true;
            continue;
        }
        if target.is_some() {
            return Err(());
        }
        target = Some(if argument == "last" {
            BackgroundTarget::Last
        } else if !argument.is_empty() && argument.bytes().all(|byte| byte.is_ascii_digit()) {
            BackgroundTarget::Id(argument.parse().map_err(|_| ())?)
        } else {
            return Err(());
        });
    }
    Ok(BackgroundArguments {
        target: target.unwrap_or(BackgroundTarget::List),
        json,
    })
}

fn validate_snapshot(snapshot: &BackgroundSnapshot) -> Result<(), BackgroundOperationalFailure> {
    match snapshot {
        BackgroundSnapshot::List(list) => validate_list(list),
        BackgroundSnapshot::Detail(detail) => validate_detail(detail),
    }
}

fn validate_list(list: &BackgroundListSnapshot) -> Result<(), BackgroundOperationalFailure> {
    if list.records.len() > MAX_BACKGROUND_RECORDS {
        return Err(BackgroundOperationalFailure::ResourceLimit);
    }
    let mut previous = None;
    for record in &list.records {
        validate_state(&record.state)?;
        validate_bounded_string(
            &record.command_preview,
            MAX_BACKGROUND_COMMAND_PREVIEW_BYTES,
            false,
        )?;
        if let Some((updated_at_ms, id)) = previous
            && (record.updated_at_ms, record.id) >= (updated_at_ms, id)
        {
            return Err(BackgroundOperationalFailure::ResourceLimit);
        }
        previous = Some((record.updated_at_ms, record.id));
    }
    Ok(())
}

fn validate_detail(detail: &BackgroundDetailSnapshot) -> Result<(), BackgroundOperationalFailure> {
    validate_state(&detail.state)?;
    if detail.updated_at_ms < detail.started_at_ms || detail.pid == Some(0) {
        return Err(BackgroundOperationalFailure::ResourceLimit);
    }
    match detail.state.as_str() {
        "running" if detail.exit_code.is_some() => {
            return Err(BackgroundOperationalFailure::ResourceLimit);
        }
        "exited" if detail.exit_code != Some(0) => {
            return Err(BackgroundOperationalFailure::ResourceLimit);
        }
        "failed" if detail.exit_code.is_none_or(|code| code == 0) => {
            return Err(BackgroundOperationalFailure::ResourceLimit);
        }
        _ => {}
    }
    validate_bounded_string(&detail.command, MAX_BACKGROUND_COMMAND_BYTES, true)?;
    validate_path(&detail.cwd)?;
    if let Some(server_url) = &detail.server_url {
        validate_bounded_string(server_url, MAX_BACKGROUND_SERVER_URL_BYTES, false)?;
    }
    if let Some(diagnostic) = &detail.diagnostic {
        validate_bounded_string(diagnostic, MAX_BACKGROUND_DIAGNOSTIC_BYTES, false)?;
    }
    Ok(())
}

fn validate_state(state: &str) -> Result<(), BackgroundOperationalFailure> {
    if matches!(
        state,
        "running" | "exited" | "failed" | "stopped" | "dead" | "stale"
    ) {
        Ok(())
    } else {
        Err(BackgroundOperationalFailure::ResourceLimit)
    }
}

fn validate_bounded_string(
    value: &str,
    maximum_bytes: usize,
    require_nonempty: bool,
) -> Result<(), BackgroundOperationalFailure> {
    if value.len() > maximum_bytes || value.contains('\0') || (require_nonempty && value.is_empty())
    {
        Err(BackgroundOperationalFailure::ResourceLimit)
    } else {
        Ok(())
    }
}

fn validate_path(value: &str) -> Result<(), BackgroundOperationalFailure> {
    validate_bounded_string(value, MAX_BACKGROUND_PATH_BYTES, true)?;
    let path = Path::new(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(BackgroundOperationalFailure::ResourceLimit);
    }
    Ok(())
}

fn render_snapshot(
    snapshot: &BackgroundSnapshot,
    json: bool,
) -> Result<String, BackgroundOperationalFailure> {
    validate_snapshot(snapshot)?;
    let mut output = BoundedBackgroundOutput::new();
    let result = match (snapshot, json) {
        (BackgroundSnapshot::List(list), false) => write_human_list(&mut output, list),
        (BackgroundSnapshot::List(list), true) => write_json_list(&mut output, list),
        (BackgroundSnapshot::Detail(detail), false) => write_human_detail(&mut output, detail),
        (BackgroundSnapshot::Detail(detail), true) => write_json_detail(&mut output, detail),
    };
    result.map_err(|_| BackgroundOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn write_human_list(
    output: &mut BoundedBackgroundOutput,
    list: &BackgroundListSnapshot,
) -> std::fmt::Result {
    if list.records.is_empty() && !list.truncated {
        return output.write_str("[background] no persisted background records\n");
    }
    writeln!(output, "[background] {} saved", list.records.len())?;
    for record in &list.records {
        write!(
            output,
            "[background] id={} state={} updated_at_ms={} command_preview=",
            record.id, record.state, record.updated_at_ms,
        )?;
        write_json_string(output, &record.command_preview)?;
        writeln!(output, " preview_truncated={}", record.preview_truncated)?;
    }
    if list.truncated {
        output.write_str("[background] listing incomplete: a resource limit was reached\n")?;
    }
    Ok(())
}

fn write_json_list(
    output: &mut BoundedBackgroundOutput,
    list: &BackgroundListSnapshot,
) -> std::fmt::Result {
    write!(
        output,
        "{{\"kind\":\"background\",\"count\":{},\"truncated\":{},\"records\":[",
        list.records.len(),
        list.truncated,
    )?;
    for (index, record) in list.records.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        write!(output, "{{\"id\":{},\"state\":", record.id,)?;
        write_json_string(output, &record.state)?;
        write!(
            output,
            ",\"updated_at_ms\":{},\"command_preview\":",
            record.updated_at_ms,
        )?;
        write_json_string(output, &record.command_preview)?;
        write!(
            output,
            ",\"preview_truncated\":{}}}",
            record.preview_truncated,
        )?;
    }
    output.write_str("]}\n")
}

fn write_human_detail(
    output: &mut BoundedBackgroundOutput,
    detail: &BackgroundDetailSnapshot,
) -> std::fmt::Result {
    writeln!(output, "[background] id={}", detail.id)?;
    writeln!(output, "[background] state={}", detail.state)?;
    writeln!(
        output,
        "[background] started_at_ms={}",
        detail.started_at_ms,
    )?;
    writeln!(
        output,
        "[background] updated_at_ms={}",
        detail.updated_at_ms,
    )?;
    write_optional_number(output, "pid", detail.pid)?;
    output.write_str("[background] command=")?;
    write_json_string(output, &detail.command)?;
    output.write_char('\n')?;
    output.write_str("[background] cwd=")?;
    write_json_string(output, &detail.cwd)?;
    output.write_char('\n')?;
    write_optional_number(output, "exit_code", detail.exit_code)?;
    write_optional_string(output, "server_url", detail.server_url.as_deref())?;
    write_optional_string(output, "diagnostic", detail.diagnostic.as_deref())
}

fn write_optional_number(
    output: &mut BoundedBackgroundOutput,
    label: &str,
    value: Option<impl std::fmt::Display>,
) -> std::fmt::Result {
    write!(output, "[background] {label}=")?;
    match value {
        Some(value) => writeln!(output, "{value}"),
        None => output.write_str("none\n"),
    }
}

fn write_optional_string(
    output: &mut BoundedBackgroundOutput,
    label: &str,
    value: Option<&str>,
) -> std::fmt::Result {
    write!(output, "[background] {label}=")?;
    if let Some(value) = value {
        write_json_string(output, value)?;
        output.write_char('\n')
    } else {
        output.write_str("none\n")
    }
}

fn write_json_detail(
    output: &mut BoundedBackgroundOutput,
    detail: &BackgroundDetailSnapshot,
) -> std::fmt::Result {
    write!(
        output,
        "{{\"kind\":\"background_detail\",\"id\":{},\"state\":",
        detail.id,
    )?;
    write_json_string(output, &detail.state)?;
    write!(
        output,
        ",\"started_at_ms\":{},\"updated_at_ms\":{},\"pid\":",
        detail.started_at_ms, detail.updated_at_ms,
    )?;
    write_json_optional_number(output, detail.pid)?;
    output.write_str(",\"command\":")?;
    write_json_string(output, &detail.command)?;
    output.write_str(",\"cwd\":")?;
    write_json_string(output, &detail.cwd)?;
    output.write_str(",\"exit_code\":")?;
    write_json_optional_number(output, detail.exit_code)?;
    output.write_str(",\"server_url\":")?;
    write_json_optional_string(output, detail.server_url.as_deref())?;
    output.write_str(",\"diagnostic\":")?;
    write_json_optional_string(output, detail.diagnostic.as_deref())?;
    output.write_str("}\n")
}

fn write_json_optional_number(
    output: &mut BoundedBackgroundOutput,
    value: Option<impl std::fmt::Display>,
) -> std::fmt::Result {
    match value {
        Some(value) => write!(output, "{value}"),
        None => output.write_str("null"),
    }
}

fn write_json_optional_string(
    output: &mut BoundedBackgroundOutput,
    value: Option<&str>,
) -> std::fmt::Result {
    match value {
        Some(value) => write_json_string(output, value),
        None => output.write_str("null"),
    }
}

fn write_failure(
    failure: BackgroundOperationalFailure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
    output_failure: &str,
) -> u8 {
    let category = failure.category();
    let write_result = if json {
        let output = format!(
            "{{\"kind\":\"background\",\"error\":\"could not inspect background history: {category}\",\"code\":\"{category}\"}}\n"
        );
        stdout.write_all(output.as_bytes())
    } else {
        let output =
            format!("machine-god background: could not inspect background history: {category}\n");
        stderr.write_all(output.as_bytes())
    };
    if write_result.is_err() {
        let _ = stderr.write_all(output_failure.as_bytes());
    }
    1
}

#[derive(Debug)]
struct BoundedBackgroundOutput {
    value: String,
}

impl BoundedBackgroundOutput {
    fn new() -> Self {
        Self {
            value: String::with_capacity(1024),
        }
    }

    fn finish(self) -> String {
        self.value
    }
}

impl std::fmt::Write for BoundedBackgroundOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.value.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > MAX_BACKGROUND_OUTPUT_BYTES {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

fn write_json_string(output: &mut BoundedBackgroundOutput, value: &str) -> std::fmt::Result {
    output.write_char('"')?;
    for character in value.chars() {
        match character {
            '"' => output.write_str("\\\"")?,
            '\\' => output.write_str("\\\\")?,
            '\u{08}' => output.write_str("\\b")?,
            '\u{0c}' => output.write_str("\\f")?,
            '\n' => output.write_str("\\n")?,
            '\r' => output.write_str("\\r")?,
            '\t' => output.write_str("\\t")?,
            '\u{00}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{061c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => write!(output, "\\u{:04x}", character as u32)?,
            _ => output.write_char(character)?,
        }
    }
    output.write_char('"')
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    use super::*;

    const INVALID: &str = "invalid\n";
    const OUTPUT: &str = "output failed\n";

    #[derive(Clone, Debug)]
    struct FakeHost {
        result: Result<BackgroundSnapshot, BackgroundOperationalFailure>,
        calls: Rc<Cell<usize>>,
        polls: Arc<AtomicUsize>,
        queries: Rc<RefCell<Vec<NativeBackgroundQuery>>>,
        pending: bool,
    }

    impl FakeHost {
        fn ready(result: Result<BackgroundSnapshot, BackgroundOperationalFailure>) -> Self {
            Self {
                result,
                calls: Rc::new(Cell::new(0)),
                polls: Arc::new(AtomicUsize::new(0)),
                queries: Rc::new(RefCell::new(Vec::new())),
                pending: false,
            }
        }

        fn pending() -> Self {
            let mut host = Self::ready(Err(BackgroundOperationalFailure::Unavailable));
            host.pending = true;
            host
        }
    }

    impl BackgroundCommandHost for FakeHost {
        fn inspect_background(
            &self,
            query: NativeBackgroundQuery,
        ) -> BoxFuture<'static, Result<BackgroundSnapshot, BackgroundOperationalFailure>> {
            self.calls.set(self.calls.get() + 1);
            self.queries.borrow_mut().push(query);
            Box::pin(FakeFuture {
                result: Some(self.result.clone()),
                polls: Arc::clone(&self.polls),
                pending: self.pending,
            })
        }
    }

    struct FakeFuture {
        result: Option<Result<BackgroundSnapshot, BackgroundOperationalFailure>>,
        polls: Arc<AtomicUsize>,
        pending: bool,
    }

    impl Future for FakeFuture {
        type Output = Result<BackgroundSnapshot, BackgroundOperationalFailure>;

        fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            self.polls.fetch_add(1, Ordering::Relaxed);
            if self.pending {
                Poll::Pending
            } else {
                Poll::Ready(self.result.take().expect("fake future is polled once"))
            }
        }
    }

    fn empty_list() -> BackgroundSnapshot {
        BackgroundSnapshot::List(BackgroundListSnapshot {
            records: Vec::new(),
            truncated: false,
        })
    }

    fn detail() -> BackgroundSnapshot {
        BackgroundSnapshot::Detail(BackgroundDetailSnapshot {
            id: 7,
            state: "failed".to_owned(),
            started_at_ms: 10,
            updated_at_ms: 20,
            pid: Some(41),
            command: "printf '\u{1b}[red'\n".to_owned(),
            cwd: "/tmp/quoted-\"".to_owned(),
            exit_code: Some(-9),
            server_url: Some("https://example.invalid/a\\b".to_owned()),
            diagnostic: Some("line\n\u{202e}".to_owned()),
        })
    }

    fn invoke(host: &impl BackgroundCommandHost, arguments: &[&str]) -> (u8, Vec<u8>, Vec<u8>) {
        let arguments = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_background(host, &arguments, &mut stdout, &mut stderr, INVALID, OUTPUT);
        (exit, stdout, stderr)
    }

    #[test]
    fn parser_accepts_only_the_frozen_position_independent_grammar() {
        for (arguments, target, json) in [
            (&[][..], BackgroundTarget::List, false),
            (&["--json"][..], BackgroundTarget::List, true),
            (&["last"][..], BackgroundTarget::Last, false),
            (&["last", "--json"][..], BackgroundTarget::Last, true),
            (&["--json", "last"][..], BackgroundTarget::Last, true),
            (&["00042"][..], BackgroundTarget::Id(42), false),
            (&["00042", "--json"][..], BackgroundTarget::Id(42), true),
            (&["--json", "00042"][..], BackgroundTarget::Id(42), true),
        ] {
            let arguments = arguments.iter().map(OsString::from).collect::<Vec<_>>();
            assert_eq!(
                parse_arguments(&arguments),
                Ok(BackgroundArguments { target, json })
            );
        }
    }

    #[test]
    fn parser_rejects_every_ambiguous_or_noncanonical_form_without_host_effects() {
        let host = FakeHost::ready(Ok(empty_list()));
        for arguments in [
            &["--json", "--json"][..],
            &["last", "last"][..],
            &["1", "2"][..],
            &["Last"][..],
            &["+1"][..],
            &["-1"][..],
            &[" 1"][..],
            &["1 "][..],
            &["--json=true"][..],
            &["18446744073709551616"][..],
            &[""][..],
        ] {
            let (exit, stdout, stderr) = invoke(&host, arguments);
            assert_eq!(exit, 2);
            assert!(stdout.is_empty());
            assert_eq!(stderr, INVALID.as_bytes());
        }
        assert_eq!(host.calls.get(), 0);
        assert_eq!(host.polls.load(Ordering::Relaxed), 0);
    }

    #[cfg(unix)]
    #[test]
    fn parser_rejects_non_unicode_without_host_effects() {
        use std::os::unix::ffi::OsStringExt;

        let host = FakeHost::ready(Ok(empty_list()));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_background(
                &host,
                &[OsString::from_vec(vec![0xff])],
                &mut stdout,
                &mut stderr,
                INVALID,
                OUTPUT,
            ),
            2
        );
        assert_eq!(host.calls.get(), 0);
        assert_eq!(host.polls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn valid_queries_are_mapped_and_the_host_future_is_polled_once() {
        for (arguments, expected) in [
            (&[][..], NativeBackgroundQuery::List),
            (&["last"][..], NativeBackgroundQuery::Last),
            (&["42"][..], NativeBackgroundQuery::Id(42)),
        ] {
            let snapshot = if arguments.is_empty() {
                empty_list()
            } else {
                let BackgroundSnapshot::Detail(mut detail) = detail() else {
                    unreachable!()
                };
                if arguments == ["42"] {
                    detail.id = 42;
                }
                BackgroundSnapshot::Detail(detail)
            };
            let host = FakeHost::ready(Ok(snapshot));
            let (exit, _, _) = invoke(&host, arguments);
            assert_eq!(exit, 0);
            assert_eq!(&*host.queries.borrow(), &[expected]);
            assert_eq!(host.calls.get(), 1);
            assert_eq!(host.polls.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn a_pending_host_future_is_bounded_as_unavailable() {
        let host = FakeHost::pending();
        let (exit, stdout, stderr) = invoke(&host, &["--json"]);
        assert_eq!(exit, 1);
        assert_eq!(
            stdout,
            b"{\"kind\":\"background\",\"error\":\"could not inspect background history: Unavailable\",\"code\":\"Unavailable\"}\n"
        );
        assert!(stderr.is_empty());
        assert_eq!(host.polls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn query_response_mismatches_fail_closed_before_success_output() {
        let host = FakeHost::ready(Ok(empty_list()));
        let (exit, stdout, stderr) = invoke(&host, &["last"]);
        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            b"machine-god background: could not inspect background history: ResourceLimit\n"
        );

        let host = FakeHost::ready(Ok(detail()));
        let (exit, stdout, stderr) = invoke(&host, &["8", "--json"]);
        assert_eq!(exit, 1);
        assert_eq!(
            stdout,
            b"{\"kind\":\"background\",\"error\":\"could not inspect background history: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn empty_and_truncated_lists_have_fixed_human_and_json_shapes() {
        let host = FakeHost::ready(Ok(empty_list()));
        assert_eq!(
            invoke(&host, &[]),
            (
                0,
                b"[background] no persisted background records\n".to_vec(),
                Vec::new(),
            )
        );
        let host = FakeHost::ready(Ok(empty_list()));
        assert_eq!(
            invoke(&host, &["--json"]),
            (
                0,
                b"{\"kind\":\"background\",\"count\":0,\"truncated\":false,\"records\":[]}\n"
                    .to_vec(),
                Vec::new(),
            )
        );
        let host = FakeHost::ready(Ok(BackgroundSnapshot::List(BackgroundListSnapshot {
            records: vec![BackgroundRecordSnapshot {
                id: 8,
                state: "running".to_owned(),
                updated_at_ms: 55,
                command_preview: "line\n\u{1b}[31m".to_owned(),
                preview_truncated: true,
            }],
            truncated: true,
        })));
        let (exit, stdout, stderr) = invoke(&host, &[]);
        assert_eq!(exit, 0);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            concat!(
                "[background] 1 saved\n",
                "[background] id=8 state=running updated_at_ms=55 command_preview=\"line\\n\\u001b[31m\" preview_truncated=true\n",
                "[background] listing incomplete: a resource limit was reached\n",
            )
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn list_json_has_fixed_key_order_and_escaping() {
        let host = FakeHost::ready(Ok(BackgroundSnapshot::List(BackgroundListSnapshot {
            records: vec![
                BackgroundRecordSnapshot {
                    id: 9,
                    state: "exited".to_owned(),
                    updated_at_ms: 20,
                    command_preview: "quote\"\\".to_owned(),
                    preview_truncated: false,
                },
                BackgroundRecordSnapshot {
                    id: 7,
                    state: "stale".to_owned(),
                    updated_at_ms: 20,
                    command_preview: "ok".to_owned(),
                    preview_truncated: false,
                },
            ],
            truncated: false,
        })));
        let (exit, stdout, stderr) = invoke(&host, &["--json"]);
        assert_eq!(exit, 0);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            concat!(
                "{\"kind\":\"background\",\"count\":2,\"truncated\":false,\"records\":[",
                "{\"id\":9,\"state\":\"exited\",\"updated_at_ms\":20,\"command_preview\":\"quote\\\"\\\\\",\"preview_truncated\":false},",
                "{\"id\":7,\"state\":\"stale\",\"updated_at_ms\":20,\"command_preview\":\"ok\",\"preview_truncated\":false}]}\n",
            )
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn detail_human_and_json_render_every_frozen_field_safely() {
        let host = FakeHost::ready(Ok(detail()));
        let (exit, stdout, stderr) = invoke(&host, &["last"]);
        assert_eq!(exit, 0);
        let output = String::from_utf8(stdout).unwrap();
        assert!(output.contains("[background] id=7\n"));
        assert!(output.contains("[background] pid=41\n"));
        assert!(output.contains("[background] command=\"printf '\\u001b[red'\\n\"\n"));
        assert!(output.contains("[background] diagnostic=\"line\\n\\u202e\"\n"));
        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains('\u{202e}'));
        assert!(stderr.is_empty());

        let host = FakeHost::ready(Ok(detail()));
        let (exit, stdout, stderr) = invoke(&host, &["last", "--json"]);
        assert_eq!(exit, 0);
        let output = String::from_utf8(stdout).unwrap();
        assert!(output.starts_with(concat!(
            "{\"kind\":\"background_detail\",\"id\":7,\"state\":\"failed\",",
            "\"started_at_ms\":10,\"updated_at_ms\":20,\"pid\":41,",
        )));
        assert!(output.contains("\"exit_code\":-9"));
        assert!(output.ends_with("\"diagnostic\":\"line\\n\\u202e\"}\n"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn optional_detail_fields_are_explicit() {
        let BackgroundSnapshot::Detail(mut snapshot) = detail() else {
            unreachable!()
        };
        snapshot.state = "stopped".to_owned();
        snapshot.pid = None;
        snapshot.exit_code = None;
        snapshot.server_url = None;
        snapshot.diagnostic = None;
        let host = FakeHost::ready(Ok(BackgroundSnapshot::Detail(snapshot)));
        let (_, stdout, _) = invoke(&host, &["last", "--json"]);
        let output = String::from_utf8(stdout).unwrap();
        assert!(output.contains("\"pid\":null"));
        assert!(output.contains("\"exit_code\":null"));
        assert!(output.contains("\"server_url\":null"));
        assert!(output.contains("\"diagnostic\":null"));
    }

    #[test]
    fn every_closed_operational_failure_has_fixed_streams_and_shapes() {
        for failure in [
            BackgroundOperationalFailure::NotFound,
            BackgroundOperationalFailure::Corrupt,
            BackgroundOperationalFailure::ResourceLimit,
            BackgroundOperationalFailure::Unavailable,
            BackgroundOperationalFailure::Unsupported,
        ] {
            let host = FakeHost::ready(Err(failure));
            let (exit, stdout, stderr) = invoke(&host, &[]);
            assert_eq!(exit, 1);
            assert!(stdout.is_empty());
            assert_eq!(
                String::from_utf8(stderr).unwrap(),
                format!(
                    "machine-god background: could not inspect background history: {}\n",
                    failure.category()
                )
            );

            let host = FakeHost::ready(Err(failure));
            let (exit, stdout, stderr) = invoke(&host, &["--json"]);
            assert_eq!(exit, 1);
            assert_eq!(
                String::from_utf8(stdout).unwrap(),
                format!(
                    "{{\"kind\":\"background\",\"error\":\"could not inspect background history: {}\",\"code\":\"{}\"}}\n",
                    failure.category(),
                    failure.category(),
                )
            );
            assert!(stderr.is_empty());
        }
    }

    #[test]
    fn malformed_snapshots_fail_closed_before_success_output() {
        let invalid = [
            BackgroundSnapshot::List(BackgroundListSnapshot {
                records: (0..=MAX_BACKGROUND_RECORDS)
                    .map(|id| BackgroundRecordSnapshot {
                        id: id as u64,
                        state: "stale".to_owned(),
                        updated_at_ms: (MAX_BACKGROUND_RECORDS - id) as u64,
                        command_preview: String::new(),
                        preview_truncated: false,
                    })
                    .collect(),
                truncated: true,
            }),
            BackgroundSnapshot::List(BackgroundListSnapshot {
                records: vec![
                    BackgroundRecordSnapshot {
                        id: 1,
                        state: "running".to_owned(),
                        updated_at_ms: 1,
                        command_preview: String::new(),
                        preview_truncated: false,
                    },
                    BackgroundRecordSnapshot {
                        id: 2,
                        state: "running".to_owned(),
                        updated_at_ms: 2,
                        command_preview: String::new(),
                        preview_truncated: false,
                    },
                ],
                truncated: false,
            }),
            BackgroundSnapshot::Detail(BackgroundDetailSnapshot {
                id: 1,
                state: "running".to_owned(),
                started_at_ms: 2,
                updated_at_ms: 1,
                pid: Some(0),
                command: String::new(),
                cwd: "relative".to_owned(),
                exit_code: Some(0),
                server_url: None,
                diagnostic: None,
            }),
        ];
        for snapshot in invalid {
            let host = FakeHost::ready(Ok(snapshot));
            let (exit, stdout, stderr) = invoke(&host, &[]);
            assert_eq!(exit, 1);
            assert!(stdout.is_empty());
            assert_eq!(
                stderr,
                b"machine-god background: could not inspect background history: ResourceLimit\n"
            );
        }
    }

    #[test]
    fn representation_expansion_over_the_limit_fails_before_success_output() {
        let BackgroundSnapshot::Detail(mut snapshot) = detail() else {
            unreachable!()
        };
        snapshot.command = "\u{1b}".repeat(MAX_BACKGROUND_COMMAND_BYTES);
        let host = FakeHost::ready(Ok(BackgroundSnapshot::Detail(snapshot)));
        let (exit, stdout, stderr) = invoke(&host, &["--json"]);
        assert_eq!(exit, 1);
        assert_eq!(
            stdout,
            b"{\"kind\":\"background\",\"error\":\"could not inspect background history: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"
        );
        assert!(stderr.is_empty());
    }

    struct BrokenWriter;

    impl io::Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn success_and_failure_writer_errors_use_only_the_global_diagnostic() {
        let host = FakeHost::ready(Ok(empty_list()));
        let mut stdout = BrokenWriter;
        let mut stderr = Vec::new();
        assert_eq!(
            run_background(
                &host,
                std::iter::empty::<OsString>(),
                &mut stdout,
                &mut stderr,
                INVALID,
                OUTPUT,
            ),
            1
        );
        assert_eq!(stderr, OUTPUT.as_bytes());

        let host = FakeHost::ready(Err(BackgroundOperationalFailure::Corrupt));
        stderr.clear();
        assert_eq!(
            run_background(
                &host,
                &[OsString::from("--json")],
                &mut stdout,
                &mut stderr,
                INVALID,
                OUTPUT,
            ),
            1
        );
        assert_eq!(stderr, OUTPUT.as_bytes());
    }
}
