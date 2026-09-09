//! Doctor observation and bounded command presentation.
use crate::bounded_output::BoundedOutput;
use crate::{OUTPUT_FAILURE, write_json_string};
use machine_god_native::{NativeDoctorCheckStatus, NativeDoctorReport, inspect_process_doctor};
use std::{fmt::Write as _, io};
const DOCTOR_RENDER_FAILURE: &str = "machine-god doctor: could not render report\n";
const DOCTOR_CHECK_COUNT: usize = 4;
const MAX_DOCTOR_OUTPUT_BYTES: usize = 4096;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DoctorCheckStatus {
    Ok,
    Warn,
    Fail,
}

impl DoctorCheckStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

impl From<NativeDoctorCheckStatus> for DoctorCheckStatus {
    fn from(status: NativeDoctorCheckStatus) -> Self {
        match status {
            NativeDoctorCheckStatus::Ok => Self::Ok,
            NativeDoctorCheckStatus::Warn => Self::Warn,
            NativeDoctorCheckStatus::Fail => Self::Fail,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DoctorCheckSnapshot {
    name: &'static str,
    status: DoctorCheckStatus,
    detail: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DoctorReportSnapshot {
    ok_count: usize,
    warn_count: usize,
    fail_count: usize,
    checks: [DoctorCheckSnapshot; DOCTOR_CHECK_COUNT],
}

impl DoctorReportSnapshot {
    fn from_native(report: &NativeDoctorReport) -> Result<Self, ()> {
        if report.checked_count() != DOCTOR_CHECK_COUNT
            || report.checks().len() != DOCTOR_CHECK_COUNT
            || report
                .ok_count()
                .checked_add(report.warn_count())
                .and_then(|count| count.checked_add(report.fail_count()))
                != Some(DOCTOR_CHECK_COUNT)
        {
            return Err(());
        }

        let checks = std::array::from_fn(|index| {
            let check = &report.checks()[index];
            DoctorCheckSnapshot {
                name: check.name(),
                status: check.status().into(),
                detail: check.detail(),
            }
        });
        Ok(Self {
            ok_count: report.ok_count(),
            warn_count: report.warn_count(),
            fail_count: report.fail_count(),
            checks,
        })
    }
}

pub(crate) trait DoctorCommandHost {
    fn inspect_doctor(&self) -> Result<DoctorReportSnapshot, ()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProductionDoctorCommandHost;

impl DoctorCommandHost for ProductionDoctorCommandHost {
    fn inspect_doctor(&self) -> Result<DoctorReportSnapshot, ()> {
        DoctorReportSnapshot::from_native(&inspect_process_doctor())
    }
}

pub(crate) fn run_doctor(
    host: &(impl DoctorCommandHost + ?Sized),
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let Ok(report) = host.inspect_doctor() else {
        let _ = stderr.write_all(DOCTOR_RENDER_FAILURE.as_bytes());
        return 1;
    };
    let Ok(output) = render_doctor(&report, json) else {
        let _ = stderr.write_all(DOCTOR_RENDER_FAILURE.as_bytes());
        return 1;
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

fn render_doctor(report: &DoctorReportSnapshot, json: bool) -> Result<String, ()> {
    if report.checks.len() != DOCTOR_CHECK_COUNT {
        return Err(());
    }
    let mut ok_count = 0usize;
    let mut warn_count = 0usize;
    let mut fail_count = 0usize;
    for check in &report.checks {
        match check.status {
            DoctorCheckStatus::Ok => ok_count += 1,
            DoctorCheckStatus::Warn => warn_count += 1,
            DoctorCheckStatus::Fail => fail_count += 1,
        }
    }
    if (report.ok_count, report.warn_count, report.fail_count) != (ok_count, warn_count, fail_count)
    {
        return Err(());
    }

    let mut output = BoundedOutput::with_capacity(MAX_DOCTOR_OUTPUT_BYTES, 1024);
    let rendered = if json {
        write_json_doctor(&mut output, report)
    } else {
        write_human_doctor(&mut output, report)
    };
    rendered.map_err(|_| ())?;
    Ok(output.finish())
}

fn write_human_doctor(
    output: &mut BoundedOutput,
    report: &DoctorReportSnapshot,
) -> std::fmt::Result {
    writeln!(
        output,
        "[doctor] ok={} warn={} fail={}",
        report.ok_count, report.warn_count, report.fail_count
    )?;
    for check in &report.checks {
        writeln!(
            output,
            "[{}] {}: {}",
            check.status.as_str(),
            check.name,
            check.detail
        )?;
    }
    Ok(())
}

fn write_json_doctor(
    output: &mut BoundedOutput,
    report: &DoctorReportSnapshot,
) -> std::fmt::Result {
    output.write_str("{\"kind\":\"doctor\",\"ok_count\":")?;
    write!(
        output,
        "{},\"warn_count\":{},\"fail_count\":{},\"checks\":[",
        report.ok_count, report.warn_count, report.fail_count
    )?;
    for (index, check) in report.checks.iter().enumerate() {
        if index != 0 {
            output.write_char(',')?;
        }
        output.write_str("{\"name\":")?;
        write_json_string(output, check.name)?;
        output.write_str(",\"status\":")?;
        write_json_string(output, check.status.as_str())?;
        output.write_str(",\"detail\":")?;
        write_json_string(output, check.detail)?;
        output.write_char('}')?;
    }
    output.write_str("]}\n")
}

#[cfg(test)]
mod tests;
