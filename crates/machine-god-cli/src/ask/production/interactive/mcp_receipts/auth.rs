//! Thin observations of native auth receipts; never browser or token authority.

use crate::bounded_output::BoundedOutput;
use machine_god_native::mcp::{
    auth::{McpAuthError, McpAuthLocalRemoval, McpAuthLogoutReceipt, McpAuthRemoteRevocation},
    controller::{NativeMcpAuthenticationError, NativeMcpAuthenticationReceipt},
};
use std::fmt::Write;

pub(crate) fn render(
    id: u64,
    result: Result<&NativeMcpAuthenticationReceipt, &NativeMcpAuthenticationError>,
) -> Result<Vec<u8>, ()> {
    let mut text = super::super::bounded_output();
    writeln!(text, "\n[control {id}: mcp authentication observation]").map_err(|_| ())?;
    match result {
        Ok(NativeMcpAuthenticationReceipt::ConfirmationRequired { server }) => {
            text.write_str("To confirm opening your browser, repeat: /mcp auth ")
                .map_err(|_| ())?;
            name(&mut text, server)?;
            text.write_str(" --open\nNo browser handoff or authentication completion is established by this confirmation request.\n").map_err(|_| ())?;
        }
        Ok(NativeMcpAuthenticationReceipt::Authenticated {
            server,
            usable,
            activation,
        }) => {
            text.write_str("Server: ").map_err(|_| ())?;
            name(&mut text, server)?;
            writeln!(
                text,
                "\nCredential persistence: confirmed.\nCredential usability at observation: {}.",
                if *usable { "usable" } else { "not usable" }
            )
            .map_err(|_| ())?;
            match activation {
                Some(result) => {
                    text.write_str("Runtime activation: full configured reload observation (not a targeted reconnect).\n").map_err(|_| ())?;
                    super::reload_observation(&mut text, result.as_ref())?;
                }
                None => text
                    .write_str("Runtime activation: not attempted.\n")
                    .map_err(|_| ())?,
            }
            text.write_str("Saved credentials and reload observations do not establish a current connection.\n").map_err(|_| ())?;
        }
        Ok(NativeMcpAuthenticationReceipt::LoggedOut { server, outcome }) => {
            text.write_str("Server: ").map_err(|_| ())?;
            name(&mut text, server)?;
            text.write_char('\n').map_err(|_| ())?;
            logout(&mut text, *outcome)?;
        }
        Err(NativeMcpAuthenticationError::Selection(error)) => {
            writeln!(
                text,
                "Authentication selection failed ({:?}); no automatic retry.",
                error.kind()
            )
            .map_err(|_| ())?;
        }
        Err(NativeMcpAuthenticationError::Authorization(error)) => {
            authorization_error(&mut text, *error)?
        }
    }
    text.write_str("> ").map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}

fn name(text: &mut BoundedOutput, server: &str) -> Result<(), ()> {
    if server.is_empty() || server.len() > 128 {
        return Err(());
    }
    super::super::presentation::escaped(text, server)
}

fn authorization_error(text: &mut BoundedOutput, error: McpAuthError) -> Result<(), ()> {
    writeln!(text, "{error}; no automatic retry.").map_err(|_| ())?;
    match error {
        McpAuthError::IssuerMismatch => text.write_str("The issuer mismatch was rejected. Edit oauth.issuer in the selected server's MCP configuration to the issuer you intend to trust, then retry /mcp auth NAME --open.\n").map_err(|_| ()),
        McpAuthError::AmbiguousPublication => text.write_str("Credentials may have been published; inspect stored state before retrying.\n").map_err(|_| ()),
        _ => Ok(()),
    }
}

fn logout(text: &mut BoundedOutput, outcome: McpAuthLogoutReceipt) -> Result<(), ()> {
    writeln!(
        text,
        "Local credential removal: {}.\nRemote token revocation: {}.",
        match outcome.local {
            McpAuthLocalRemoval::Unchanged => "unchanged",
            McpAuthLocalRemoval::Removed => "confirmed",
            McpAuthLocalRemoval::Ambiguous => "ambiguous",
            McpAuthLocalRemoval::Failed => "failed",
        },
        match outcome.remote {
            McpAuthRemoteRevocation::NotAttempted => "not attempted",
            McpAuthRemoteRevocation::Confirmed => "confirmed",
            McpAuthRemoteRevocation::Unsupported => "unsupported",
            McpAuthRemoteRevocation::Ambiguous => "ambiguous",
        },
    )
    .map_err(|_| ())?;
    text.write_str("Local removal is not remote revocation. No automatic retry.\n")
        .map_err(|_| ())
}

#[cfg(test)]
mod tests;
