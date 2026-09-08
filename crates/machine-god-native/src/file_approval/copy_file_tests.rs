use super::*;
use crate::NativeFileApprovalKind as Kind;
use crate::file_approval::tests::{Fixture, Policy, arguments};
use std::fs;

struct Tamper {
    target: std::path::PathBuf,
    stage: bool,
    descriptor: Option<OwnedFd>,
}
impl CopyFileEvidence for Tamper {
    fn open_stage(
        &mut self,
        parent: BorrowedFd<'_>,
        name: &str,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        let descriptor = rustix::fs::openat(
            parent,
            name,
            OFlags::RDWR
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )?;
        self.descriptor = Some(rustix::io::fcntl_dupfd_cloexec(&descriptor, 3)?);
        Ok(descriptor)
    }
    fn checkpoint(&mut self, checkpoint: CopyCheckpoint, _: &CancellationToken) {
        if checkpoint == CopyCheckpoint::FinalPrePublish {
            if self.stage {
                assert_eq!(
                    rustix::io::pwrite(self.descriptor.as_ref().unwrap(), b"tamper", 0).unwrap(),
                    6
                );
            } else {
                fs::write(&self.target, b"tamper").unwrap();
            }
        }
    }
}

#[test]
fn file_approval_rejects_copy_stage_and_source_changes_at_final_native_checkpoint() {
    for stage in [true, false] {
        let fixture = Fixture::new();
        fixture.seed();
        let args = arguments(Kind::Copy);
        fixture.admit(Kind::Copy, &args, Policy::new(usize::MAX));
        let tool = CopyFileTool::open(&fixture.path)
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
            &mut Tamper {
                target: fixture.path.join("a"),
                stage,
                descriptor: None,
            },
            &NativeCopyFileCleanupEvidence,
        );
        assert_eq!(result.unwrap_err().code, "file_approval_failed");
        assert_eq!(
            fs::read(fixture.path.join("a")).unwrap(),
            if stage { b"before" } else { b"tamper" }
        );
        assert!(!fixture.path.join("b").exists());
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);
    }
}
