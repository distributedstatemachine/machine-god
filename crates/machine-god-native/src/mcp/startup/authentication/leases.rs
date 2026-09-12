use super::{Error, MAX_SERVERS, McpAuthLease, NativeMcpStartup, Result};
use crate::mcp::startup::NativeMcpStartupCompletion;
use std::sync::Arc;

pub(in crate::mcp::startup) struct RetainedLease {
    server: Box<str>,
    lease: Arc<McpAuthLease>,
    completion: NativeMcpStartupCompletion,
}

impl NativeMcpStartup {
    fn selected_leases(&self) -> Vec<Arc<RetainedLease>> {
        let snapshot = self
            .authentication_leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // Clock/profile observations run outside the registry lock. A closed
        // peer must not erase the expired/revoked credentials that require a
        // fresh controller generation, so those records remain as tombstones.
        let completed: Vec<_> = snapshot
            .iter()
            .filter(|entry| {
                entry.completion.is_complete() && entry.lease.refresh_due() == Ok(false)
            })
            .collect();
        let (removed, selected) = {
            let mut leases = self
                .authentication_leases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let removed: Vec<_> = leases
                .extract_if(.., |entry| {
                    completed.iter().any(|old| Arc::ptr_eq(old, entry))
                })
                .collect();
            (removed, leases.clone())
        };
        drop(removed);
        selected
    }

    pub(in crate::mcp::startup) fn authentication_slot(&self, server: &str) -> Result<()> {
        self.authentication_identity_slot(server)?;
        let selected = self.selected_leases();
        if selected.iter().any(|entry| entry.server.as_ref() == server) {
            return Err(Error::Unavailable);
        }
        if selected.len() >= MAX_SERVERS {
            return Err(Error::Limit);
        }
        Ok(())
    }

    pub(in crate::mcp::startup) fn retain_authentication(
        &self,
        server: &str,
        lease: Arc<McpAuthLease>,
        completion: NativeMcpStartupCompletion,
    ) -> Result<()> {
        // The build permit serializes selection/insertion; observations may
        // only prune completed entries, never add a competing lease.
        self.authentication_slot(server)?;
        lease.access_token().map_err(|_| Error::Authentication)?;
        self.authentication_leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Arc::new(RetainedLease {
                server: server.into(),
                lease,
                completion,
            }));
        Ok(())
    }

    pub(in crate::mcp::startup) fn retained_authentication_refresh_due(&self) -> Result<bool> {
        let mut due = false;
        for entry in self.selected_leases() {
            // Observe every retained authority even after finding one due;
            // an earlier due lease must not hide a later revoked profile.
            due |= entry
                .lease
                .refresh_due()
                .map_err(|_| Error::Authentication)?;
        }
        Ok(due)
    }
}
