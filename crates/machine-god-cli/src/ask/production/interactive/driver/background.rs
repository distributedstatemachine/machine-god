//! Bounded presentation of native receipts; no process or selection authority.

use crate::bounded_output::BoundedOutput;
use machine_god_core::TerminalLifecycle;
use machine_god_native::{
    NativeBackgroundControlReceipt as Receipt, NativeBackgroundLogWindow,
    NativeBackgroundOpenOutcome,
};
use std::fmt::Write;

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_COMMAND_PREVIEW_BYTES: usize = 256;
const MAX_LOG_LINES: usize = 40;

pub(super) fn render(id: u64, receipt: &Receipt) -> Result<Vec<u8>, ()> {
    let mut text = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 1024);
    writeln!(text, "\n[control {id}: background]").map_err(|_| ())?;
    match receipt {
        Receipt::Listed(snapshot) => listing(&mut text, snapshot)?,
        Receipt::Stopped {
            session_id,
            receipt,
        } => {
            write!(text, "{}: ", session_id.as_str()).map_err(|_| ())?;
            if receipt.was_live() {
                text.write_str("graceful close receipt; ").map_err(|_| ())?;
            } else {
                text.write_str("history-only receipt; no live process was stopped; ")
                    .map_err(|_| ())?;
            }
            writeln!(
                text,
                "state={}",
                receipt
                    .result()
                    .session()
                    .map_or("unavailable", |facts| lifecycle(facts.lifecycle))
            )
            .map_err(|_| ())?;
        }
        Receipt::Logs(summary) => {
            writeln!(
                text,
                "{}: escaped raw output; up to 40 lines per 16 KiB window",
                summary.session_id().as_str()
            )
            .map_err(|_| ())?;
            window(&mut text, "head", summary.head(), false)?;
            window(&mut text, "tail", summary.tail(), true)?;
        }
        Receipt::Opened {
            session_id,
            url,
            outcome,
        } => {
            write!(
                text,
                "{}: {}: ",
                session_id.as_str(),
                match outcome {
                    NativeBackgroundOpenOutcome::Opened => "URL launcher accepted",
                    NativeBackgroundOpenOutcome::LauncherFailed =>
                        "URL launcher failed; a browser may have opened",
                    NativeBackgroundOpenOutcome::Indeterminate =>
                        "URL handoff uncertain; no automatic retry",
                }
            )
            .map_err(|_| ())?;
            escaped(&mut text, url)?;
            text.write_str("\nNo server reachability or browser-lifetime claim.\n")
                .map_err(|_| ())?;
        }
        Receipt::NoKnownUrl { session_id } => writeln!(
            text,
            "{}: no HTTP(S) URL found in the bounded output prefix",
            session_id.as_str()
        )
        .map_err(|_| ())?,
        Receipt::NotRunning { session_id } => writeln!(
            text,
            "{}: no currently owned running backend; URL not opened",
            session_id.as_str()
        )
        .map_err(|_| ())?,
    }
    text.write_str("> ").map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}

fn listing(
    text: &mut BoundedOutput,
    snapshot: &machine_god_native::NativeTerminalBackgroundSnapshot,
) -> Result<(), ()> {
    if snapshot.entries().is_empty() {
        text.write_str("No background command history.\n")
            .map_err(|_| ())?;
    }
    for entry in snapshot.entries() {
        write!(
            text,
            "{} {}{} created={} ",
            entry.id().as_str(),
            lifecycle(entry.facts().lifecycle),
            if entry.owns_backend() {
                " (owned backend)"
            } else {
                " (history only)"
            },
            entry.created_at_ms()
        )
        .map_err(|_| ())?;
        if let Some(command) = entry.command() {
            let end = command.floor_char_boundary(command.len().min(MAX_COMMAND_PREVIEW_BYTES));
            escaped(text, &command[..end])?;
            if end < command.len() {
                text.write_str(" [command preview truncated]")
                    .map_err(|_| ())?;
            }
        } else {
            text.write_str("(interactive shell)").map_err(|_| ())?;
        }
        text.write_char('\n').map_err(|_| ())?;
    }
    Ok(())
}

