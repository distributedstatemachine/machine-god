//! Effect-free invocation planning over an already observed skill catalog.
//!
//! Explicit bindings have already been tied to the submitted prompt's token
//! spans and acknowledged frames by the caller. This planner additionally checks
//! their exact catalog authority/revision. It reads no skill, constructs no
//! filesystem authority, and grants no tool permission.

use std::{collections::BTreeMap, fmt};

use crate::skills_catalog::{NativeSkillSelection, NativeSkillSnapshot};

#[path = "skills_invocation/reference.rs"]
mod reference;
#[cfg(test)]
#[path = "skills_invocation/tests.rs"]
mod tests;

pub const MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES: usize = 256 * 1_024;
pub const MAX_NATIVE_SKILL_INVOCATION_SELECTIONS: usize = 16;
pub const MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES: usize = 64 * 1_024;

/// Fixed failures without prompt text, metadata, locations or revisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillInvocationError {
    PromptTooLong,
    TooManySelections,
    SelectionBytesExceeded,
    StaleSelection,
}

impl fmt::Display for NativeSkillInvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill invocation: {self:?}")
    }
}
impl std::error::Error for NativeSkillInvocationError {}

/// An owned bounded list, not materialized instructions or an execution grant.
#[derive(Clone, Debug)]
pub struct NativeSkillInvocationPlan {
    selections: Vec<NativeSkillSelection>,
    retained_bytes: usize,
    automatic_matching_incomplete: bool,
}

type Result<T> = std::result::Result<T, NativeSkillInvocationError>;

impl NativeSkillInvocationPlan {
    /// Preserves explicit order, then appends automatic matches in catalog order,
    /// deduplicating by exact location. Every explicit binding is validated even
    /// if a prior binding already selected the same location.
    ///
    /// Incomplete discovery suppresses all automatic matching but preserves
    /// validated explicit choices. The caller must visibly report
    /// `automatic_matching_incomplete()`; absence of automatic selections is not
    /// evidence that a name is absent or unambiguous.
    ///
    /// # Errors
    /// Rejects oversized prompts, incoming pre-deduplication count/bytes, stale or
    /// foreign explicit bindings, and excessive resulting selections/bytes.
    /// All validation and aggregate admission precede selection cloning.
    pub fn resolve(
        prompt: &str,
        snapshot: &NativeSkillSnapshot,
        explicit: &[NativeSkillSelection],
    ) -> Result<Self> {
        preflight(prompt, explicit)?;
        for selected in explicit {
            if !snapshot
                .entries()
                .iter()
                .any(|entry| entry.selection_ref() == selected)
            {
                return Err(NativeSkillInvocationError::StaleSelection);
            }
        }
        let mut borrowed = BorrowedPlan::default();
        for selected in explicit {
            borrowed.add(selected)?;
        }
        if snapshot.complete() {
            borrowed.automatic(prompt, snapshot)?;
        }
        Ok(Self {
            selections: borrowed.selections.into_iter().cloned().collect(),
            retained_bytes: borrowed.bytes,
            automatic_matching_incomplete: !snapshot.complete(),
        })
    }

    #[must_use]
    pub fn selections(&self) -> &[NativeSkillSelection] {
        &self.selections
    }
    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    #[must_use]
    pub const fn automatic_matching_incomplete(&self) -> bool {
        self.automatic_matching_incomplete
    }
}

fn preflight(prompt: &str, explicit: &[NativeSkillSelection]) -> Result<()> {
    if prompt.len() > MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES {
        return Err(NativeSkillInvocationError::PromptTooLong);
    }
    if explicit.len() > MAX_NATIVE_SKILL_INVOCATION_SELECTIONS {
        return Err(NativeSkillInvocationError::TooManySelections);
    }
    let mut bytes = 0_usize;
    for selection in explicit {
        bytes = admitted_bytes(bytes, selection.retained_bytes())?;
    }
    Ok(())
}

fn admitted_bytes(current: usize, next: usize) -> Result<usize> {
    current
        .checked_add(next)
        .filter(|bytes| *bytes <= MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES)
        .ok_or(NativeSkillInvocationError::SelectionBytesExceeded)
}

#[derive(Default)]
struct BorrowedPlan<'a> {
    selections: Vec<&'a NativeSkillSelection>,
    bytes: usize,
}

impl<'a> BorrowedPlan<'a> {
    fn add(&mut self, selected: &'a NativeSkillSelection) -> Result<()> {
        if self
            .selections
            .iter()
            .any(|existing| existing.location() == selected.location())
        {
            return Ok(());
        }
        if self.selections.len() == MAX_NATIVE_SKILL_INVOCATION_SELECTIONS {
            return Err(NativeSkillInvocationError::TooManySelections);
        }
        self.bytes = admitted_bytes(self.bytes, selected.retained_bytes())?;
        self.selections.push(selected);
        Ok(())
    }

    fn automatic(&mut self, prompt: &str, snapshot: &'a NativeSkillSnapshot) -> Result<()> {
        let reference = reference::PromptReference::new(prompt);
        if !reference.can_match() {
            return Ok(());
        }
        let mut counts = BTreeMap::new();
        for entry in snapshot.entries() {
            *counts
                .entry(entry.metadata.name.as_str())
                .or_insert(0_usize) += 1;
        }
        for entry in snapshot.entries() {
            let name = entry.metadata.name.as_str();
            if counts.get(name) == Some(&1) && reference.matches(name) {
                self.add(entry.selection_ref())?;
            }
        }
        Ok(())
    }
}
