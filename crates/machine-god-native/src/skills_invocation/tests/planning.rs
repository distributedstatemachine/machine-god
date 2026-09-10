use super::super::*;
use crate::skills_catalog::{
    NativeSkillCatalog, NativeSkillLinkPolicy, NativeSkillRoot, NativeSkillSource,
};
use machine_god_core::CancellationToken;
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
            "mg-skill-invocation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn skill(&self, directory: &str, name: &str, description: &str) {
        fs::create_dir_all(self.0.join(directory)).unwrap();
        fs::write(
            self.0.join(directory).join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nBody\n"),
        )
        .unwrap();
    }
    fn catalog(&self) -> NativeSkillCatalog {
        NativeSkillCatalog::new(vec![
            NativeSkillRoot::from_directory(
                Arc::new(fs::File::open(&self.0).unwrap()),
                PathBuf::new(),
                self.0.clone(),
                self.0.clone(),
                NativeSkillSource::WorkspaceShared,
                NativeSkillLinkPolicy::Contained,
            )
            .unwrap(),
        ])
        .unwrap()
    }
    fn snapshot(&self) -> NativeSkillSnapshot {
        self.catalog().discover(&CancellationToken::new()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn names(plan: &NativeSkillInvocationPlan) -> Vec<&str> {
    plan.selections()
        .iter()
        .map(NativeSkillSelection::name)
        .collect()
}

#[test]
fn explicit_order_precedes_stable_automatic_order_and_location_deduplication() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "review", "");
    fixture.skill("beta", "release", "");
    fixture.skill("gamma", "extra", "");
    let snapshot = fixture.snapshot();
    let explicit = [
        snapshot.resolve("extra", None).unwrap(),
        snapshot.resolve("release", None).unwrap(),
    ];
    let plan = NativeSkillInvocationPlan::resolve("$review inspect", &snapshot, &explicit).unwrap();
    assert_eq!(names(&plan), ["extra", "release", "review"]);
    assert!(!plan.automatic_matching_incomplete());
    assert_eq!(
        plan.retained_bytes(),
        plan.selections()
            .iter()
            .map(NativeSkillSelection::retained_bytes)
            .sum::<usize>()
    );
    let review = snapshot.resolve("review", None).unwrap();
    let plan = NativeSkillInvocationPlan::resolve("$review", &snapshot, &[review.clone(), review])
        .unwrap();
    assert_eq!(names(&plan), ["review"]);
}

#[test]
fn duplicate_exact_names_are_suppressed_but_exact_bindings_disambiguate() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "review", "");
    fixture.skill("beta", "review", "");
    let snapshot = fixture.snapshot();
    let plan = NativeSkillInvocationPlan::resolve("$review", &snapshot, &[]).unwrap();
    assert!(plan.selections().is_empty());
    let selected = snapshot.entries()[1].selection();
    let plan =
        NativeSkillInvocationPlan::resolve("$review", &snapshot, std::slice::from_ref(&selected))
            .unwrap();
    assert_eq!(plan.selections(), &[selected]);
}

#[test]
fn distinct_names_matching_casefold_or_normalization_are_all_retained() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "Review", "");
    fixture.skill("beta", "review", "");
    fixture.skill("gamma", "release-notes", "");
    fixture.skill("delta", "release_notes", "");
    let snapshot = fixture.snapshot();
    assert_eq!(
        names(&NativeSkillInvocationPlan::resolve("$REVIEW", &snapshot, &[]).unwrap()),
        ["Review", "review"]
    );
    assert_eq!(
        names(
            &NativeSkillInvocationPlan::resolve("use release notes skill", &snapshot, &[]).unwrap()
        ),
        ["release_notes", "release-notes"]
    );
}

#[test]
fn incomplete_snapshots_suppress_automatic_but_preserve_exact_explicit_choices() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "review", "");
    fixture.skill("beta", "release", "");
    let catalog = fixture.catalog();
    let complete = catalog.discover(&CancellationToken::new()).unwrap();
    let explicit = [complete.resolve("release", None).unwrap()];
    assert_eq!(
        names(&NativeSkillInvocationPlan::resolve("$review", &complete, &explicit).unwrap()),
        ["release", "review"]
    );
    fixture.skill("invalid", "", "");
    let incomplete = catalog.discover(&CancellationToken::new()).unwrap();
    assert!(!incomplete.complete());
    for prompt in ["$review", "ordinary prompt", "use review skill"] {
        let automatic = NativeSkillInvocationPlan::resolve(prompt, &incomplete, &[]).unwrap();
        assert!(automatic.selections().is_empty());
        assert!(automatic.automatic_matching_incomplete());
        let explicit_plan =
            NativeSkillInvocationPlan::resolve(prompt, &incomplete, &explicit).unwrap();
        assert_eq!(names(&explicit_plan), ["release"]);
        assert!(explicit_plan.automatic_matching_incomplete());
    }
}

