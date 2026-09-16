//! Bounded unsent text, independent of page, editor and input-ACK lifetimes.
use super::view::NativeManagedNavigationError as Error;
use crate::NativeObservedManagedAgent;
use std::fmt;

const MAX_DRAFTS: usize = 128;
const MAX_DRAFT_BYTES: usize = 256 * 1024;
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

/// Borrowed unsent text. Its revision is correlation, never command authority.
#[derive(Clone, Copy, Default)]
pub struct NativeManagedDraftView<'a> {
    pub text: &'a str,
    pub cursor: usize,
    pub revision: u64,
}
impl fmt::Debug for NativeManagedDraftView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeManagedDraftView")
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

struct Draft {
    owner: NativeObservedManagedAgent,
    text: Box<str>,
    cursor: usize,
    revision: u64,
}

/// Retained with the original pending message, including across navigation close.
pub(super) struct Submission {
    owner: NativeObservedManagedAgent,
    revision: u64,
}

#[derive(Default)]
pub(super) struct Drafts {
    entries: Vec<Draft>,
    bytes: usize,
    revision: u64,
}
impl Drafts {
    fn index(&self, owner: &NativeObservedManagedAgent) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.owner.same_conversation(owner))
    }

    pub(super) fn view(&self, owner: &NativeObservedManagedAgent) -> NativeManagedDraftView<'_> {
        self.index(owner)
            .map_or_else(NativeManagedDraftView::default, |index| {
                let entry = &self.entries[index];
                NativeManagedDraftView {
                    text: &entry.text,
                    cursor: entry.cursor,
                    revision: entry.revision,
                }
            })
    }

    pub(super) fn replace(
        &mut self,
        owner: &NativeObservedManagedAgent,
        text: &str,
        cursor: usize,
    ) -> Result<(), Error> {
        if text.len() > MAX_DRAFT_BYTES {
            return Err(Error::DraftCapacity);
        }
        if !text.is_char_boundary(cursor) || text.contains('\0') {
            return Err(Error::InvalidAction);
        }
        let index = self.index(owner);
        let previous = index.map_or(0, |index| self.entries[index].text.len());
        if (index.is_none() && !text.is_empty() && self.entries.len() == MAX_DRAFTS)
            || self.bytes - previous + text.len() > MAX_TOTAL_BYTES
        {
            return Err(Error::DraftCapacity);
        }
        if index.is_some_and(|index| {
            self.entries[index].text.as_ref() == text && self.entries[index].cursor == cursor
        }) {
            return Ok(());
        }
        let revision = self.revision.checked_add(1).ok_or(Error::Exhausted)?;
        if text.is_empty() {
            if let Some(index) = index {
                self.entries.remove(index);
            }
        } else if let Some(index) = index {
            let entry = &mut self.entries[index];
            if entry.text.as_ref() != text {
                entry.text = text.into();
            }
            entry.cursor = cursor;
            entry.revision = revision;
        } else {
            self.entries.push(Draft {
                owner: owner.clone(),
                text: text.into(),
                cursor,
                revision,
            });
        }
        self.bytes = self.bytes - previous + text.len();
        self.revision = revision;
        Ok(())
    }

    pub(super) fn submission(
        &self,
        owner: &NativeObservedManagedAgent,
        text: &str,
    ) -> Option<Submission> {
        let entry = &self.entries[self.index(owner)?];
        (entry.text.as_ref() == text).then(|| Submission {
            owner: owner.clone(),
            revision: entry.revision,
        })
    }

    pub(super) fn accepted(&mut self, submission: &Submission) {
        if let Some(index) = self.index(&submission.owner)
            && self.entries[index].revision == submission.revision
        {
            self.bytes -= self.entries.remove(index).text.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_and_byte_pressure_preserve_existing_drafts_and_refund_only_cleared_text() {
        let owners = NativeObservedManagedAgent::test_observations(MAX_DRAFTS + 1);
        let mut drafts = Drafts::default();
        for owner in &owners[..MAX_DRAFTS] {
            drafts.replace(owner, "x", 0).unwrap();
        }
        assert_eq!(
            drafts.replace(&owners[MAX_DRAFTS], "y", 1),
            Err(Error::DraftCapacity)
        );
        assert_eq!(drafts.view(&owners[0]).text, "x");
        drafts.replace(&owners[0], "", 0).unwrap();
        drafts.replace(&owners[MAX_DRAFTS], "y", 1).unwrap();
        assert_eq!(drafts.entries.len(), MAX_DRAFTS);

        let mut drafts = Drafts::default();
        let text = "x".repeat(MAX_DRAFT_BYTES);
        for owner in &owners[..MAX_TOTAL_BYTES / MAX_DRAFT_BYTES] {
            drafts.replace(owner, &text, 0).unwrap();
        }
        assert_eq!(drafts.bytes, MAX_TOTAL_BYTES);
        assert_eq!(
            drafts.replace(&owners[MAX_DRAFTS], "y", 1),
            Err(Error::DraftCapacity)
        );
        assert_eq!(drafts.view(&owners[0]).text, text);
        drafts.replace(&owners[0], "small", 3).unwrap();
        drafts.replace(&owners[MAX_DRAFTS], "y", 1).unwrap();
        assert_eq!(drafts.bytes, MAX_TOTAL_BYTES - MAX_DRAFT_BYTES + 6);
    }

    #[test]
    fn identity_ignores_head_revision_but_not_generation_or_actual_manager() {
        let owner = NativeObservedManagedAgent::test_observations(1).remove(0);
        let foreign = NativeObservedManagedAgent::test_observations(1).remove(0);
        let mut drafts = Drafts::default();
        drafts.replace(&owner, "α\nbeta", 2).unwrap();
        let mut later = owner.clone();
        later.revision += 1;
        assert_eq!(drafts.view(&later).text, "α\nbeta");
        later.generation += 1;
        assert!(drafts.view(&later).text.is_empty());
        assert!(drafts.view(&foreign).text.is_empty());
        assert_eq!(drafts.replace(&owner, "α", 1), Err(Error::InvalidAction));
        assert_eq!(
            drafts.replace(&owner, "bad\0", 0),
            Err(Error::InvalidAction)
        );
        assert_eq!(drafts.view(&owner).cursor, 2);
    }

    #[test]
    fn acceptance_clears_only_the_exact_submitted_revision() {
        let owner = NativeObservedManagedAgent::test_observations(1).remove(0);
        let mut drafts = Drafts::default();
        drafts.replace(&owner, "send", 4).unwrap();
        assert!(drafts.submission(&owner, "different").is_none());
        let old = drafts.submission(&owner, "send").unwrap();
        drafts.replace(&owner, "next", 4).unwrap();
        drafts.accepted(&old);
        assert_eq!(drafts.view(&owner).text, "next");
        let current = drafts.submission(&owner, "next").unwrap();
        drafts.accepted(&current);
        assert!(drafts.view(&owner).text.is_empty());
        assert_eq!(drafts.bytes, 0);
        assert!(drafts.entries.is_empty());
    }
}
