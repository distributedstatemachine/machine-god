use super::{super::*, check_frame};
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeSkillCatalog, NativeSkillLinkPolicy, NativeSkillPicker, NativeSkillRoot,
    NativeSkillSnapshot, NativeSkillSource,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-skills-view-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn skill(&self, directory: &str, name: &str, description: &str) {
        let path = self.0.join(directory);
        fs::create_dir_all(&path).unwrap();
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
    fn picker(&self, query: &str) -> NativeSkillPicker {
        let mut picker = NativeSkillPicker::new("draft remains unchanged".into(), 5).unwrap();
        picker.open_menu(self.snapshot(), query).unwrap();
        picker
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn exact_names_descriptions_locations_and_query_are_projected_without_effects() {
    let fixture = Fixture::new();
    fixture.skill("a-location", "réview code", "first description");
    fixture.skill("b-location", "réview code", "second description");
    let picker = fixture.picker("réview");
    let view = picker.view().unwrap();
    let identity = view.identity.clone();
    let draft = picker.draft().to_owned();
    drop(fixture); // The renderer needs no source filesystem after discovery.
    let frame = render(&view, 300, 12).unwrap();
    let text = check_frame(&frame, 300, 12);
    assert!(text.contains("> 1. réview code — first description"));
    assert!(text.contains("  2. réview code — second description"));
    assert!(text.contains("a-location"));
    assert!(text.contains("b-location"));
    assert!(text.contains("Search: réview"));
    assert!(text.contains("1/2 Skills · rows 1-2"));
    assert!(text.contains("Enter select"));
    assert!(frame.selection_visible);
    assert_eq!(frame.identity, identity);
    assert_eq!(picker.draft(), draft);
    assert!(!format!("{frame:?}").contains("réview"));
}

#[test]
fn basename_previews_distinguish_duplicate_names_with_clipped_common_prefixes() {
    let fixture = Fixture::new();
    fixture.skill("alpha-location", "same-name", "description");
    fixture.skill("beta-location", "same-name", "description");
    let picker = fixture.picker("");
    let frame = render(&picker.view().unwrap(), 32, 10).unwrap();
    let text = check_frame(&frame, 32, 10);
    assert!(text.contains("@ alpha-location | "));
    assert!(text.contains("@ beta-location | "));
    assert!(text.contains("> 1. same-name"));
    assert!(text.contains("  2. same-name"));
    assert!(frame.selection_visible);
}

#[test]
fn selection_after_native_first_window_is_visible_and_uses_absolute_number() {
    let fixture = Fixture::new();
    for index in 0..260 {
        fixture.skill(
            &format!("location-{index:03}"),
            &format!("skill-{index:03}"),
            "description",
        );
    }
    let mut picker = fixture.picker("");
    for _ in 0..259 {
        picker.move_selection(true).unwrap();
    }
    let view = picker.view().unwrap();
    assert_eq!(view.window_start, 256);
    let frame = render(&view, 200, 5).unwrap();
    let text = check_frame(&frame, 200, 5);
    assert!(text.contains("> 260. skill-259"));
    assert!(text.contains("location-259"));
    assert!(!text.contains("skill-256"));
    assert!(text.contains("260/260 Skills · rows 260-260"));
    assert!(frame.selection_visible);
}

#[test]
fn selected_row_slides_through_supplied_window_in_small_viewport() {
    let fixture = Fixture::new();
    for index in 0..128 {
        fixture.skill(
            &format!("location-{index:03}"),
            &format!("skill-{index:03}"),
            "",
        );
    }
    let mut picker = fixture.picker("");
    for selected in 0..128 {
        let view = picker.view().unwrap();
        let frame = render(&view, 160, 7).unwrap();
        let text = check_frame(&frame, 160, 7);
        assert!(text.contains(&format!("> {}. skill-{selected:03}", selected + 1)));
        assert!(text.contains(&format!("location-{selected:03}")));
        assert!(frame.selection_visible);
        picker.move_selection(true).unwrap();
    }
}

#[test]
fn incomplete_notice_is_never_displaced_even_in_one_cell_viewport() {
    let fixture = Fixture::new();
    fixture.skill("skill", "review", "description");
    let picker = fixture.picker("");
    let mut view = picker.view().unwrap();
    view.discovery_incomplete = true;
    for columns in 2..=24 {
        for rows in 1..=8 {
            let frame = render(&view, columns, rows).unwrap();
            let text = check_frame(&frame, columns, rows);
            assert!(text.starts_with('!'));
            assert_eq!(
                frame.selection_visible,
                columns >= MIN_SELECT_COLUMNS && rows >= 6
            );
            if frame.selection_visible {
                assert!(text.contains("> 1. review"));
            }
        }
    }
    let frame = render(&view, 2, 1).unwrap();
    assert_eq!(frame.bytes, b"!");
    assert_eq!(frame.height, 0);
    assert!(!frame.selection_visible);
}

#[test]
fn tiny_complete_frames_are_nonselectable_and_never_add_scroll_rows() {
    let fixture = Fixture::new();
    fixture.skill("skill", "review", "description");
    let picker = fixture.picker("");
    let view = picker.view().unwrap();
    for columns in 2..=20 {
        for rows in 1..=7 {
            let frame = render(&view, columns, rows).unwrap();
            check_frame(&frame, columns, rows);
            assert_eq!(
                frame.selection_visible,
                columns >= MIN_SELECT_COLUMNS && rows >= 5
            );
        }
    }
}

#[test]
fn all_untrusted_fields_are_terminal_safe_and_unicode_width_bounded() {
    let fixture = Fixture::new();
    fixture.skill("path-\u{202e}\u{1b}-tail", "base", "base");
    let picker = fixture.picker("");
    let mut view = picker.view().unwrap();
    let mut entry = view.rows[0].clone();
    entry.metadata.name = "👩‍💻日本é\u{1b}]52;c;PRIVATE\u{7}\u{202e}".into();
    entry.metadata.description = "desc\n\r\t\u{85}\u{2066}\\".into();
    view.rows = vec![&entry];
    view.query = "q\u{1b}[2J\n\u{202e}";
    let frame = render(&view, 400, 8).unwrap();
    let text = check_frame(&frame, 400, 8);
    assert!(text.contains("👩‍💻日本é"));
    assert!(text.contains("\\u{1b}"));
    assert!(text.contains("\\u{202e}"));
    assert!(text.contains("desc\\n\\r\\t"));
    assert!(!text.contains('\u{202e}'));
    for columns in 16..=40 {
        check_frame(&render(&view, columns, 8).unwrap(), columns, 8);
    }
    assert!(!format!("{frame:?}").contains("PRIVATE"));
}

#[test]
fn large_metadata_and_combining_clusters_obey_independent_byte_and_cell_bounds() {
    let fixture = Fixture::new();
    for index in 0..128 {
        fixture.skill(
            &format!("location-{index:03}"),
            &"n".repeat(256),
            &"d".repeat(4096),
        );
    }
    let picker = fixture.picker("");
    let mut view = picker.view().unwrap();
    let frame = render(&view, u16::MAX, u16::MAX).unwrap();
    check_frame(&frame, u16::MAX, u16::MAX);
    assert!(frame.bytes.len() < MAX_OUTPUT_BYTES);
    let mut entry = view.rows[0].clone();
    entry.metadata.description = format!("a{}", "\u{301}".repeat(2047));
    view.rows = vec![&entry];
    view.total_matches = 1;
    let frame = render(&view, 500, 5).unwrap();
    assert!(check_frame(&frame, 500, 5).contains('…'));
}

#[test]
fn empty_and_incomplete_results_never_enable_selection_or_claim_global_absence() {
    let fixture = Fixture::new();
    let picker = fixture.picker("missing");
    let mut view = picker.view().unwrap();
    for incomplete in [false, true] {
        view.discovery_incomplete = incomplete;
        let frame = render(&view, 100, 8).unwrap();
        let text = check_frame(&frame, 100, 8);
        assert!(text.contains("No matching observed skills"));
        assert!(text.contains("Search: missing"));
        assert!(!frame.selection_visible);
        assert_eq!(text.starts_with('!'), incomplete);
    }
}

#[test]
fn invalid_views_and_zero_width_fail_with_fixed_redacted_errors() {
    let fixture = Fixture::new();
    fixture.skill("skill", "name", "description");
    let picker = fixture.picker("");
    for (columns, rows) in [(0, 10), (1, 10), (80, 0)] {
        assert_eq!(
            render(&picker.view().unwrap(), columns, rows).unwrap_err(),
            RenderError::InvalidDimensions
        );
    }
    let mut view = picker.view().unwrap();
    view.rows = vec![view.rows[0]; 129];
    assert_eq!(render(&view, 80, 20).unwrap_err(), RenderError::InvalidView);
    let query = "PRIVATE".repeat(200);
    let mut view = picker.view().unwrap();
    view.query = &query;
    assert_eq!(render(&view, 80, 20).unwrap_err(), RenderError::InvalidView);
    for (start, selected, total) in [
        (usize::MAX, Some(0), 1),
        (0, Some(1), 1),
        (0, None, 1),
        (0, Some(0), 0),
        (0, Some(0), 1025),
    ] {
        let mut view = picker.view().unwrap();
        view.window_start = start;
        view.selected = selected;
        view.total_matches = total;
        assert_eq!(render(&view, 80, 20).unwrap_err(), RenderError::InvalidView);
    }
    let mut view = picker.view().unwrap();
    let mut entry = view.rows[0].clone();
    entry.metadata.name = "n".repeat(257);
    view.rows = vec![&entry];
    assert_eq!(render(&view, 80, 20).unwrap_err(), RenderError::InvalidView);
    assert_eq!(
        RenderError::InvalidView.to_string(),
        "skills view unavailable"
    );
}
