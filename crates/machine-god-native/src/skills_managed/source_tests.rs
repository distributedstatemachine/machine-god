use super::{Error, NativeSkillInstallSource, NativeSkillSourceKind};

#[test]
fn explicit_filter_preserves_empty_and_equal_filter_semantics() {
    for input in ["./local", "owner/repo", "owner/repo@"] {
        assert_eq!(
            NativeSkillInstallSource::parse(input, None),
            NativeSkillInstallSource::parse(input, Some(""))
        );
    }
    for input in [
        "owner/repo@review",
        "https://skills.sh/owner/repo/review",
        "npx skills add owner/repo@review --skill review",
    ] {
        for explicit in [None, Some(""), Some("review")] {
            let source = NativeSkillInstallSource::parse(input, explicit).unwrap();
            assert_eq!(source.kind(), NativeSkillSourceKind::Git);
            assert_eq!(source.source(), "https://github.com/owner/repo.git");
            assert_eq!(source.filter(), Some("review"));
        }
    }
    let source = NativeSkillInstallSource::parse("./local", Some("review")).unwrap();
    assert_eq!(source.kind(), NativeSkillSourceKind::Local);
    assert_eq!(source.source(), "./local");
    assert_eq!(source.filter(), Some("review"));
}

#[test]
fn explicit_filter_limit_counts_utf8_bytes_without_trimming() {
    for name in ["x".repeat(256), "é".repeat(128), " skill ".to_owned()] {
        let source = NativeSkillInstallSource::parse("./local", Some(&name)).unwrap();
        assert_eq!(source.filter(), Some(name.as_str()));
    }
    for name in ["x".repeat(257), format!("{}x", "é".repeat(128))] {
        assert_eq!(
            NativeSkillInstallSource::parse("./local", Some(&name)),
            Err(Error::InvalidName)
        );
    }
}

#[test]
fn explicit_malformed_filters_keep_invalid_name_errors() {
    for name in [".", "..", "a/b", "a\\b", "a\0b", "a\nb", "a\tb", "a\u{85}b"] {
        assert_eq!(
            NativeSkillInstallSource::parse("owner/repo", Some(name)),
            Err(Error::InvalidName)
        );
    }
}

#[test]
fn explicit_filter_validation_preserves_existing_error_precedence() {
    for (input, explicit, error) in [
        ("bad\0source", "..", Error::InvalidSource),
        ("npx skills add", "..", Error::InvalidSource),
        ("https://skills.sh/a/b/c/d", "..", Error::InvalidSource),
        (
            "npx skills add owner/repo@two --skill one",
            "..",
            Error::ConflictingFilter,
        ),
        ("owner/repo@..", "different", Error::InvalidName),
        ("owner/repo@one", "..", Error::InvalidName),
        ("owner/repo@one", "two", Error::ConflictingFilter),
        ("file:///tmp/repo", "..", Error::InvalidName),
        ("file:///tmp/repo", "valid", Error::InvalidSource),
    ] {
        assert_eq!(
            NativeSkillInstallSource::parse(input, Some(explicit)),
            Err(error),
            "{input}"
        );
    }
}

#[test]
fn huge_borrowed_explicit_filter_is_rejected_without_copying_it() {
    let small = "x".repeat(257);
    let huge = "x".repeat(8 * 1024 * 1024);
    allocation_counter::measure(|| {});
    let measure = |name: &str| {
        allocation_counter::measure(|| {
            assert_eq!(
                NativeSkillInstallSource::parse("./local", Some(name)),
                Err(Error::InvalidName)
            );
        })
    };
    let small_allocations = measure(&small);
    let huge_allocations = measure(&huge);
    assert_eq!(huge_allocations.bytes_total, small_allocations.bytes_total);
    assert!(huge_allocations.bytes_total <= 4096, "{huge_allocations:?}");
}
