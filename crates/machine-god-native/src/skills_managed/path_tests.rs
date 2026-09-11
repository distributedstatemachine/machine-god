use super::{FakeGit, Fixture};
use crate::skills_managed::{
    NativeManagedSkills, NativeSkillInstallSource, NativeSkillManagedErrorKind as Error, planning,
};
use machine_god_core::CancellationToken;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn huge_borrowed_cwd_is_rejected_without_copying_or_normalizing_it() {
    let fixture = Fixture::new();
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let small = format!("/{}", "x".repeat(4096));
    let huge = format!("/{}", "x/../".repeat(2 * 1024 * 1024));
    for spelling in ["./skill", ".", ".."] {
        let source = NativeSkillInstallSource::parse(spelling, None).unwrap();
        allocation_counter::measure(|| {});
        let measure = |cwd: &str| {
            allocation_counter::measure(|| {
                assert_eq!(
                    owner
                        .prepare_install(&source, Path::new(cwd), &token)
                        .unwrap_err()
                        .kind,
                    Error::InvalidSource
                );
            })
        };
        let small_allocations = measure(&small);
        let huge_allocations = measure(&huge);
        assert_eq!(huge_allocations.bytes_total, small_allocations.bytes_total);
        assert!(huge_allocations.bytes_total <= 4096, "{huge_allocations:?}");
    }
    assert_eq!(fs::read_dir(fixture.path.join("state")).unwrap().count(), 0);
}

#[test]
fn joined_path_limit_counts_separator_and_original_spelling_before_normalization() {
    let source = NativeSkillInstallSource::parse("./skill", None).unwrap();
    for trailing_slash in [false, true] {
        for length in [4096, 4097] {
            let cwd_len = length - source.source().len() - usize::from(!trailing_slash);
            let cwd = if trailing_slash {
                format!("/{}/", "x".repeat(cwd_len - 2))
            } else {
                format!("/{}", "x".repeat(cwd_len - 1))
            };
            assert_eq!(
                Path::new(&cwd).join(source.source()).as_os_str().len(),
                length
            );
            assert_eq!(
                planning::source_root_name(&source, Path::new(&cwd)),
                if length == 4096 {
                    Ok("skill".to_owned())
                } else {
                    Err(Error::InvalidSource)
                }
            );
        }
    }
    // The raw joined spelling is too long, even though parent normalization would shorten it.
    let cwd = format!("/{}/..", "x".repeat(4085));
    assert_eq!(cwd.len(), 4089);
    assert_eq!(
        planning::source_root_name(&source, Path::new(&cwd)),
        Err(Error::InvalidSource)
    );
}

#[test]
fn bounded_local_path_normalization_and_error_precedence_are_unchanged() {
    for (source, cwd, expected) in [
        ("./skill", "/", Ok("skill")),
        ("./skill", "///", Ok("skill")),
        ("./skill", "/workspace/", Ok("skill")),
        (".", "/workspace/skill", Ok("skill")),
        ("../skill", "/workspace/old", Ok("skill")),
        ("../../skill", "/", Ok("skill")),
        (".", "/", Err(Error::InvalidName)),
        ("..", "relative", Err(Error::InvalidName)),
        ("./skill", "", Ok("skill")),
    ] {
        let source = NativeSkillInstallSource::parse(source, None).unwrap();
        assert_eq!(
            planning::source_root_name(&source, Path::new(cwd)),
            expected.map(str::to_owned)
        );
    }
    let fixture = Fixture::new();
    let owner = fixture.manager();
    let token = CancellationToken::new();
    for (source, cwd, expected) in [
        ("./skill", "relative", Error::InvalidSource),
        ("..", "relative", Error::InvalidName),
    ] {
        assert_eq!(
            owner
                .prepare_install(
                    &NativeSkillInstallSource::parse(source, None).unwrap(),
                    Path::new(cwd),
                    &token,
                )
                .unwrap_err()
                .kind,
            expected
        );
    }
}

#[test]
fn absolute_local_and_git_sources_ignore_unused_invalid_cwd() {
    let fixture = Fixture::new();
    fixture.write("source/SKILL.md", "local");
    let calls = Arc::new(AtomicUsize::new(0));
    let owner = NativeManagedSkills::open(
        &fixture.path.join("state"),
        Some(Arc::new(FakeGit {
            calls: Arc::clone(&calls),
        })),
    )
    .unwrap();
    let token = CancellationToken::new();
    let huge = format!("/{}", "x".repeat(8 * 1024 * 1024));
    for cwd in [huge.as_str(), "relative", ""] {
        for source in [
            fixture.source(),
            NativeSkillInstallSource::parse("owner/repo", None).unwrap(),
        ] {
            let plan = owner
                .prepare_install(&source, Path::new(cwd), &token)
                .unwrap();
            assert_eq!(plan.items().len(), 1);
        }
    }
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    assert_eq!(fs::read_dir(fixture.path.join("state")).unwrap().count(), 0);
}

#[test]
fn cancellation_precedes_path_validation_and_git_effects() {
    let fixture = Fixture::new();
    let owner = fixture.manager();
    let huge = format!("/{}", "x".repeat(8 * 1024 * 1024));
    let token = CancellationToken::new();
    token.cancel();
    for source in [
        NativeSkillInstallSource::parse("./skill", None).unwrap(),
        fixture.source(),
        NativeSkillInstallSource::parse("owner/repo", None).unwrap(),
    ] {
        assert_eq!(
            owner
                .prepare_install(&source, Path::new(&huge), &token)
                .unwrap_err()
                .kind,
            Error::Cancelled
        );
    }
    assert_eq!(fs::read_dir(fixture.path.join("state")).unwrap().count(), 0);
}