#[test]
fn foreign_and_stale_duplicate_bindings_are_rejected_before_deduplication() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "review", "old");
    let catalog = fixture.catalog();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    let old = snapshot.resolve("review", None).unwrap();
    let foreign = fixture.snapshot().resolve("review", None).unwrap();
    assert_eq!(
        NativeSkillInvocationPlan::resolve("", &snapshot, &[old.clone(), foreign]).unwrap_err(),
        NativeSkillInvocationError::StaleSelection
    );
    fixture.skill("alpha", "review", "updated");
    let updated = catalog.discover(&CancellationToken::new()).unwrap();
    let current = updated.resolve("review", None).unwrap();
    assert_eq!(
        NativeSkillInvocationPlan::resolve("", &updated, &[current, old]).unwrap_err(),
        NativeSkillInvocationError::StaleSelection
    );
}

#[test]
fn planning_never_reopens_removed_skills_or_changes_raw_prompt() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "review", "");
    let snapshot = fixture.snapshot();
    fs::remove_dir_all(fixture.0.join("alpha")).unwrap();
    let prompt = "  $review\nKeep canonical text".to_owned();
    let plan = NativeSkillInvocationPlan::resolve(&prompt, &snapshot, &[]).unwrap();
    assert_eq!(names(&plan), ["review"]);
    assert_eq!(prompt, "  $review\nKeep canonical text");
    assert!(!format!("{plan:?}").contains("canonical text"));
    assert!(!format!("{plan:?}").contains("review"));
}

#[test]
fn prompt_and_incoming_count_limits_are_checked_before_deduplication() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "review", "");
    let snapshot = fixture.snapshot();
    let selected = snapshot.resolve("review", None).unwrap();
    assert!(
        NativeSkillInvocationPlan::resolve(
            &"x".repeat(MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES),
            &snapshot,
            &[]
        )
        .is_ok()
    );
    assert_eq!(
        NativeSkillInvocationPlan::resolve(
            &"x".repeat(MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES + 1),
            &snapshot,
            &[]
        )
        .unwrap_err(),
        NativeSkillInvocationError::PromptTooLong
    );
    assert!(
        NativeSkillInvocationPlan::resolve(
            "",
            &snapshot,
            &vec![selected.clone(); MAX_NATIVE_SKILL_INVOCATION_SELECTIONS]
        )
        .is_ok()
    );
    assert_eq!(
        NativeSkillInvocationPlan::resolve(
            "",
            &snapshot,
            &vec![selected; MAX_NATIVE_SKILL_INVOCATION_SELECTIONS + 1]
        )
        .unwrap_err(),
        NativeSkillInvocationError::TooManySelections
    );
}

#[test]
fn incoming_bytes_are_checked_even_when_all_selections_would_deduplicate() {
    let fixture = Fixture::new();
    fixture.skill(
        "alpha",
        "review",
        &"d".repeat(crate::skills_metadata::MAX_NATIVE_SKILL_DESCRIPTION_BYTES),
    );
    let snapshot = fixture.snapshot();
    let selected = snapshot.resolve("review", None).unwrap();
    assert_eq!(
        NativeSkillInvocationPlan::resolve(
            "",
            &snapshot,
            &vec![selected; MAX_NATIVE_SKILL_INVOCATION_SELECTIONS]
        )
        .unwrap_err(),
        NativeSkillInvocationError::SelectionBytesExceeded
    );
}

