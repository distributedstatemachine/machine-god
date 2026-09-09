//! Thin session-maintenance grammar, native host selection and bounded receipts.

use crate::bounded_output::BoundedOutput;
use machine_god_core::SessionId;
use machine_god_native::{
    NativeSessionCleanupMode, NativeSessionMaintenanceError as Failure,
    NativeSessionMaintenanceReceipt as Receipt, NativeSessionMaintenanceRequest,
};
use std::{ffi::OsString, fmt::Write as _, io};

mod rendering;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Migrate(SessionId),
    Recover(SessionId),
    Cleanup { apply: bool },
}

impl Action {
    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Migrate(_) => "migrate",
            Self::Recover(_) => "recover",
            Self::Cleanup { .. } => "cleanup",
        }
    }

    fn request(&self) -> NativeSessionMaintenanceRequest {
        match self {
            Self::Migrate(id) => NativeSessionMaintenanceRequest::Migrate {
                session_id: id.clone(),
            },
            Self::Recover(id) => NativeSessionMaintenanceRequest::Recover {
                session_id: id.clone(),
            },
            Self::Cleanup { apply } => NativeSessionMaintenanceRequest::Cleanup {
                mode: if *apply {
                    NativeSessionCleanupMode::Apply
                } else {
                    NativeSessionCleanupMode::ReportOnly
                },
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Options {
    pub(crate) action: Action,
    pub(crate) json: bool,
}

pub(crate) fn parse_session(
    recover: bool,
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Options, ()> {
    let mut arguments = arguments.into_iter();
    let id = arguments.next().ok_or(())?.into_string().map_err(|_| ())?;
    if id.starts_with('-') || id == "last" {
        return Err(());
    }
    let id = SessionId::new(id).map_err(|_| ())?;
    let json = match arguments.next() {
        None => false,
        Some(flag) if flag == "--json" => true,
        Some(_) => return Err(()),
    };
    if arguments.next().is_some() {
        return Err(());
    }
    Ok(Options {
        action: if recover {
            Action::Recover(id)
        } else {
            Action::Migrate(id)
        },
        json,
    })
}

pub(crate) fn parse_cleanup(arguments: impl IntoIterator<Item = OsString>) -> Result<Options, ()> {
    let mut apply = false;
    let mut json = false;
    for flag in arguments {
        if flag == "--apply" && !apply {
            apply = true;
        } else if flag == "--json" && !json {
            json = true;
        } else {
            return Err(());
        }
    }
    Ok(Options {
        action: Action::Cleanup { apply },
        json,
    })
}

pub(crate) trait MaintenanceCommandHost {
    /// Returns only after native work and its actual worker completion settle.
    fn execute(&self, action: &Action) -> Result<Receipt, Failure>;
}

pub(crate) struct ProductionMaintenanceCommandHost;

impl MaintenanceCommandHost for ProductionMaintenanceCommandHost {
    fn execute(&self, action: &Action) -> Result<Receipt, Failure> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| Failure::Unavailable)?;
            // No one-poll approximation and no CLI-owned filesystem operations.
            // Do not register signal handlers: default process interruption is
            // handled by native atomic publication and authoritative reload.
            runtime.block_on(machine_god_native::execute_process_session_maintenance(
                action.request(),
                machine_god_core::CancellationToken::new(),
            ))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = action.request();
            Err(Failure::UnsupportedPlatform)
        }
    }
}

pub(crate) fn run(
    host: &dyn MaintenanceCommandHost,
    options: &Options,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let result = host
        .execute(&options.action)
        .and_then(|receipt| rendering::render(options, receipt));
    let (output, exit) = match result {
        Ok(result) => result,
        Err(error) => return write_failure(options, error, stdout, stderr),
    };
    if stdout.write_all(output.as_bytes()).is_err() {
        let _ = stderr.write_all(crate::OUTPUT_FAILURE.as_bytes());
        return 1;
    }
    exit
}

fn write_failure(
    options: &Options,
    error: Failure,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> u8 {
    let mut output = BoundedOutput::with_capacity(4096, 256);
    let result = if options.json {
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "kind":"session_maintenance", "action":options.action.name(),
                "code":format!("{error:?}"), "error":"maintenance failed; reload before retrying uncertain effects"
            })
        )
    } else {
        writeln!(
            output,
            "machine-god {}: {error}; reload before retrying uncertain effects",
            options.action.name()
        )
    };
    let failed = result.is_err()
        || if options.json {
            stdout.write_all(output.finish().as_bytes()).is_err()
        } else {
            stderr.write_all(output.finish().as_bytes()).is_err()
        };
    if failed {
        let _ = stderr.write_all(crate::OUTPUT_FAILURE.as_bytes());
    }
    1
}
