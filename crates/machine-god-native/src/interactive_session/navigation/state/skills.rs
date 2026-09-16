//! Exact child-local skill menus over an explicitly supplied observation.
use super::{Error, NativeInteractiveSession, Navigation, Route};
use crate::{NativeManagedEditorIdentity, NativeSkillReference, NativeSkillSnapshot};
use std::{ops::Range, sync::Arc};

impl Navigation {
    pub(in crate::interactive_session) fn set_skills_snapshot(
        &mut self,
        snapshot: Option<Arc<NativeSkillSnapshot>>,
    ) {
        self.skills_snapshot = snapshot;
        if self.open && self.route == Route::Skills {
            let result = self.open_skills().and_then(|()| self.change(true));
            if let Err(error) = result {
                self.error = Some(error);
                self.close();
            }
        }
    }
    pub(super) fn open_skills(&mut self) -> Result<(), Error> {
        let snapshot = self.skills_snapshot.clone().ok_or(Error::Unavailable)?;
        self.drafts
            .open_skills(&self.target()?.observation.clone(), snapshot)?;
        self.skills_cursor = 0;
        self.route = Route::Skills;
        self.result = None;
        Ok(())
    }
    pub(in crate::interactive_session) fn edit_skill_query(
        &mut self,
        editor: &NativeManagedEditorIdentity,
        query: &str,
        cursor: usize,
    ) -> Result<(), Error> {
        if !self.open || *editor != self.frame().editor {
            return Err(Error::StaleFrame);
        }
        if self.route != Route::Skills || !query.is_char_boundary(cursor) || query.contains('\0') {
            return Err(Error::InvalidAction);
        }
        self.drafts
            .query_skills(&self.target()?.observation.clone(), query)?;
        self.skills_cursor = cursor;
        self.change(false)
    }
    pub(in crate::interactive_session) fn edit_draft_range(
        &mut self,
        editor: &NativeManagedEditorIdentity,
        range: Range<usize>,
        inserted: &str,
        cursor: usize,
    ) -> Result<(), Error> {
        if !self.open || *editor != self.frame().editor {
            return Err(Error::StaleFrame);
        }
        if !matches!(self.route, Route::Conversation | Route::Agent(_)) {
            return Err(Error::InvalidAction);
        }
        self.drafts
            .edit(&self.target()?.observation.clone(), range, inserted, cursor)
    }
    pub(super) fn skill_references(
        &self,
        owner: &NativeInteractiveSession,
        text: &str,
    ) -> Result<Vec<NativeSkillReference>, Error> {
        let selections = self.drafts.selections(&self.target()?.observation, text)?;
        let Some(snapshot) = &self.skills_snapshot else {
            return if selections.is_empty() {
                Ok(Vec::new())
            } else {
                Err(Error::Unavailable)
            };
        };
        let catalog = owner.skills_catalog().ok_or(Error::Unavailable)?;
        let plan = crate::NativeSkillInvocationPlan::resolve(text, snapshot, &selections)
            .map_err(|_| Error::InvalidAction)?;
        // Every navigation frame exposes the incomplete-discovery warning;
        // explicit selections remain valid while automatic matching is suppressed.
        plan.references(&catalog).map_err(|_| Error::InvalidAction)
    }
}
