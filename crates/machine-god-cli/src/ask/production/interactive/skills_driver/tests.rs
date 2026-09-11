use super::*;
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeSkillCatalog, NativeSkillLinkPolicy, NativeSkillRoot, NativeSkillSource,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn snapshot() -> Arc<NativeSkillSnapshot> {
    let path = std::env::temp_dir().join(format!(
        "mg-skills-ui-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    for name in ["first", "second"] {
        fs::create_dir(path.join(name)).unwrap();
        fs::write(
            path.join(name).join("SKILL.md"),
            format!("---\nname: duplicate\ndescription: {name}\n---\nbody"),
        )
        .unwrap();
    }
    let root = NativeSkillRoot::from_directory(
        Arc::new(fs::File::open(&path).unwrap()),
        PathBuf::new(),
        path.clone(),
        path.clone(),
        NativeSkillSource::Managed,
        NativeSkillLinkPolicy::Reject,
    )
    .unwrap();
    let snapshot = NativeSkillCatalog::new(vec![root])
        .unwrap()
        .discover(&CancellationToken::new())
        .unwrap();
    fs::remove_dir_all(path).unwrap();
    Arc::new(snapshot)
}

fn shown(skills: &SkillsUi, selectable: bool) -> InputBinding {
    InputBinding::Skills {
        epoch: skills.epoch.clone(),
        frame: selectable.then(|| skills.picker.view().unwrap().identity),
        query: skills.picker.view().unwrap().mode == NativeSkillPickerMode::Menu,
    }
}

fn selectable(binding: &InputBinding) -> bool {
    matches!(binding, InputBinding::Skills { frame: Some(_), .. })
}

#[test]
fn hidden_or_unflushed_skill_frames_never_acknowledge_selection() {
    let mut skills = SkillsUi::new(Some(snapshot()));
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    assert!(!selectable(&skills.binding()));
    skills.acknowledge(&shown(&skills, false));
    assert!(!selectable(&skills.binding()));
    skills.acknowledge(&shown(&skills, true));
    assert!(selectable(&skills.binding()));
}

#[test]
fn resize_and_navigation_reject_late_skill_frame_acknowledgements() {
    let mut skills = SkillsUi::new(Some(snapshot()));
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    let first = shown(&skills, true);
    skills.invalidate_frame();
    skills.acknowledge(&first);
    assert!(!selectable(&skills.binding()));
    let resized = shown(&skills, true);
    skills.acknowledge(&resized);
    assert!(selectable(&skills.binding()));
    skills.picker.move_selection(true).unwrap();
    assert!(!selectable(&skills.binding()));
    skills.acknowledge(&resized);
    assert!(!selectable(&skills.binding()));
    assert_eq!(skills.picker.view().unwrap().selected, Some(1));
}

#[test]
fn reset_and_reopen_reject_identical_text_earlier_owner_and_frame() {
    let mut skills = SkillsUi::new(Some(snapshot()));
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    let first = shown(&skills, true);
    skills.picker.close();
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    skills.acknowledge(&first);
    assert!(!selectable(&skills.binding()));
    let before_reset = shown(&skills, true);
    skills.reset("", 0);
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    skills.acknowledge(&before_reset);
    assert!(!selectable(&skills.binding()));
    assert!(skills.edit(&before_reset, 0..0, "x", 1).is_err());
    assert_eq!(skills.picker.draft(), "");
}

#[test]
fn exact_duplicate_binding_shifts_only_from_actual_observed_edit() {
    let mut skills = SkillsUi::new(Some(snapshot()));
    skills.reset("$duplicate ", 11);
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    skills.picker.move_selection(true).unwrap();
    let frame = skills.picker.view().unwrap().identity;
    skills.acknowledge(&shown(&skills, true));
    skills.picker.choose(&frame).unwrap();
    let selected = skills.picker.bindings()[0].selection().clone();
    assert!(selected.location().ends_with("second"));
    assert_eq!(skills.picker.draft(), "$duplicate $duplicate ");
    let input = skills.binding();
    skills.edit(&input, 0..11, "", 0).unwrap();
    assert_eq!(skills.picker.draft(), "$duplicate ");
    assert_eq!(skills.picker.bindings()[0].span(), 0..10);
    assert_eq!(skills.picker.bindings()[0].selection(), &selected);
    skills.edit(&input, 0..10, "", 0).unwrap();
    assert!(skills.picker.bindings().is_empty());
}

#[test]
fn menu_query_edits_never_replace_original_native_draft() {
    let mut skills = SkillsUi::new(Some(snapshot()));
    skills.reset("before 🦀 after", 7);
    skills
        .picker
        .open_menu(skills.snapshot.clone().unwrap(), "")
        .unwrap();
    skills
        .edit(&skills.binding(), 0..0, "duplicate", 9)
        .unwrap();
    assert_eq!(skills.picker.draft(), "before 🦀 after");
    assert_eq!(skills.picker.cursor(), 7);
    assert!(skills.picker.bindings().is_empty());
}
