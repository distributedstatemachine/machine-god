//! Exact session observation and bounded command presentation.
use crate::bounded_output::BoundedOutput;
use crate::{OUTPUT_FAILURE, write_json_string};
use machine_god_core::{BoxFuture, SessionId, SessionIncarnationId};
use machine_god_native::{
    NativeSessionInspection, NativeSessionInspectionError, NativeSessionInspectionErrorKind,
    inspect_process_session,
};
use std::{
    fmt::Write as _,
    io,
    task::{Context, Poll, Waker},
};
const MAX_SESSION_OUTPUT_BYTES: usize = 4096;
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionSnapshot {
    id: String,
    incarnation_id: String,
    revision: u64,
    next_turn_sequence: u64,
    message_count: usize,
    metadata_entry_count: usize,
}

impl SessionSnapshot {
    fn from_native(
        inspection: &NativeSessionInspection,
    ) -> Result<Self, SessionOperationalFailure> {
        let snapshot = Self {
            id: inspection.session_id().as_str().to_owned(),
            incarnation_id: inspection.incarnation_id().as_str().to_owned(),
            revision: inspection.revision().0,
            next_turn_sequence: inspection.next_turn_sequence(),
            message_count: inspection.message_count(),
            metadata_entry_count: inspection.metadata_entry_count(),
        };
        validate_session_snapshot(&snapshot)?;
        Ok(snapshot)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionOperationalFailure {
    NotFound,
    Corrupt,
    ResourceLimit,
    Unavailable,
    Unsupported,
}

impl SessionOperationalFailure {
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

pub(crate) trait SessionCommandHost {
    fn inspect_session(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<SessionSnapshot, SessionOperationalFailure>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProductionSessionCommandHost;

impl SessionCommandHost for ProductionSessionCommandHost {
    fn inspect_session(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<SessionSnapshot, SessionOperationalFailure>> {
        Box::pin(async move {
            let inspection = inspect_process_session(id)
                .await
                .map_err(classify_session_inspection_error)?;
            SessionSnapshot::from_native(&inspection)
        })
    }
}

fn classify_session_inspection_error(
    error: NativeSessionInspectionError,
) -> SessionOperationalFailure {
    classify_session_inspection_error_kind(error.kind())
}

fn classify_session_inspection_error_kind(
    kind: NativeSessionInspectionErrorKind,
) -> SessionOperationalFailure {
    match kind {
        NativeSessionInspectionErrorKind::UnsupportedPlatform => {
            SessionOperationalFailure::Unsupported
        }
        NativeSessionInspectionErrorKind::NotFound => SessionOperationalFailure::NotFound,
        NativeSessionInspectionErrorKind::Corrupt => SessionOperationalFailure::Corrupt,
        _ => SessionOperationalFailure::Unavailable,
    }
}

pub(crate) fn run_session(
    host: &(impl SessionCommandHost + ?Sized),
    id: SessionId,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let requested_id = id.as_str().to_owned();
    let mut inspection = host.inspect_session(id);
    let mut context = Context::from_waker(Waker::noop());
    let result = match inspection.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => Err(SessionOperationalFailure::Unavailable),
    };
    let snapshot = match result {
        Ok(snapshot) if snapshot.id == requested_id => snapshot,
        Ok(_) => {
            return write_session_failure(
                SessionOperationalFailure::ResourceLimit,
                json,
                stdout,
                stderr,
            );
        }
        Err(failure) => return write_session_failure(failure, json, stdout, stderr),
    };
    let output = match render_session(&snapshot, json) {
        Ok(output) => output,
        Err(failure) => return write_session_failure(failure, json, stdout, stderr),
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn write_session_failure(
    failure: SessionOperationalFailure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let category = failure.category();
    if json {
        let mut output = BoundedOutput::with_capacity(MAX_SESSION_OUTPUT_BYTES, 512);
        let rendered = (|| {
            output.write_str("{\"kind\":\"session\",\"error\":")?;
            write_json_string(
                &mut output,
                &format!("could not inspect session: {category}"),
            )?;
            output.write_str(",\"code\":")?;
            write_json_string(&mut output, category)?;
            output.write_str("}\n")
        })();
        let output = if rendered.is_ok() {
            output.finish()
        } else {
            concat!(
                "{\"kind\":\"session\",\"error\":",
                "\"could not inspect session: ResourceLimit\",",
                "\"code\":\"ResourceLimit\"}\n",
            )
            .to_owned()
        };
        if stdout.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    } else {
        let mut output = String::from("machine-god session: could not inspect session: ");
        output.push_str(category);
        output.push('\n');
        if stderr.write_all(output.as_bytes()).is_err() {
            let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        }
    }
    1
}

fn render_session(
    snapshot: &SessionSnapshot,
    json: bool,
) -> Result<String, SessionOperationalFailure> {
    validate_session_snapshot(snapshot)?;
    let mut output = BoundedOutput::with_capacity(MAX_SESSION_OUTPUT_BYTES, 512);
    let rendered = if json {
        write_json_session(&mut output, snapshot)
    } else {
        write_human_session(&mut output, snapshot)
    };
    rendered.map_err(|_| SessionOperationalFailure::ResourceLimit)?;
    Ok(output.finish())
}

fn validate_session_snapshot(snapshot: &SessionSnapshot) -> Result<(), SessionOperationalFailure> {
    if SessionId::new(snapshot.id.clone()).is_err()
        || SessionIncarnationId::new(snapshot.incarnation_id.clone()).is_err()
        || snapshot.revision == 0
        || snapshot.next_turn_sequence == 0
    {
        return Err(SessionOperationalFailure::ResourceLimit);
    }
    Ok(())
}

fn write_human_session(output: &mut BoundedOutput, snapshot: &SessionSnapshot) -> std::fmt::Result {
    writeln!(output, "[session] {}", snapshot.id)?;
    writeln!(output, " - incarnation_id: {}", snapshot.incarnation_id)?;
    writeln!(output, " - revision: {}", snapshot.revision)?;
    writeln!(
        output,
        " - next_turn_sequence: {}",
        snapshot.next_turn_sequence
    )?;
    writeln!(output, " - message_count: {}", snapshot.message_count)?;
    writeln!(
        output,
        " - metadata_entry_count: {}",
        snapshot.metadata_entry_count
    )
}

fn write_json_session(output: &mut BoundedOutput, snapshot: &SessionSnapshot) -> std::fmt::Result {
    output.write_str("{\"kind\":\"session\",\"id\":")?;
    write_json_string(output, &snapshot.id)?;
    output.write_str(",\"incarnation_id\":")?;
    write_json_string(output, &snapshot.incarnation_id)?;
    write!(
        output,
        ",\"revision\":{},\"next_turn_sequence\":{},\"message_count\":{},\"metadata_entry_count\":{}",
        snapshot.revision,
        snapshot.next_turn_sequence,
        snapshot.message_count,
        snapshot.metadata_entry_count,
    )?;
    output.write_str("}\n")
}

#[cfg(test)]
mod tests;
