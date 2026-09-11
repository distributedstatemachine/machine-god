use crate::skills_catalog::{
    NativeSkillCatalog, NativeSkillCatalogError, NativeSkillLinkPolicy, NativeSkillRoot,
    NativeSkillSource,
};
use crate::skills_metadata::MAX_NATIVE_SKILL_HEADER_BYTES;
use machine_god_core::CancellationToken;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new(text: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mg-skills-prefix-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("review")).unwrap();
        fs::write(root.join("review/SKILL.md"), text).unwrap();
        Self(root)
    }
    fn catalog(&self) -> NativeSkillCatalog {
        NativeSkillCatalog::new(vec![
            NativeSkillRoot::from_directory(
                Arc::new(fs::File::open(&self.0).unwrap()),
                PathBuf::new(),
                self.0.clone(),
                self.0.clone(),
                NativeSkillSource::WorkspaceShared,
                NativeSkillLinkPolicy::Reject,
            )
            .unwrap(),
        ])
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn round_trip(text: &str, body_offset: usize) {
    let fixture = Fixture::new(text);
    let catalog = fixture.catalog();
    let token = CancellationToken::new();
    let snapshot = catalog.discover(&token).unwrap();
    assert!(snapshot.complete(), "{:?}", snapshot.diagnostics());
    let selection = snapshot.resolve("review", None).unwrap();
    let materialized = catalog.materialize(&selection, &token).unwrap();
    assert_eq!(materialized.text, text);
    assert_eq!(materialized.metadata.body_offset, body_offset);
}

#[test]
fn unchanged_skill_with_closing_dashes_at_read_boundary_materializes() {
    let text = format!("---\nname: review\n#{}\n---\n\nbody\n", "x".repeat(16_362));
    round_trip(&text, 16_385);
}

fn closing_at(end: usize) -> String {
    let start = "---\nname: review\n#";
    let text = format!("{start}{}\n---", "x".repeat(end - start.len() - 4));
    assert_eq!(text.len(), end);
    text
}

#[test]
fn split_or_continued_closing_lines_round_trip_at_each_nonfinal_chunk_boundary() {
    for boundary in [16_384, 32_768, 49_152] {
        for ending in ["\n", "\r\n", "x\n---\n", "\rX\n---\n"] {
            let text = format!("{}{ending}\nbody\n", closing_at(boundary));
            round_trip(&text, boundary + ending.len());
        }
        // The first read ends at CR; the next read must include its LF.
        let text = format!("{}\r\n\nbody\n", closing_at(boundary - 1));
        round_trip(&text, boundary + 1);
    }
}

#[test]
fn complete_newline_and_actual_eof_delimiters_preserve_exact_chunk_and_header_limits() {
    for boundary in [16_384, 32_768, 49_152, MAX_NATIVE_SKILL_HEADER_BYTES] {
        round_trip(&closing_at(boundary), boundary);
        for newline in ["\n", "\r\n"] {
            let text = format!("{}{newline}\nbody\n", closing_at(boundary - newline.len()));
            round_trip(&text, boundary);
        }
    }
}

#[test]
fn delimiter_newlines_beyond_header_limit_are_rejected_without_advertising_selection() {
    for (dashes_end, ending) in [
        (MAX_NATIVE_SKILL_HEADER_BYTES, "\n"),
        (MAX_NATIVE_SKILL_HEADER_BYTES, "\r\n"),
        (MAX_NATIVE_SKILL_HEADER_BYTES - 1, "\r\n"),
        (MAX_NATIVE_SKILL_HEADER_BYTES, "x\n---\n"),
        (MAX_NATIVE_SKILL_HEADER_BYTES, "\rX\n---\n"),
    ] {
        let fixture = Fixture::new(&format!("{}{ending}\nbody\n", closing_at(dashes_end)));
        let snapshot = fixture
            .catalog()
            .discover(&CancellationToken::new())
            .unwrap();
        assert!(!snapshot.complete());
        assert!(snapshot.entries().is_empty());
        assert_eq!(snapshot.diagnostics().len(), 1);
        assert_eq!(
            snapshot.diagnostics()[0].cause,
            NativeSkillCatalogError::ResourceLimit
        );
    }
}
