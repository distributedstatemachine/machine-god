//! Refresh one original conversation without discarding its mutation receipt.
use super::{
    Error, NativeInteractiveSession, Navigation, Pending, Refresh, Route, mutation_receipt,
};
use crate::NativeManagedCatalogPage;
use machine_god_core::ManagedAgentState;

impl Navigation {
    pub(super) fn accept_catalog(
        &mut self,
        result: Result<NativeManagedCatalogPage, crate::NativeManagedCatalogError>,
        targeted: bool,
    ) -> Option<Refresh> {
        match result {
            Ok(page) if targeted => return self.accept_target(page),
            Ok(page) => self.replace_page(page),
            Err(_) => self.reject_catalog(targeted),
        }
        None
    }

    pub(super) fn refresh_target(
        &mut self,
        owner: &mut NativeInteractiveSession,
    ) -> Result<(), Error> {
        let observed = self.target()?.observation.clone();
        let request = owner
            .managed
            .as_mut()
            .ok_or(Error::Unavailable)?
            .agents
            .request_catalog_target(observed)
            .map_err(|_| Error::Unavailable)?;
        self.pending = Some(Pending::Catalog {
            request,
            epoch: self.epoch,
            targeted: true,
        });
        owner.notify();
        Ok(())
    }

    pub(super) fn accept_target(&mut self, page: NativeManagedCatalogPage) -> Option<Refresh> {
        let entry = page.entries.into_iter().next();
        let usable = entry.as_ref().is_some_and(|entry| {
            self.target().is_ok_and(|old| {
                // Successful mutation receipts already retired their form. An
                // open form cannot adopt a target archived by another command.
                let archived_form = self.form.is_some()
                    && old.state != ManagedAgentState::Archived
                    && entry.state == ManagedAgentState::Archived;
                old.observation.same_conversation(&entry.observation) && !archived_form
            })
        });
        if !usable {
            self.reject_catalog(true);
            return None;
        }
        self.rows[self.selected.expect("matching original selection")] = entry.unwrap();
        match self.route {
            Route::Conversation => Some(Refresh::History),
            Route::Agent(section) if !self.result.as_ref().is_some_and(mutation_receipt) => {
                Some(Refresh::Inspect(section))
            }
            _ => None,
        }
    }

    pub(super) fn reject_catalog(&mut self, targeted: bool) {
        self.error = Some(Error::Unavailable);
        if targeted {
            // A failed original-target refresh cannot leave a replacement or
            // stale row as an actionable target. Its receipt remains evidence.
            self.rows.clear();
            self.selected = None;
            self.route = Route::Catalog(self.filter);
            self.form = None;
            self.history.clear();
            if self.change(true).is_err() {
                self.close();
            }
        }
    }
}
