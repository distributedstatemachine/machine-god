use super::*;
use crate::NativeFileApprovalKind as Kind;
use crate::file_approval::tests::{Fixture, Policy, arguments};
use std::cell::RefCell;
use std::fs;

#[test]
fn file_approval_rejects_stage_tamper_after_baseline_final_verification() {
    let fixture = Fixture::new();
    fixture.seed();
    let args = arguments(Kind::Write);
    fixture.admit(Kind::Write, &args, Policy::new(usize::MAX));
    let tool = WriteFileTool::open(&fixture.path)
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
    let staged = RefCell::new(String::new());
    let result = bound.execute_supported_with(
        "a",
        b"after",
        &cancellation,
        native_set_mode,
        write_content,
        sync_before_commit,
        |_, name| *staged.borrow_mut() = name.to_owned(),
        || fs::write(fixture.path.join(staged.borrow().as_str()), b"evil!").unwrap(),
        native_publish_staged,
        native_sync_parent,
    );
    assert_eq!(result.unwrap_err().code, "file_approval_failed");
    assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"before");
    assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);
}

#[test]
fn file_approval_rejects_preimage_change_after_initial_execution_claim() {
    let fixture = Fixture::new();
    fixture.seed();
    let args = arguments(Kind::Write);
    fixture.admit(Kind::Write, &args, Policy::new(usize::MAX));
    let tool = WriteFileTool::open(&fixture.path)
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
    let result = bound.execute_supported_with(
        "a",
        b"after",
        &cancellation,
        native_set_mode,
        write_content,
        sync_before_commit,
        |_, _| {},
        || fs::write(fixture.path.join("a"), b"tamper").unwrap(),
        native_publish_staged,
        native_sync_parent,
    );
    assert_eq!(result.unwrap_err().code, "file_approval_failed");
    assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"tamper");
}
