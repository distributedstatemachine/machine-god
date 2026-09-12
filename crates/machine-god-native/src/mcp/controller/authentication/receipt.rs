//! Credential publication and runtime activation are separate observations.

use super::super::{NativeMcpControllerFailure, NativeMcpControllerReceipt};
use crate::mcp::auth::{
    McpAuthError, McpAuthLocalRemoval, McpAuthLogoutReceipt, McpAuthRemoteRevocation,
};
use std::fmt;

pub enum NativeMcpAuthenticationError {
    Selection(NativeMcpControllerFailure),
    Authorization(McpAuthError),
}
impl fmt::Debug for NativeMcpAuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpAuthenticationError { .. }")
    }
}
impl fmt::Display for NativeMcpAuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP authentication command failed")
    }
}
impl std::error::Error for NativeMcpAuthenticationError {}

pub enum NativeMcpAuthenticationReceipt {
    ConfirmationRequired {
        server: Box<str>,
    },
    Authenticated {
        server: Box<str>,
        usable: bool,
        activation: Option<Result<NativeMcpControllerReceipt, NativeMcpControllerFailure>>,
    },
    LoggedOut {
        server: Box<str>,
        outcome: McpAuthLogoutReceipt,
    },
}
impl fmt::Debug for NativeMcpAuthenticationReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpAuthenticationReceipt { .. }")
    }
}
impl NativeMcpAuthenticationReceipt {
    #[must_use]
    pub fn failed(&self) -> bool {
        match self {
            Self::ConfirmationRequired { .. } => false,
            Self::Authenticated {
                usable, activation, ..
            } => {
                !usable
                    || activation.as_ref().is_none_or(|result| match result {
                        Err(_) => true,
                        Ok(receipt) => {
                            receipt.closed_after_publication()
                                || receipt.startup().is_some_and(
                                    crate::mcp::startup::NativeMcpStartupReceipt::has_failures,
                                )
                        }
                    })
            }
            Self::LoggedOut { outcome, .. } => {
                matches!(
                    outcome.local,
                    McpAuthLocalRemoval::Ambiguous | McpAuthLocalRemoval::Failed
                ) || outcome.remote == McpAuthRemoteRevocation::Ambiguous
            }
        }
    }
}
