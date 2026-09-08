use super::*;
use crate::NativeFileApprovalKind as Kind;
use crate::file_approval::tests::{Fixture, Policy, arguments};
use std::fs;

struct Tamper {
    target: std::path::PathBuf,
    stage: bool,
}
impl EditFileEvidence for Tamper {
    fn after_final_stage_verification(
        &mut self,
        _parent: BorrowedFd<'_>,
        file: BorrowedFd<'_>,
        _name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        if self.stage {
            assert_eq!(rustix::io::pwrite(file, b"evil!", 0).unwrap(), 5);
        } else {
            fs::write(&self.target, b"tamper").unwrap();
        }
        Ok(())
    }
}

#[test]
fn file_approval_rejects_edit_stage_and_preimage_changes_after_last_baseline_checks() {
    for stage in [true, false] {
        let fixture = Fixture::new();
        fixture.seed();
        let args = arguments(Kind::Edit);
        fixture.admit(Kind::Edit, &args, Policy::new(usize::MAX));
        let tool = EditFileTool::open(&fixture.path)
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
            b"before",
            b"after",
            &cancellation,
            &mut Tamper {
                target: fixture.path.join("a"),
                stage,
            },
            native_set_mode,
            write_content,
            sync_before_commit,
            |_, _| {},
            || {},
            || {},
            native_publish_staged,
            native_sync_parent,
        );
        assert_eq!(result.unwrap_err().code, "file_approval_failed");
        assert_eq!(
            fs::read(fixture.path.join("a")).unwrap(),
            if stage { b"before" } else { b"tamper" }
        );
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);
    }
}
