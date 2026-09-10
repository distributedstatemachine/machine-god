use super::super::*;
use crate::skills_catalog::{
    NativeSkillCatalog, NativeSkillLinkPolicy, NativeSkillRoot, NativeSkillSource,
};
use machine_god_core::CancellationToken;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-picker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn skill(&self, directory: &str, name: &str, description: &str) {
        let path = self.0.join(directory);
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nbody"),
        )
        .unwrap();
    }
    fn snapshot(&self) -> Arc<NativeSkillSnapshot> {
        let root = NativeSkillRoot::from_directory(
            Arc::new(fs::File::open(&self.0).unwrap()),
            PathBuf::new(),
            self.0.clone(),
            self.0.clone(),
            NativeSkillSource::Managed,
            NativeSkillLinkPolicy::Reject,
        )
        .unwrap();
        Arc::new(
            NativeSkillCatalog::new(vec![root])
                .unwrap()
                .discover(&CancellationToken::new())
                .unwrap(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture() -> (Fixture, Arc<NativeSkillSnapshot>) {
    let fixture = Fixture::new();
    fixture.skill("review", "review", "review code");
    let snapshot = fixture.snapshot();
    (fixture, snapshot)
}

fn choose(picker: &mut NativeSkillPicker) -> NativeSkillPickerInsertion {
    let frame = picker.view().unwrap().identity;
    picker.acknowledge(&frame).unwrap();
    picker.choose(&frame).unwrap()
}

fn bound(text: &str, cursor: usize, snapshot: Arc<NativeSkillSnapshot>) -> NativeSkillPicker {
    let mut picker = NativeSkillPicker::new(text.into(), cursor).unwrap();
    picker.open_inline(snapshot).unwrap();
    choose(&mut picker);
    picker
}

#[test]
fn exact_ack_required_and_old_frame_cannot_select_after_navigation_or_query() {
    let (_fixture, snapshot) = fixture();
    let mut picker = NativeSkillPicker::new("surrounding text".into(), 0).unwrap();
    picker.open_menu(snapshot, "").unwrap();
    let original = picker.view().unwrap().identity;
    assert_eq!(
        picker.choose(&original).unwrap_err(),
        NativeSkillPickerError::FrameNotAcknowledged
    );
    picker.acknowledge(&original).unwrap();
    picker.move_selection(true).unwrap();
    assert_eq!(
        picker.choose(&original).unwrap_err(),
        NativeSkillPickerError::StaleFrame
    );
    assert_eq!(
        picker.acknowledge(&original),
        Err(NativeSkillPickerError::StaleFrame)
    );
    let current = picker.view().unwrap().identity;
    picker.acknowledge(&current).unwrap();
    picker.query_menu("REVIEW").unwrap();
    assert_eq!(
        picker.choose(&current).unwrap_err(),
        NativeSkillPickerError::StaleFrame
    );
    assert_eq!(picker.draft(), "surrounding text");
    assert_eq!(picker.cursor(), 0);
}

#[test]
fn close_reopen_reset_and_other_owner_reject_old_frames() {
    let (_fixture, snapshot) = fixture();
    let mut picker = NativeSkillPicker::new("$rev".into(), 4).unwrap();
    picker.open_inline(snapshot.clone()).unwrap();
    let old = picker.view().unwrap().identity;
    picker.acknowledge(&old).unwrap();
    picker.close();
    assert_eq!(
        picker.choose(&old).unwrap_err(),
        NativeSkillPickerError::NotOpen
    );
    picker.open_inline(snapshot.clone()).unwrap();
    assert_eq!(
        picker.choose(&old).unwrap_err(),
        NativeSkillPickerError::StaleFrame
    );
    let mut other = NativeSkillPicker::new("$rev".into(), 4).unwrap();
    other.open_inline(snapshot.clone()).unwrap();
    assert_eq!(
        other.acknowledge(&old),
        Err(NativeSkillPickerError::StaleFrame)
    );
    picker.reset("$rev".into(), 4).unwrap();
    picker.open_inline(snapshot).unwrap();
    assert_eq!(
        picker.acknowledge(&old),
        Err(NativeSkillPickerError::StaleFrame)
    );
}

#[test]
fn prefix_completion_preserves_suffix_and_returns_precise_edit() {
    let (_fixture, snapshot) = fixture();
    let mut picker = NativeSkillPicker::new("before $revnext after".into(), 11).unwrap();
    let old = picker.draft_identity().clone();
    picker.open_inline(snapshot).unwrap();
    let insertion = choose(&mut picker);
    assert_eq!(insertion.expected_draft, old);
    assert_eq!(&insertion.updated_draft, picker.draft_identity());
    assert_eq!(insertion.range, 7..11);
    assert_eq!(insertion.inserted, "$review ");
    assert_eq!(insertion.cursor_after, 15);
    assert_eq!(insertion.binding.span(), 7..14);
    assert_eq!(picker.draft(), "before $review next after");
    assert_eq!(
        picker.selections(&old).unwrap_err(),
        NativeSkillPickerError::StaleDraft
    );
    assert_eq!(picker.selections(picker.draft_identity()).unwrap().len(), 1);
    assert!(picker.view().is_none());
}

#[test]
fn unicode_spaced_names_duplicate_locations_and_exact_binding_survive_without_io() {
    let fixture = Fixture::new();
    fixture.skill("a", "réview code", "PRIVATE-DESCRIPTION");
    fixture.skill("b", "réview code", "different location");
    let snapshot = fixture.snapshot();
    let mut picker = NativeSkillPicker::new("left right".into(), 4).unwrap();
    picker.open_menu(snapshot.clone(), "réview").unwrap();
    assert_eq!(picker.view().unwrap().total_matches, 2);
    picker.move_selection(true).unwrap();
    let view = picker.view().unwrap();
    assert!(!format!("{view:?}").contains("PRIVATE-DESCRIPTION"));
    drop(fixture); // selection, edits and submission remain entirely inert.
    let insertion = choose(&mut picker);
    assert_eq!(picker.draft(), "left $réview code right");
    assert_eq!(
        insertion.binding.selection(),
        snapshot.entries()[1].selection_ref()
    );
    assert!(!format!("{insertion:?}").contains("réview"));
    assert_eq!(
        picker.selections(picker.draft_identity()).unwrap()[0],
        snapshot.entries()[1].selection()
    );
    let cursor = insertion.binding.span().end;
    picker
        .move_cursor(&picker.draft_identity().clone(), cursor)
        .unwrap();
    assert!(picker.inline_query().is_none());
}

#[test]
fn identical_token_deletion_never_transfers_binding() {
    let (_fixture, snapshot) = fixture();
    let mut picker = bound("$review $rev", 12, snapshot.clone());
    assert_eq!(picker.bindings()[0].span(), 8..15);
    let id = picker.draft_identity().clone();
    picker.apply_edit(&id, 0..8, "", 0).unwrap();
    assert_eq!(picker.draft(), "$review ");
    assert_eq!(picker.bindings()[0].span(), 0..7);

    let mut picker = bound("$review $rev", 12, snapshot);
    let id = picker.draft_identity().clone();
    picker.apply_edit(&id, 7..16, "", 7).unwrap();
    assert_eq!(picker.draft(), "$review");
    assert!(picker.bindings().is_empty());
    assert!(
        picker
            .selections(picker.draft_identity())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn adjacency_requires_ascii_separator_and_overlaps_invalidate() {
    let (_fixture, snapshot) = fixture();
    for (range, inserted, preserved) in [
        (0..0, "x ", true),
        (0..0, "x", false),
        (7..7, " ", true),
        (7..7, "x", false),
        (7..7, "\u{a0}", false),
        (3..3, "x", false),
        (1..2, "r", false),
    ] {
        let mut picker = bound("$rev", 4, snapshot.clone());
        let id = picker.draft_identity().clone();
        let cursor = range.start + inserted.len();
        picker.apply_edit(&id, range, inserted, cursor).unwrap();
        assert_eq!(
            !picker.bindings().is_empty(),
            preserved,
            "inserted {inserted:?}"
        );
    }
    let mut picker = bound("x $rev", 6, snapshot.clone());
    picker
        .apply_edit(&picker.draft_identity().clone(), 1..2, "", 1)
        .unwrap();
    assert!(picker.bindings().is_empty());
    let mut picker = bound("$rev next", 4, snapshot);
    picker
        .apply_edit(&picker.draft_identity().clone(), 7..8, "", 7)
        .unwrap();
    assert!(picker.bindings().is_empty());
}

#[test]
fn exact_inline_edit_refreshes_query_and_invalidates_ack() {
    let (_fixture, snapshot) = fixture();
    let mut picker = NativeSkillPicker::new("$r".into(), 2).unwrap();
    picker.open_inline(snapshot).unwrap();
    let old = picker.view().unwrap().identity;
    picker.acknowledge(&old).unwrap();
    picker
        .apply_edit(&picker.draft_identity().clone(), 2..2, "ev", 4)
        .unwrap();
    assert_eq!(picker.view().unwrap().query, "rev");
    assert_eq!(
        picker.choose(&old).unwrap_err(),
        NativeSkillPickerError::StaleFrame
    );
    choose(&mut picker);
    assert_eq!(picker.draft(), "$review ");
    picker.close();
    assert_eq!(picker.bindings().len(), 1);
    picker
        .reset(picker.draft().to_owned(), picker.cursor())
        .unwrap();
    assert!(picker.bindings().is_empty());
}

#[test]
fn all_catalog_rows_are_reachable_after_first_window() {
    let fixture = Fixture::new();
    for index in 0..260 {
        fixture.skill(&format!("s{index:03}"), &format!("skill{index:03}"), "");
    }
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.entries().len(), 260);
    let mut picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    picker.open_menu(snapshot.clone(), "").unwrap();
    assert_eq!(picker.view().unwrap().rows.len(), 128);
    for _ in 0..259 {
        picker.move_selection(true).unwrap();
    }
    let view = picker.view().unwrap();
    assert_eq!(view.window_start, 256);
    assert_eq!(view.rows.len(), 4);
    assert_eq!(view.selected, Some(259));
    let insertion = choose(&mut picker);
    assert_eq!(
        insertion.binding.selection(),
        snapshot.entries()[259].selection_ref()
    );
    picker.open_menu(snapshot, "skill259").unwrap();
    assert_eq!(picker.view().unwrap().total_matches, 1);
    picker.query_menu("no results").unwrap();
    let frame = picker.view().unwrap().identity;
    picker.acknowledge(&frame).unwrap();
    assert_eq!(
        picker.choose(&frame).unwrap_err(),
        NativeSkillPickerError::NoSelection
    );
}

#[test]
fn query_binding_count_and_aggregate_bytes_reject_atomically() {
    let (_fixture, snapshot) = fixture();
    let mut picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    picker.open_menu(snapshot.clone(), "").unwrap();
    let frame = picker.view().unwrap().identity;
    assert_eq!(
        picker.query_menu(&"x".repeat(1025)),
        Err(NativeSkillPickerError::InvalidQuery)
    );
    assert_eq!(picker.view().unwrap().identity, frame);
    for _ in 0..16 {
        picker.open_menu(snapshot.clone(), "").unwrap();
        choose(&mut picker);
    }
    picker.open_menu(snapshot, "").unwrap();
    let frame = picker.view().unwrap().identity;
    picker.acknowledge(&frame).unwrap();
    let draft = picker.draft().to_owned();
    assert_eq!(
        picker.choose(&frame).unwrap_err(),
        NativeSkillPickerError::TooManySelections
    );
    assert_eq!(picker.draft(), draft);
    assert_eq!(picker.bindings().len(), 16);

    let fixture = Fixture::new();
    fixture.skill("long", "long", &"d".repeat(4096));
    let snapshot = fixture.snapshot();
    let mut picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    let mut exceeded = false;
    for _ in 0..16 {
        picker.open_menu(snapshot.clone(), "").unwrap();
        let frame = picker.view().unwrap().identity;
        picker.acknowledge(&frame).unwrap();
        match picker.choose(&frame) {
            Ok(_) => (),
            Err(NativeSkillPickerError::SelectionBytesExceeded) => {
                exceeded = true;
                break;
            }
            result => panic!("unexpected {result:?}"),
        }
    }
    assert!(exceeded);
}

#[test]
fn incomplete_snapshot_is_visible_but_exact_rows_remain_selectable() {
    let (fixture, _) = fixture();
    fixture.skill("bad", "", "bad");
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    let mut picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    picker.open_menu(snapshot.clone(), "review").unwrap();
    assert!(picker.view().unwrap().discovery_incomplete);
    choose(&mut picker);
    let selections = picker.selections(picker.draft_identity()).unwrap();
    let plan = crate::skills_invocation::NativeSkillInvocationPlan::resolve(
        picker.draft(),
        &snapshot,
        &selections,
    )
    .unwrap();
    assert!(plan.automatic_matching_incomplete());
    assert_eq!(plan.selections().len(), 1);
}

#[test]
fn choices_preserve_selection_order_and_disjoint_unicode_edit_positions() {
    let fixture = Fixture::new();
    fixture.skill("a", "alpha", "");
    fixture.skill("z", "zeta", "");
    let snapshot = fixture.snapshot();
    let mut picker = NativeSkillPicker::new("$zeta".into(), 5).unwrap();
    picker.open_inline(snapshot.clone()).unwrap();
    choose(&mut picker);
    picker.open_menu(snapshot.clone(), "alpha").unwrap();
    choose(&mut picker);
    assert_eq!(picker.draft(), "$zeta $alpha ");
    let id = picker.draft_identity().clone();
    picker.apply_edit(&id, 0..0, "é ", 3).unwrap();
    assert_eq!(picker.bindings()[0].span(), 3..8);
    assert_eq!(picker.bindings()[1].span(), 9..15);
    let selections = picker.selections(picker.draft_identity()).unwrap();
    assert_eq!(
        selections,
        [
            snapshot.entries()[1].selection(),
            snapshot.entries()[0].selection()
        ]
    );
    let text = picker.draft().to_owned();
    let id = picker.draft_identity().clone();
    picker.apply_edit(&id, 5..5, "", 5).unwrap();
    assert_eq!(picker.draft(), text);
    assert_eq!(picker.bindings().len(), 2);
}

#[test]
fn insertion_prompt_limit_retains_acknowledged_menu_and_previous_draft() {
    let (_fixture, snapshot) = fixture();
    let text = format!(
        "{}$rev",
        " ".repeat(MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES - 4)
    );
    let mut picker = NativeSkillPicker::new(text.clone(), text.len()).unwrap();
    picker.open_inline(snapshot).unwrap();
    let frame = picker.view().unwrap().identity;
    picker.acknowledge(&frame).unwrap();
    let id = picker.draft_identity().clone();
    assert_eq!(
        picker.choose(&frame).unwrap_err(),
        NativeSkillPickerError::PromptTooLong
    );
    assert_eq!(picker.draft(), text);
    assert_eq!(picker.draft_identity(), &id);
    assert_eq!(picker.view().unwrap().identity, frame);
    assert!(picker.bindings().is_empty());
}

#[test]
fn punctuation_and_existing_whitespace_follow_pinned_separator_rule() {
    let (_fixture, snapshot) = fixture();
    for (suffix, expected) in [
        ("", "$review "),
        (" rest", "$review rest"),
        (", rest", "$review, rest"),
        ("_suffix", "$review _suffix"),
        ("é", "$review é"),
    ] {
        let text = format!("$rev{suffix}");
        let picker = bound(&text, 4, snapshot.clone());
        assert_eq!(picker.draft(), expected);
        assert_eq!(picker.bindings()[0].span(), 0..7);
    }
}
