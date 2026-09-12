use super::{Error, MAX_SERVERS, NativeMcpStartup, Result};
use crate::mcp::{auth::McpAuthSelection, startup::NativeMcpStartupCompletion};
use std::sync::{Arc, Mutex};

pub(in crate::mcp::startup) struct IdentityCleanup(pub Arc<Mutex<Vec<RetainedIdentity>>>);
impl Drop for IdentityCleanup {
    fn drop(&mut self) {
        prune(&self.0);
    }
}

fn prune(identities: &Mutex<Vec<RetainedIdentity>>) {
    let removed = {
        let mut identities = identities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        identities
            .extract_if(.., |entry| entry.completion.is_complete())
            .collect::<Vec<_>>()
    };
    drop(removed);
}

pub(in crate::mcp::startup) struct RetainedIdentity {
    server: Box<str>,
    _selection: McpAuthSelection,
    completion: NativeMcpStartupCompletion,
}
impl NativeMcpStartup {
    pub(in crate::mcp::startup) fn retain_authentication_identity(
        &self,
        server: &str,
        selection: McpAuthSelection,
        completion: NativeMcpStartupCompletion,
    ) -> Result<()> {
        self.prune_authentication_identities();
        let mut identities = self
            .authentication_identities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if identities.len() >= MAX_SERVERS
            || identities
                .iter()
                .any(|entry| entry.server.as_ref() == server)
        {
            return Err(Error::Limit);
        }
        identities.push(RetainedIdentity {
            server: server.into(),
            _selection: selection,
            completion,
        });
        Ok(())
    }

    pub(crate) fn prune_authentication_identities(&self) {
        prune(&self.authentication_identities);
    }

    pub(in crate::mcp::startup) fn authentication_identity_slot(&self, server: &str) -> Result<()> {
        self.prune_authentication_identities();
        let identities = self
            .authentication_identities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if identities
            .iter()
            .any(|entry| entry.server.as_ref() == server)
        {
            return Err(Error::Unavailable);
        }
        if identities.len() >= MAX_SERVERS {
            return Err(Error::Limit);
        }
        Ok(())
    }

    /// Controller invokes only after actual replacement, rejected generation or
    /// closure. Historical startup/receipt retention is not live auth selection.
    pub(crate) fn release_authentication_identities(&self) {
        let selected = std::mem::take(
            &mut *self
                .authentication_identities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        drop(selected);
    }
}