#[test]
fn resulting_selection_count_is_bounded_atomically() {
    let fixture = Fixture::new();
    for index in 0..=MAX_NATIVE_SKILL_INVOCATION_SELECTIONS {
        fixture.skill(
            &format!("item-{index:02}"),
            &format!("skill-{index:02}"),
            "",
        );
    }
    let snapshot = fixture.snapshot();
    let explicit: Vec<_> = snapshot.entries()[..MAX_NATIVE_SKILL_INVOCATION_SELECTIONS]
        .iter()
        .map(crate::skills_catalog::NativeSkillEntry::selection)
        .collect();
    assert_eq!(
        NativeSkillInvocationPlan::resolve("$skill-16", &snapshot, &explicit).unwrap_err(),
        NativeSkillInvocationError::TooManySelections
    );
    assert!(NativeSkillInvocationPlan::resolve("$skill-00", &snapshot, &explicit).is_ok());
}

#[test]
fn unicode_and_spaced_advertised_names_round_trip_through_catalog_plan() {
    let fixture = Fixture::new();
    fixture.skill("alpha", "café", "");
    fixture.skill("beta", "review code", "");
    fixture.skill("gamma", "🦀", "");
    let snapshot = fixture.snapshot();
    for (prompt, expected) in [
        ("$café", "café"),
        ("$review code now", "review code"),
        ("$🦀", "🦀"),
    ] {
        assert_eq!(
            names(&NativeSkillInvocationPlan::resolve(prompt, &snapshot, &[]).unwrap()),
            [expected]
        );
    }
}

#[test]
fn rejected_preflight_does_not_allocate_or_clone_selections() {
    let fixture = Fixture::new();
    fixture.skill(
        "alpha",
        "review",
        &"d".repeat(crate::skills_metadata::MAX_NATIVE_SKILL_DESCRIPTION_BYTES),
    );
    let snapshot = fixture.snapshot();
    let selection = snapshot.resolve("review", None).unwrap();
    let oversized_prompt = "x".repeat(MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES + 1);
    let too_many = vec![selection.clone(); MAX_NATIVE_SKILL_INVOCATION_SELECTIONS + 1];
    let too_large = vec![selection; MAX_NATIVE_SKILL_INVOCATION_SELECTIONS];
    for (prompt, explicit) in [
        (oversized_prompt.as_str(), &[][..]),
        ("", too_many.as_slice()),
        ("", too_large.as_slice()),
    ] {
        allocation_counter::measure(|| {});
        let measured = allocation_counter::measure(|| {
            assert!(NativeSkillInvocationPlan::resolve(prompt, &snapshot, explicit).is_err());
        });
        assert_eq!(measured.count_total, 0, "{measured:?}");
    }
}

#[test]
fn natural_prompt_is_not_copied_or_normalized_once_per_catalog_entry() {
    let fixture = Fixture::new();
    for index in 0..128 {
        let name = if index == 73 {
            "release-notes".to_owned()
        } else {
            format!("catalog-skill-{index:03}")
        };
        fixture.skill(&format!("item-{index:03}"), &name, "");
    }
    let snapshot = fixture.snapshot();
    let mut plan = None;
    allocation_counter::measure(|| {});
    let measured = allocation_counter::measure(|| {
        plan = Some(
            NativeSkillInvocationPlan::resolve(
                "Please invoke the release_notes skill for this patch.",
                &snapshot,
                &[],
            )
            .unwrap(),
        );
    });
    assert_eq!(names(&plan.unwrap()), ["release-notes"]);
    assert!(measured.count_total < 128, "{measured:?}");
}

#[test]
fn resulting_bytes_are_checked_before_copying_automatic_selections() {
    let fixture = Fixture::new();
    let description = "d".repeat(crate::skills_metadata::MAX_NATIVE_SKILL_DESCRIPTION_BYTES);
    for index in 0..MAX_NATIVE_SKILL_INVOCATION_SELECTIONS {
        fixture.skill(
            &format!("item-{index:02}"),
            &format!("skill-{index:02}"),
            &description,
        );
    }
    let snapshot = fixture.snapshot();
    let mut bytes = 0;
    let explicit: Vec<_> = snapshot
        .entries()
        .iter()
        .take_while(|entry| {
            bytes += entry.selection_ref().retained_bytes();
            bytes <= MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES
        })
        .map(crate::skills_catalog::NativeSkillEntry::selection)
        .collect();
    assert!(explicit.len() < MAX_NATIVE_SKILL_INVOCATION_SELECTIONS);
    let prompt = format!("$skill-{:02}", explicit.len());
    assert_eq!(
        NativeSkillInvocationPlan::resolve(&prompt, &snapshot, &explicit).unwrap_err(),
        NativeSkillInvocationError::SelectionBytesExceeded
    );
}
