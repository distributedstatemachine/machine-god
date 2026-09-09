use super::{Action, BoundedOutput, Failure, Options, Receipt};
use machine_god_native::{NativeSessionCleanupStatus, NativeSessionMigration};
use serde_json::{Value, json};
use std::fmt::Write as _;

pub(super) fn render(options: &Options, receipt: Receipt) -> Result<(String, u8), Failure> {
    let (value, human, exit) = project(&options.action, receipt)?;
    let mut output = BoundedOutput::with_capacity(4096, 512);
    if options.json {
        writeln!(output, "{value}")
    } else {
        writeln!(output, "{human}")
    }
    .map_err(|_| Failure::Oversized)?;
    Ok((output.finish(), exit))
}

fn project(action: &Action, receipt: Receipt) -> Result<(Value, String, u8), Failure> {
    match (action, receipt) {
        (Action::Migrate(expected), Receipt::Migration(migration)) => {
            let (record, status) = match migration {
                NativeSessionMigration::AlreadyCurrent(record) => (record, "already_current"),
                NativeSessionMigration::Migrated(record) => (record, "migrated"),
            };
            if record.id != *expected || record.revision.0 == 0 || record.next_turn_sequence == 0 {
                return Err(Failure::Unavailable);
            }
            Ok((
                json!({"kind":"session_maintenance","action":"migrate","status":status,
                "id":record.id.as_str(),"revision":record.revision.0}),
                format!(
                    "[session migration] {status}: {} (revision {})",
                    record.id.as_str(),
                    record.revision.0
                ),
                0,
            ))
        }
        (Action::Recover(source), Receipt::Recovery(recovery)) => {
            let record = recovery.record;
            if record.id == *source || record.revision.0 == 0 || record.next_turn_sequence == 0 {
                return Err(Failure::Unavailable);
            }
            Ok((
                json!({"kind":"session_maintenance","action":"recover","status":"recovered",
                "source_id":source.as_str(),"id":record.id.as_str(),"revision":record.revision.0,
                "message_count":record.messages.len(),"truncated_source":recovery.truncated_source,
                "unknown_tool_results":recovery.unknown_tool_results,"source_unchanged":true}),
                format!(
                    "[session recovery] {} -> {}; source unchanged; messages={}; unknown tool results={}; truncated source={}",
                    source.as_str(),
                    record.id.as_str(),
                    record.messages.len(),
                    recovery.unknown_tool_results,
                    recovery.truncated_source
                ),
                0,
            ))
        }
        (Action::Recover(source), Receipt::RecoveryIndeterminate { session_id }) => {
            if session_id == *source {
                return Err(Failure::Unavailable);
            }
            Ok((
                json!({"kind":"session_maintenance","action":"recover","status":"indeterminate",
                "source_id":source.as_str(),"id":session_id.as_str(),"source_unchanged":true}),
                format!(
                    "[session recovery] indeterminate copy {}; source {} unchanged; reload copy before retrying",
                    session_id.as_str(),
                    source.as_str()
                ),
                1,
            ))
        }
        (Action::Cleanup { apply }, Receipt::Cleanup(report)) => cleanup(*apply, report),
        _ => Err(Failure::Unavailable),
    }
}

fn cleanup(
    apply: bool,
    report: machine_god_native::NativeSessionCleanupReport,
) -> Result<(Value, String, u8), Failure> {
    if report.outcomes.len() > 1024 {
        return Err(Failure::Oversized);
    }
    let mut counts = [0usize; 5];
    for status in report.outcomes {
        counts[match status {
            NativeSessionCleanupStatus::ActiveWriter => 0,
            NativeSessionCleanupStatus::Untrusted => 1,
            NativeSessionCleanupStatus::ReportOnly => 2,
            NativeSessionCleanupStatus::Completed => 3,
            NativeSessionCleanupStatus::Indeterminate => 4,
        }] += 1;
    }
    // Read-only requests must not describe a deletion as their own result.
    if !apply && counts[3] != 0 {
        return Err(Failure::Unavailable);
    }
    let exit = u8::from(
        !report.scan_complete
            || counts[4] != 0
            || (apply && counts[..3].iter().any(|count| *count != 0)),
    );
    Ok((
        json!({"kind":"session_maintenance","action":"cleanup","apply":apply,
        "active_writer":counts[0],"untrusted":counts[1],"report_only":counts[2],
        "completed":counts[3],"indeterminate":counts[4],"scan_complete":report.scan_complete}),
        format!(
            "[session cleanup] apply={apply}; active_writer={}; untrusted={}; report_only={}; completed={}; indeterminate={}; scan_complete={}",
            counts[0], counts[1], counts[2], counts[3], counts[4], report.scan_complete
        ),
        exit,
    ))
}
