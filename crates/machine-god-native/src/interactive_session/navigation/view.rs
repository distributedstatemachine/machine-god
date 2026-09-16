//! Allocation-bound presentation identities; none retains execution authority.
use crate::{NativeManagedCatalogEntry, NativeManagedCatalogFilter};
use machine_god_core::{
    ManagedConfigure, ManagedCreate, ManagedInspectSection, ManagedLifecycleAction,
    ManagedRelationshipAction, ManagedSubagentResult,
};
use std::{
    fmt,
    sync::{Arc, Weak},
};

pub(super) struct Identity;

/// One editor route, separate from a rendered page and from input ownership.
#[derive(Clone)]
pub struct NativeManagedEditorIdentity {
    pub(super) owner: Weak<Identity>,
    pub(super) epoch: u64,
}
impl PartialEq for NativeManagedEditorIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.owner.ptr_eq(&other.owner) && self.epoch == other.epoch
    }
}
impl Eq for NativeManagedEditorIdentity {}
impl fmt::Debug for NativeManagedEditorIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedEditorIdentity { .. }")
    }
}

/// Exact native projection. Only the owner can issue or acknowledge this frame.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeManagedFrameIdentity {
    pub(super) editor: NativeManagedEditorIdentity,
    pub(super) revision: u64,
}
impl fmt::Debug for NativeManagedFrameIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedFrameIdentity { .. }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedNavigationRoute {
    Catalog(NativeManagedCatalogFilter),
    Agent(ManagedInspectSection),
    ConfirmClose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedNavigationError {
    Unavailable,
    Busy,
    StaleFrame,
    NotDisplayed,
    NoSelection,
    InvalidAction,
    Exhausted,
}
impl fmt::Display for NativeManagedNavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "agent navigation unavailable",
            Self::Busy => "agent navigation operation pending",
            Self::StaleFrame => "agent navigation frame changed",
            Self::NotDisplayed => "agent navigation frame not displayed",
            Self::NoSelection => "agent navigation selection unavailable",
            Self::InvalidAction => "agent navigation action invalid",
            Self::Exhausted => "agent navigation identity exhausted",
        })
    }
}
impl std::error::Error for NativeManagedNavigationError {}

/// Typed human intent. IDs in configuration are checked against the exact row;
/// lifecycle Close is admitted only through a separately displayed confirmation.
pub enum NativeManagedNavigationAction {
    Previous,
    Next,
    NextPage,
    Filter(NativeManagedCatalogFilter),
    Select,
    Back,
    Refresh,
    Inspect(ManagedInspectSection),
    Message(String),
    Create(ManagedCreate),
    Configure(ManagedConfigure),
    Relationship {
        action: ManagedRelationshipAction,
        parent_id: Option<String>,
    },
    Lifecycle(ManagedLifecycleAction),
    ConfirmClose,
}

impl fmt::Debug for NativeManagedNavigationAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let action = match self {
            Self::Previous => "Previous",
            Self::Next => "Next",
            Self::NextPage => "NextPage",
            Self::Filter(_) => "Filter",
            Self::Select => "Select",
            Self::Back => "Back",
            Self::Refresh => "Refresh",
            Self::Inspect(_) => "Inspect",
            Self::Message(_) => "Message",
            Self::Create(_) => "Create",
            Self::Configure(_) => "Configure",
            Self::Relationship { .. } => "Relationship",
            Self::Lifecycle(_) => "Lifecycle",
            Self::ConfirmClose => "ConfirmClose",
        };
        f.debug_struct("NativeManagedNavigationAction")
            .field("action", &action)
            .finish_non_exhaustive()
    }
}

/// Borrowed unsanitized display values. The renderer must escape terminal text.
/// A retained frame identity does not retain this projection or a child runtime.
pub struct NativeManagedNavigationView<'a> {
    pub editor: NativeManagedEditorIdentity,
    pub frame: NativeManagedFrameIdentity,
    pub route: NativeManagedNavigationRoute,
    pub rows: &'a [NativeManagedCatalogEntry],
    pub selected: Option<usize>,
    pub target: Option<&'a NativeManagedCatalogEntry>,
    pub has_next: bool,
    pub busy: bool,
    pub result: Option<&'a ManagedSubagentResult>,
    pub error: Option<NativeManagedNavigationError>,
}

impl fmt::Debug for NativeManagedNavigationView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeManagedNavigationView")
            .field("route", &self.route)
            .field("row_count", &self.rows.len())
            .field("has_next", &self.has_next)
            .field("busy", &self.busy)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

pub(super) fn editor(owner: &Arc<Identity>, epoch: u64) -> NativeManagedEditorIdentity {
    NativeManagedEditorIdentity {
        owner: Arc::downgrade(owner),
        epoch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_expose_navigation_payloads_or_owner_identity() {
        let action = NativeManagedNavigationAction::Message("private draft".into());
        let debug = format!("{action:?}");
        assert!(debug.contains("Message"));
        assert!(!debug.contains("private draft"));
        let identity = Arc::new(Identity);
        let editor = editor(&identity, 567_890);
        let frame = NativeManagedFrameIdentity {
            editor: editor.clone(),
            revision: 456_789,
        };
        let view = NativeManagedNavigationView {
            editor,
            frame,
            route: NativeManagedNavigationRoute::Catalog(NativeManagedCatalogFilter::Current),
            rows: &[],
            selected: None,
            target: None,
            has_next: false,
            busy: false,
            result: None,
            error: None,
        };
        let debug = format!("{view:?} {:?} {:?}", view.frame, view.editor);
        assert!(!debug.contains("567890"));
        assert!(!debug.contains("456789"));
        assert_eq!(Arc::strong_count(&identity), 1);
    }
}
