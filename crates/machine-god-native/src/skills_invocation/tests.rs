use super::*;

#[test]
fn pinned_sigil_and_natural_forms_match_without_fuzzy_activation() {
    for (prompt, name) in [
        ("$review inspect this patch", "review"),
        ("please use the release-notes skill", "release-notes"),
        ("apply review skill", "review"),
        ("activate the review skill", "review"),
        ("invoke release_notes skill", "release-notes"),
        (
            "run the release notes skill for this patch",
            "release-notes",
        ),
        ("/review inspect this patch", "review"),
        (" \t\r\n$REVIEW: inspect", "review"),
        ("Please, USE the review skill", "review"),
        ("PLEASE:invoke review skill", "review"),
        ("Please-use review skill", "review"),
        ("use the review skill", "the review"),
    ] {
        assert!(
            reference::PromptReference::new(prompt).matches(name),
            "{prompt:?}"
        );
    }
}

#[test]
fn negated_quoted_incidental_and_continuation_forms_remain_text() {
    for prompt in [
        "Do not use the review skill.",
        "Please do not apply the review skill.",
        "Explain why \"use the review skill\" is unsafe.",
        "\"use the review skill\" is only an example.",
        "A pasted example says: use the review skill.",
        "Please use the reviewer skill.",
        "Please use the review skills.",
        "Please use the review skillful workflow.",
        "Do not $review this patch.",
        "The literal `$review` should remain text.",
        "review this patch",
        "$reviewer",
        "$review_extra",
        "$review-more",
        "/review9",
        "Please ‘use review skill’",
        "“use review skill”",
        "`use review skill`",
        "Pleaseuse review skill",
        "use review skillful",
    ] {
        assert!(
            !reference::PromptReference::new(prompt).matches("review"),
            "{prompt:?}"
        );
    }
}

#[test]
fn sigils_preserve_exact_unicode_and_spaces_with_ascii_only_casefolding() {
    for (prompt, name, expected) in [
        ("$café check", "café", true),
        ("$CAFé check", "café", true),
        ("$CAFÉ check", "café", false),
        ("$🦀 check", "🦀", true),
        ("$review code now", "review code", true),
        ("$review  code now", "review code", false),
        ("$reviewé", "review", true),
        ("\u{a0}$review", "review", false),
        ("Use café skill", "café", true),
        ("Use release_notes skill", "release-notes", true),
        ("Use 🦀 skill", "🦀", false),
    ] {
        assert_eq!(
            reference::PromptReference::new(prompt).matches(name),
            expected,
            "{prompt:?} / {name:?}"
        );
    }
}

#[test]
fn normalized_reference_checks_all_valid_name_extents() {
    let reference = reference::PromptReference::new("Use review skill guide skill to proceed");
    assert!(reference.matches("review"));
    assert!(reference.matches("review skill guide"));
    assert!(!reference.matches("guide"));
    assert!(!reference.matches("review guide"));
    assert!(!reference::PromptReference::new("use skill").matches("---"));
}

#[test]
fn inclusive_byte_accounting_and_overflow_are_checked() {
    assert_eq!(
        admitted_bytes(0, MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES).unwrap(),
        MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES
    );
    assert_eq!(
        admitted_bytes(MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES, 1),
        Err(NativeSkillInvocationError::SelectionBytesExceeded)
    );
    assert_eq!(
        admitted_bytes(usize::MAX, 1),
        Err(NativeSkillInvocationError::SelectionBytesExceeded)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "tests/planning.rs"]
mod planning;
