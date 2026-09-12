//! Bounded metadata presentation; no configuration or connection authority.

use crate::bounded_output::BoundedOutput;
use machine_god_native::mcp::{
    controller::{
        NativeMcpControllerFailure, NativeMcpControllerPublication, NativeMcpControllerReceipt,
    },
    management::{
        McpConfiguredServer, McpConfiguredTransport, McpManagementActivation,
        NativeMcpManagementError, NativeMcpManagementReceipt,
    },
    startup::{NativeMcpStartupReceipt, NativeMcpStartupState},
    store::McpConfigCommitDurability,
};
use std::fmt::Write;

pub(super) fn render(
    id: u64,
    result: Result<&NativeMcpManagementReceipt, &NativeMcpManagementError>,
) -> Result<Vec<u8>, ()> {
    let mut text = super::bounded_output();
    writeln!(text, "\n[control {id}: mcp]").map_err(|_| ())?;
    match result {
        Err(error) => writeln!(text, "{error}; no automatic retry").map_err(|_| ())?,
        Ok(NativeMcpManagementReceipt::Configured(servers)) => configured(&mut text, servers)?,
        Ok(NativeMcpManagementReceipt::Path(path)) => {
            // The admitted profile directory is at most 4096 raw bytes, plus
            // one separator and the fixed filename. Check before lossy conversion.
            if path.as_os_str().len() > 4096 + "/mcp.json".len() {
                return Err(());
            }
            text.write_str("Native MCP profile: ").map_err(|_| ())?;
            super::presentation::escaped(&mut text, &path.to_string_lossy())?;
            text.write_char('\n').map_err(|_| ())?;
        }
        Ok(NativeMcpManagementReceipt::Saved { commit, activation }) => {
            saved(
                &mut text,
                commit.changed(),
                commit.durability(),
                *activation,
            )?;
        }
    }
    text.write_str("> ").map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}

fn configured(text: &mut BoundedOutput, servers: &[McpConfiguredServer]) -> Result<(), ()> {
    if servers.len() > 64 {
        return Err(());
    }
    writeln!(
        text,
        "{} configured servers (configured, not connected):",
        servers.len()
    )
    .map_err(|_| ())?;
    for server in servers {
        if server.name.is_empty() || server.name.len() > 128 {
            return Err(());
        }
        super::presentation::escaped(text, &server.name)?;
        writeln!(
            text,
            ": {}; {}; {}",
            match server.transport {
                McpConfiguredTransport::Stdio => "stdio",
                McpConfiguredTransport::Http => "http",
                McpConfiguredTransport::Sse => "sse",
            },
            if server.enabled {
                "enabled"
            } else {
                "disabled"
            },
            if server.required {
                "required"
            } else {
                "optional"
            },
        )
        .map_err(|_| ())?;
    }
    Ok(())
}

fn saved(
    text: &mut BoundedOutput,
    changed: bool,
    durability: McpConfigCommitDurability,
    activation: McpManagementActivation,
) -> Result<(), ()> {
    writeln!(
        text,
        "Configuration save: {}; {}.",
        match durability {
            McpConfigCommitDurability::Confirmed => "confirmed",
            McpConfigCommitDurability::Ambiguous => "ambiguous",
        },
        if changed { "changed" } else { "unchanged" },
    )
    .map_err(|_| ())?;
    match activation {
        McpManagementActivation::NotAttempted => {
            text.write_str("Runtime activation not attempted.\n")
                .map_err(|_| ())?;
        }
    }
    if durability == McpConfigCommitDurability::Ambiguous {
        text.write_str("Publication may have occurred; inspect the profile before retrying. No automatic retry.\n")
            .map_err(|_| ())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub(super) fn render_reload(
    id: u64,
    result: Result<&NativeMcpControllerReceipt, &NativeMcpControllerFailure>,
) -> Result<Vec<u8>, ()> {
    let mut text = super::bounded_output();
    writeln!(text, "\n[control {id}: mcp reload observation]").map_err(|_| ())?;
    let (startup, cleanup) = match result {
        Ok(receipt) => {
            writeln!(
                text,
                "Publication: {}.",
                match receipt.publication() {
                    NativeMcpControllerPublication::Published => "published",
                    NativeMcpControllerPublication::Unchanged => "unchanged",
                }
            )
            .map_err(|_| ())?;
            if receipt.closed_after_publication() {
                text.write_str("Published, then controller close was observed; not an active connection claim.\n").map_err(|_| ())?;
            }
            (receipt.startup(), receipt.cleanup_complete())
        }
        Err(error) => {
            writeln!(text, "Reload failed ({:?}); this operation did not replace the previous publication. No automatic retry.", error.kind()).map_err(|_| ())?;
            (error.startup(), error.cleanup_complete())
        }
    };
    if let Some(startup) = startup {
        startup_observation(&mut text, startup)?;
    }
    writeln!(
        text,
        "Local cleanup evidence: {}. This is not remote revocation or current connection evidence.",
        if cleanup {
            "complete"
        } else {
            "not complete or not established"
        }
    )
    .map_err(|_| ())?;
    text.write_str("> ").map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}

fn startup_observation(
    text: &mut BoundedOutput,
    receipt: &NativeMcpStartupReceipt,
) -> Result<(), ()> {
    if receipt.servers.len() > 64 {
        return Err(());
    }
    writeln!(
        text,
        "Startup observation: {:?}; {} servers; failures: {}.",
        receipt.phase,
        receipt.servers.len(),
        receipt.has_failures()
    )
    .map_err(|_| ())?;
    for server in receipt.servers.iter() {
        if server.name.is_empty() || server.name.len() > 128 {
            return Err(());
        }
        super::presentation::escaped(text, &server.name)?;
        writeln!(
            text,
            ": {}; {}; attempts {}; local cleanup {}",
            match server.state {
                NativeMcpStartupState::NotAttempted => "not attempted",
                NativeMcpStartupState::Disabled => "disabled",
                NativeMcpStartupState::Deferred => "deferred",
                NativeMcpStartupState::Ready(_) => "ready at startup",
                NativeMcpStartupState::Failed(_) => "startup failed",
            },
            if server.required {
                "required"
            } else {
                "optional"
            },
            server.attempts,
            if server.cleanup.is_complete() {
                "complete"
            } else {
                "not complete"
            }
        )
        .map_err(|_| ())?;
    }
    Ok(())
}

pub(super) fn render_feature_error(
    id: u64,
    error: machine_god_native::mcp::runtime::NativeMcpFeatureError,
) -> Result<Vec<u8>, ()> {
    let mut text = super::bounded_output();
    writeln!(
        text,
        "\n[control {id}: mcp feature failed]\n{error}; no automatic retry.\n> "
    )
    .map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}
