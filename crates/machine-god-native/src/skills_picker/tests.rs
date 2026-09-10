use super::*;

#[test]
fn reset_and_invalid_edits_are_atomic_and_owner_scoped() {
    let mut picker = NativeSkillPicker::new("é".into(), 2).unwrap();
    let original = picker.draft_identity().clone();
    assert_eq!(
        picker.reset("é".into(), 1),
        Err(NativeSkillPickerError::InvalidEdit)
    );
    assert_eq!(picker.draft_identity(), &original);
    assert_eq!(
        picker.apply_edit(&original, 1..2, "x", 2),
        Err(NativeSkillPickerError::InvalidEdit)
    );
    assert_eq!(
        picker.apply_edit(&original, 0..2, "é", 1),
        Err(NativeSkillPickerError::InvalidEdit)
    );
    assert_eq!(picker.draft(), "é");
    picker.reset("é".into(), 2).unwrap();
    assert_ne!(picker.draft_identity(), &original);
    assert_eq!(
        picker.selections(&original).unwrap_err(),
        NativeSkillPickerError::StaleDraft
    );
    let other = NativeSkillPicker::new("é".into(), 2).unwrap();
    assert_ne!(picker.draft_identity(), other.draft_identity());
}

#[test]
fn inline_queries_are_bounded_cursor_prefixes() {
    let text = "before $révnext after";
    let picker = NativeSkillPicker::new(text.into(), "before $rév".len()).unwrap();
    let query = picker.inline_query().unwrap();
    assert_eq!(query.query, "rév");
    assert_eq!(&text[query.span], "$rév");
    for text in ["email$rev", "quote '$rev", "$two words", "no query"] {
        assert!(
            NativeSkillPicker::new(text.into(), text.len())
                .unwrap()
                .inline_query()
                .is_none()
        );
    }
    let text = format!(
        "${}",
        "x".repeat(crate::skills_catalog::MAX_NATIVE_SKILL_QUERY_BYTES)
    );
    assert!(
        NativeSkillPicker::new(text.clone(), text.len())
            .unwrap()
            .inline_query()
            .is_some()
    );
    let text = format!("{text}x");
    assert!(
        NativeSkillPicker::new(text.clone(), text.len())
            .unwrap()
            .inline_query()
            .is_none()
    );
}

#[test]
fn prompt_bounds_and_debug_redact_draft() {
    let text = "x".repeat(MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES);
    let mut picker = NativeSkillPicker::new(text.clone(), text.len()).unwrap();
    let identity = picker.draft_identity().clone();
    assert_eq!(
        picker.apply_edit(&identity, 0..0, "x", 1),
        Err(NativeSkillPickerError::PromptTooLong)
    );
    assert_eq!(picker.draft_identity(), &identity);
    assert_eq!(
        NativeSkillPicker::new(format!("{text}x"), 0).unwrap_err(),
        NativeSkillPickerError::PromptTooLong
    );
    let picker = NativeSkillPicker::new("PRIVATE-DRAFT".into(), 0).unwrap();
    assert!(!format!("{picker:?}").contains("PRIVATE-DRAFT"));
}

#[test]
fn revision_exhaustion_and_invalid_ranges_never_mutate_state() {
    let mut picker = NativeSkillPicker::new("abé".into(), 4).unwrap();
    for range in [4..5, 5..5, 3..4, usize::MAX..usize::MAX] {
        let id = picker.draft_identity().clone();
        assert_eq!(
            picker.apply_edit(&id, range, "", 0),
            Err(NativeSkillPickerError::InvalidEdit)
        );
        assert_eq!(picker.draft(), "abé");
        assert_eq!(picker.draft_identity(), &id);
    }
    picker.identity.revision = u64::MAX;
    let id = picker.draft_identity().clone();
    assert_eq!(
        picker.apply_edit(&id, 0..1, "x", 0),
        Err(NativeSkillPickerError::RevisionExhausted)
    );
    assert_eq!(
        picker.move_cursor(&id, 0),
        Err(NativeSkillPickerError::RevisionExhausted)
    );
    assert_eq!(picker.draft(), "abé");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "supported.rs"]
mod supported;
