use super::*;
use crate::NativeFileApprovalKind as Kind;
use crate::file_approval::tests::{Fixture, Policy, arguments};
use std::fs;

struct Tamper(std::path::PathBuf);
impl DeleteFileEvidence for Tamper {
    fn checkpoint(&mut self, checkpoint: DeleteCheckpoint, _: &CancellationToken) {
        if checkpoint == DeleteCheckpoint::FinalPreUnlink {
            fs::write(&self.0, b"tamper").unwrap();
        }
    }
}

#[test]
fn file_approval_rejects_content_change_at_final_native_unlink_checkpoint() {
    let fixture = Fixture::new();
    fixture.seed();
    let args = arguments(Kind::Delete);
    fixture.admit(Kind::Delete, &args, Policy::new(usize::MAX));
    let tool = DeleteFileTool::open(&fixture.path)
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
        &cancellation,
        &mut Tamper(fixture.path.join("a")),
    );
    assert_eq!(result.unwrap_err().code, "file_approval_failed");
    assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"tamper");
}