fn lifecycle(value: TerminalLifecycle) -> &'static str {
    match value {
        TerminalLifecycle::Starting => "starting",
        TerminalLifecycle::Running => "running",
        TerminalLifecycle::Exited => "exited",
        TerminalLifecycle::Lost => "lost",
        TerminalLifecycle::Closed => "closed",
    }
}

fn window(
    text: &mut BoundedOutput,
    name: &str,
    window: &NativeBackgroundLogWindow,
    tail: bool,
) -> Result<(), ()> {
    writeln!(
        text,
        "--- {name} {}:{}..{}:{} (snapshot {}:{}, gap={}, more={}) ---",
        window.source().segment(),
        window.source().offset(),
        window.next().segment(),
        window.next().offset(),
        window.snapshot_end().segment(),
        window.snapshot_end().offset(),
        window.has_gap(),
        window.truncated()
    )
    .map_err(|_| ())?;
    render_lines(text, window.bytes(), tail)
}

fn render_lines(text: &mut BoundedOutput, bytes: &[u8], tail: bool) -> Result<(), ()> {
    let lines = || bytes.split_inclusive(|byte| *byte == b'\n');
    let count = lines().count();
    let skip = if tail {
        count.saturating_sub(MAX_LOG_LINES)
    } else {
        0
    };
    if skip > 0 {
        text.write_str("[earlier lines omitted]\n")
            .map_err(|_| ())?;
    }
    for line in lines().skip(skip).take(MAX_LOG_LINES) {
        escaped_bytes(text, line.strip_suffix(b"\n").unwrap_or(line))?;
        text.write_char('\n').map_err(|_| ())?;
    }
    if !tail && count > MAX_LOG_LINES {
        text.write_str("[later lines omitted]\n").map_err(|_| ())?;
    }
    Ok(())
}

fn escaped(text: &mut BoundedOutput, value: &str) -> Result<(), ()> {
    crate::write_json_string_content(text, value).map_err(|_| ())
}

fn escaped_bytes(text: &mut BoundedOutput, mut bytes: &[u8]) -> Result<(), ()> {
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(value) => return escaped(text, value),
            Err(error) => {
                let (valid, rest) = bytes.split_at(error.valid_up_to());
                escaped(text, std::str::from_utf8(valid).map_err(|_| ())?)?;
                let invalid = error.error_len().unwrap_or(rest.len());
                for byte in &rest[..invalid] {
                    write!(text, "\\x{byte:02x}").map_err(|_| ())?;
                }
                bytes = &rest[invalid..];
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_and_terminal_controls_are_visible_not_executed() {
        let mut text = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 1024);
        render_lines(&mut text, b"ok\n\xff\x1b[2J\r\0\xe2\x82", false).unwrap();
        let text = text.finish();
        assert!(text.starts_with("ok\n\\xff"));
        assert!(text.ends_with("\\xe2\\x82\n"));
        assert!(!text.contains(['\x1b', '\r', '\0']));
    }

    #[test]
    fn head_and_tail_select_opposite_forty_lines() {
        let mut bytes = String::new();
        for line in 0..60 {
            writeln!(bytes, "line-{line}").unwrap();
        }
        let mut head = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 1024);
        let mut tail = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 1024);
        render_lines(&mut head, bytes.as_bytes(), false).unwrap();
        render_lines(&mut tail, bytes.as_bytes(), true).unwrap();
        let head = head.finish();
        let tail = tail.finish();
        assert!(head.starts_with("line-0\n"));
        assert!(head.contains("line-39\n[later lines omitted]"));
        assert!(!head.contains("line-40\n"));
        assert!(tail.starts_with("[earlier lines omitted]\nline-20\n"));
        assert!(tail.ends_with("line-59\n"));
    }

    #[test]
    fn maximum_binary_windows_fit_and_output_errors_propagate() {
        let bytes = vec![0xff; 16 * 1024];
        let mut text = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 1024);
        render_lines(&mut text, &bytes, false).unwrap();
        render_lines(&mut text, &bytes, true).unwrap();
        assert!(text.finish().len() < MAX_OUTPUT_BYTES);
        let mut text = BoundedOutput::with_capacity(1, 1);
        assert!(render_lines(&mut text, &bytes, false).is_err());
    }
}
