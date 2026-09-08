use super::*;
use crate::NativeFileApprovalKind as Kind;
use crate::file_approval::tests::{Fixture, Policy, arguments};
use std::fs;

struct Tamper(std::path::PathBuf);
impl RenameFileEvidence for Tamper {
    fn checkpoint(&mut self, checkpoint: RenameCheckpoint, _: &CancellationToken) {
        if checkpoint == RenameCheckpoint::FinalPreRename {
            fs::write(&self.0, b"tamper").unwrap();
        }
    }
}

#[test]
fn file_approval_rejects_source_change_at_final_native_rename_checkpoint() {
    let fixture = Fixture::new();
    fixture.seed();
    let args = arguments(Kind::Rename);
    fixture.admit(Kind::Rename, &args, Policy::new(usize::MAX));
    let tool = RenameFileTool::open(&fixture.path)
        .unwrap()
        .with_file_approvals(fixture.registry.clone());
    let cancellation = CancellationToken::new();
    let bound = tool
        .approval_bound(
            &Fixture::context(),
            &args,
            &cancellation,
            tool.approval_ticket(&Fixture::context()),
        )
        .unwrap()
        .unwrap();
    let result = bound.execute_supported_with_evidence(
        "a",
        "b",
        &cancellation,
        &mut Tamper(fixture.path.join("a")),
    );
    assert_eq!(result.unwrap_err().code, "file_approval_failed");
    assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"tamper");
    assert!(!fixture.path.join("b").exists());
}
