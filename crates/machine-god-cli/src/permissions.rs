//! Bounded presentation of a native-owned configured-pattern observation.

use std::{fmt::Write as _, io};

use machine_god_native::{
    ConfigOrigin, NativeConfiguredPermissionDecision, NativeConfiguredPermissionScope,
    NativePermissionInspection, NativePermissionInspectionRule, inspect_process_permissions,
};

use crate::{
    CONFIGURATION_FAILURE, OUTPUT_FAILURE, bounded_output::BoundedOutput, write_json_string,
};

const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const RENDER_FAILURE: &str = "machine-god permissions: could not render report\n";

pub(crate) trait PermissionsCommandHost {
    fn inspect(&self) -> Result<NativePermissionInspection, ()>;
}

pub(crate) struct ProductionPermissionsCommandHost;

impl PermissionsCommandHost for ProductionPermissionsCommandHost {
    fn inspect(&self) -> Result<NativePermissionInspection, ()> {
        inspect_process_permissions().map_err(|_| ())
    }
}

pub(crate) fn run_permissions(
    host: &dyn PermissionsCommandHost,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let Ok(report) = host.inspect() else {
        let _ = stderr.write_all(CONFIGURATION_FAILURE.as_bytes());
        return 1;
    };
    let Ok(output) = render(&report, json) else {
        let _ = stderr.write_all(RENDER_FAILURE.as_bytes());
        return 1;
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    0
}

pub(crate) fn render(report: &NativePermissionInspection, json: bool) -> Result<String, ()> {
    let mut output = BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 1024);
    if json {
        json_report(&mut output, report)
    } else {
        human_report(&mut output, report)
    }
    .map_err(|_| ())?;
    Ok(output.finish())
}

fn origin(report: &NativePermissionInspection) -> &'static str {
    match report.origin() {
        ConfigOrigin::BuiltInDefaults => "built_in_defaults",
        ConfigOrigin::File => "file",
    }
}

fn scope(report: &NativePermissionInspection) -> &'static str {
    match report.effective_scope() {
        NativeConfiguredPermissionScope::User => "user",
        NativeConfiguredPermissionScope::Local => "local",
    }
}

fn decision(rule: &NativePermissionInspectionRule) -> &'static str {
    match rule.decision() {
        NativeConfiguredPermissionDecision::Allow => "allow",
        NativeConfiguredPermissionDecision::Ask => "ask",
        NativeConfiguredPermissionDecision::Deny => "deny",
    }
}

fn human_report(
    output: &mut BoundedOutput,
    report: &NativePermissionInspection,
) -> std::fmt::Result {
    output.write_str(&crate::identity())?;
    writeln!(
        output,
        "permission_mode: {}",
        report.permission_mode().as_str()
    )?;
    writeln!(output, "configuration_origin: {}", origin(report))?;
    writeln!(output, "configured_rules_source: {}", scope(report))?;
    writeln!(output, "user_rules: {}", report.user_rules().len())?;
    human_rows(output, "user", report.user_rules())?;
    if let Some(local) = report.local_rules() {
        writeln!(output, "local_rules: {}", local.len())?;
        human_rows(output, "local", local)?;
    } else {
        output.write_str("local_rules: absent\n")?;
    }
    output.write_str("saved_exact_rules: unavailable\nruntime_grants: unavailable\n")
}

fn human_rows(
    output: &mut BoundedOutput,
    scope: &str,
    rows: &[NativePermissionInspectionRule],
) -> std::fmt::Result {
    for row in rows {
        write!(output, "  {scope} {} ", decision(row))?;
        write_json_string(output, row.permission())?;
        output.write_char(' ')?;
        match row.pattern() {
            Some(pattern) => write_json_string(output, pattern)?,
            None => output.write_str("[inert pattern omitted]")?,
        }
        output.write_char('\n')?;
    }
    Ok(())
}

fn json_report(
    output: &mut BoundedOutput,
    report: &NativePermissionInspection,
) -> std::fmt::Result {
    output.write_str("{\"name\":\"machine-god\",\"version\":")?;
    write_json_string(output, env!("CARGO_PKG_VERSION"))?;
    write!(
        output,
        ",\"engine_api_version\":{},\"kind\":\"permissions\",\"permission_mode\":",
        machine_god_native::supported_core_api_version()
    )?;
    write_json_string(output, report.permission_mode().as_str())?;
    output.write_str(",\"configuration_origin\":")?;
    write_json_string(output, origin(report))?;
    output.write_str(",\"configured_rules\":{\"effective_source\":")?;
    write_json_string(output, scope(report))?;
    output.write_str(",\"user\":")?;
    json_rows(output, report.user_rules())?;
    output.write_str(",\"local\":")?;
    if let Some(local) = report.local_rules() {
        json_rows(output, local)?;
    } else {
        output.write_str("null")?;
    }
    output
        .write_str("},\"saved_exact_rules_available\":false,\"runtime_grants_available\":false}\n")
}

fn json_rows(
    output: &mut BoundedOutput,
    rows: &[NativePermissionInspectionRule],
) -> std::fmt::Result {
    output.write_char('[')?;
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            output.write_char(',')?;
        }
        output.write_str("{\"permission\":")?;
        write_json_string(output, row.permission())?;
        output.write_str(",\"pattern\":")?;
        if let Some(pattern) = row.pattern() {
            write_json_string(output, pattern)?;
        } else {
            output.write_str("null")?;
        }
        output.write_str(",\"action\":")?;
        write_json_string(output, decision(row))?;
        write!(output, ",\"inert\":{}}}", row.inert())?;
    }
    output.write_char(']')
}

#[cfg(test)]
mod tests;
