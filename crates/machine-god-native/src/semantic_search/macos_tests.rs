use super::*;
use crate::list_files::enumeration_fixture::Fixture;
use crate::macos_directory::{MacosDirectoryEntry, MacosDirectoryReader};

#[test]
fn real_macos_reader_preserves_shared_admission_and_refill_ceiling() {
    let fixture = Fixture::new("semantic-macos-refills");
    for index in 0..300 {
        std::fs::write(
            fixture
                .primary
                .join(format!("{index:03}-{}.txt", "n".repeat(100))),
            "needle\n",
        )
        .unwrap();
    }
    let tool = SemanticSearchTool::open(&fixture.primary).unwrap();
    let cancellation = CancellationToken::new();
    let SearchRoot::Directory(root) = tool.open_search_root(".", &cancellation).unwrap() else {
        panic!("directory expected")
    };
    let mut budget = ScanBudget::default();
    let mut incomplete = IncompleteReasons::default();
    let entries =
        read_directory_entries(root.as_fd(), &mut budget, &mut incomplete, &cancellation).unwrap();
    assert_eq!(entries.len(), 301);
    assert_eq!(budget.visited_entries, 301);
    assert!(budget.directory_read_attempts > 2);
    assert!(!incomplete.any());
    assert!(
        entries
            .windows(2)
            .all(|pair| pair[0].sort_key < pair[1].sort_key)
    );

    let SearchRoot::Directory(root) = tool.open_search_root(".", &cancellation).unwrap() else {
        panic!("directory expected")
    };
    let mut reader = MacosDirectoryReader::new(root.as_fd());
    let mut budget = ScanBudget {
        directory_read_attempts: MAX_SEMANTIC_SEARCH_DIRECTORY_READ_ATTEMPTS - 1,
        ..ScanBudget::default()
    };
    let error =
        stage_directory_entry_names(&mut reader, &mut budget, &mut incomplete, &cancellation)
            .unwrap_err();
    assert_eq!(error.code, "semantic_search_scan_limit");
    assert_eq!(
        budget.directory_read_attempts,
        MAX_SEMANTIC_SEARCH_DIRECTORY_READ_ATTEMPTS
    );
    assert_eq!(budget.visited_entries, 0);
    assert!(!incomplete.any());
}

#[test]
fn deleted_record_adapter_is_exactly_one_dot_skip_without_name_or_visit_charge() {
    let skipped = macos_entry_name(MacosDirectoryEntry::Skipped);
    assert_eq!(skipped, b".");
    assert_eq!(
        macos_entry_name(MacosDirectoryEntry::Name(b"..".to_vec())),
        b".."
    );
    struct OneSkipped {
        calls: usize,
    }
    impl DirectoryEntryReader for OneSkipped {
        fn requires_read_attempt(&self) -> bool {
            self.calls == 1
        }
        fn next_name(&mut self) -> Option<Result<Vec<u8>, rustix::io::Errno>> {
            self.calls += 1;
            (self.calls == 1).then(|| Ok(macos_entry_name(MacosDirectoryEntry::Skipped)))
        }
    }
    let mut reader = OneSkipped { calls: 0 };
    let mut budget = ScanBudget::default();
    let mut incomplete = IncompleteReasons::default();
    let names = stage_directory_entry_names(
        &mut reader,
        &mut budget,
        &mut incomplete,
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(names.is_empty());
    assert_eq!(reader.calls, 2);
    assert_eq!(budget.directory_read_attempts, 1);
    assert_eq!(budget.visited_entries, 0);
    assert_eq!(budget.total_entry_name_bytes, 0);
    assert!(!incomplete.any());
}

#[test]
fn filesystem_root_linkage_check_is_supported_without_directory_enumeration() {
    let tool = SemanticSearchTool::open(Path::new("/")).unwrap();
    ensure_root_is_linked(tool.root.as_fd(), &CancellationToken::new()).unwrap();
}

#[test]
fn retained_root_observations_preserve_each_pre_and_post_call_checkpoint() {
    struct CancelAt {
        checks: std::cell::Cell<usize>,
        at: usize,
        cancellation: CancellationToken,
    }
    impl ScanCheck for CancelAt {
        fn check(&self) -> Result<(), ToolError> {
            self.checks.set(self.checks.get() + 1);
            if self.checks.get() == self.at {
                self.cancellation.cancel();
            }
            self.cancellation.check()
        }
    }
    let fixture = Fixture::new("semantic-macos-link-checkpoints");
    let tool = SemanticSearchTool::open(&fixture.primary).unwrap();
    // fstat, getpath, retained-parent open and no-follow stat each preserve
    // their original pre/post observation checks through the shared helper.
    for at in 1..=8 {
        let check = CancelAt {
            checks: std::cell::Cell::new(0),
            at,
            cancellation: CancellationToken::new(),
        };
        let error = ensure_root_is_linked(tool.root.as_fd(), &check).unwrap_err();
        assert_eq!(error.code, "semantic_search_cancelled");
        assert_eq!(check.checks.get(), at);
    }
}
