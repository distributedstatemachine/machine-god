use super::*;
use futures_executor::block_on;
use machine_god_core::{PermissionRisk, Tool, ToolCall, ToolCallId, ToolOutput};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};

pub(crate) struct Fixture {
    pub(crate) path: PathBuf,
    pub(crate) registry: Arc<NativeFileApprovalRegistry>,
    pub(crate) authority: NativeFileApprovalAuthority,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = loop {
            let path = std::env::temp_dir().join(format!(
                "mg-file-approval-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("{error}"),
            }
        };
        let authority =
            NativeFileApprovalAuthority::from_directory(File::open(&path).unwrap()).unwrap();
        Self {
            path,
            registry: Arc::new(NativeFileApprovalRegistry::new()),
            authority,
        }
    }
    pub(crate) fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("reused-call").unwrap(),
        }
    }
    pub(crate) fn tool(&self, kind: NativeFileApprovalKind) -> Box<dyn Tool> {
        match kind {
            NativeFileApprovalKind::Write => Box::new(
                crate::WriteFileTool::open(&self.path)
                    .unwrap()
                    .with_file_approvals(self.registry.clone()),
            ),
            NativeFileApprovalKind::Edit => Box::new(
                crate::EditFileTool::open(&self.path)
                    .unwrap()
                    .with_file_approvals(self.registry.clone()),
            ),
            NativeFileApprovalKind::Delete => Box::new(
                crate::DeleteFileTool::open(&self.path)
                    .unwrap()
                    .with_file_approvals(self.registry.clone()),
            ),
            NativeFileApprovalKind::Rename => Box::new(
                crate::RenameFileTool::open(&self.path)
                    .unwrap()
                    .with_file_approvals(self.registry.clone()),
            ),
            NativeFileApprovalKind::Copy => Box::new(
                crate::CopyFileTool::open(&self.path)
                    .unwrap()
                    .with_file_approvals(self.registry.clone()),
            ),
        }
    }
    pub(crate) fn prepare(
        &self,
        kind: NativeFileApprovalKind,
        arguments: &Value,
        request_id: &str,
    ) -> Result<PreparedFileApproval, Error> {
        let tool = self.tool(kind);
        let name = tool.spec().name;
        let context = Self::context();
        let prepared = tool
            .prepare(ToolCall {
                id: context.call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            })
            .unwrap();
        let request = PermissionRequest {
            id: PermissionRequestId::new(request_id).unwrap(),
            session_id: context.session_id,
            session_incarnation_id: context.session_incarnation_id,
            turn_id: context.turn_id,
            capability: prepared.capability().unwrap().clone(),
            risk: PermissionRisk::High,
            reason: "private-reason".into(),
        };
        block_on(self.registry.prepare(
            &self.authority,
            &request,
            PermissionInvocation {
                tool_name: &name,
                call_id: &context.call_id,
                arguments,
            },
            CancellationToken::new(),
        ))
    }
    pub(crate) fn admit(
        &self,
        kind: NativeFileApprovalKind,
        arguments: &Value,
        policy: Arc<dyn NativeFileApprovalPolicy>,
    ) {
        Box::new(
            self.prepare(kind, arguments, "request")
                .unwrap()
                .admit(policy),
        )
        .admit()
        .unwrap();
    }
    fn execute(&self, kind: NativeFileApprovalKind, args: &Value) -> Result<ToolOutput, ToolError> {
        block_on(
            self.tool(kind)
                .execute(Self::context(), args.clone(), CancellationToken::new()),
        )
    }
    pub(crate) fn seed(&self) {
        fs::write(self.path.join("a"), b"before").unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) struct Policy {
    pub(crate) valid: AtomicBool,
    calls: AtomicUsize,
    allowed_checks: usize,
}
impl Policy {
    pub(crate) fn new(allowed_checks: usize) -> Arc<Self> {
        Arc::new(Self {
            valid: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            allowed_checks,
        })
    }
}
impl NativeFileApprovalPolicy for Policy {
    fn revalidate(&self) -> Result<(), PermissionError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.valid.load(Ordering::Acquire) && call < self.allowed_checks {
            Ok(())
        } else {
            Err(PermissionError::new(
                "secret-policy",
                "secret-policy-message",
            ))
        }
    }
}
const KINDS: [NativeFileApprovalKind; 5] = [
    NativeFileApprovalKind::Write,
    NativeFileApprovalKind::Edit,
    NativeFileApprovalKind::Delete,
    NativeFileApprovalKind::Rename,
    NativeFileApprovalKind::Copy,
];
pub(crate) fn arguments(kind: NativeFileApprovalKind) -> Value {
    match kind {
        NativeFileApprovalKind::Write => json!({"path":"a","content":"after"}),
        NativeFileApprovalKind::Edit => {
            json!({"path":"a","old_string":"before","new_string":"after"})
        }
        NativeFileApprovalKind::Delete => json!({"path":"a"}),
        NativeFileApprovalKind::Rename => json!({"old_path":"a","new_path":"b"}),
        NativeFileApprovalKind::Copy => json!({"source":"a","destination":"b"}),
    }
}

#[test]
fn all_five_tools_require_exact_one_shot_claim_and_complete_real_mutation() {
    for kind in KINDS {
        let f = Fixture::new();
        f.seed();
        let args = arguments(kind);
        assert_eq!(
            f.execute(kind, &args).unwrap_err().code,
            "file_approval_failed"
        );
        f.admit(kind, &args, Policy::new(usize::MAX));
        f.execute(kind, &args).unwrap();
        assert!(f.execute(kind, &args).is_err());
        match kind {
            NativeFileApprovalKind::Write | NativeFileApprovalKind::Edit => {
                assert_eq!(fs::read(f.path.join("a")).unwrap(), b"after");
            }
            NativeFileApprovalKind::Delete => assert!(!f.path.join("a").exists()),
            NativeFileApprovalKind::Rename => {
                assert!(!f.path.join("a").exists());
                assert_eq!(fs::read(f.path.join("b")).unwrap(), b"before");
            }
            NativeFileApprovalKind::Copy => {
                assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
                assert_eq!(fs::read(f.path.join("b")).unwrap(), b"before");
            }
        }
        assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn all_five_tools_reject_revocation_at_final_effect_not_just_initial_admission() {
    for kind in KINDS {
        let f = Fixture::new();
        f.seed();
        let args = arguments(kind);
        let policy = Policy::new(2);
        f.admit(kind, &args, policy.clone());
        let error = f.execute(kind, &args).unwrap_err();
        assert_eq!(error.code, "file_approval_failed");
        assert_eq!(policy.calls.load(Ordering::SeqCst), 3);
        assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
        assert!(!f.path.join("b").exists());
        assert_eq!(fs::read_dir(&f.path).unwrap().count(), 1);
        assert!(!format!("{error:?}").contains("secret"));
    }
}

#[test]
fn all_five_tools_reject_replaced_inode_even_with_identical_preimage_bytes() {
    for kind in KINDS {
        let f = Fixture::new();
        f.seed();
        let args = arguments(kind);
        f.admit(kind, &args, Policy::new(usize::MAX));
        fs::rename(f.path.join("a"), f.path.join("retained-old")).unwrap();
        fs::write(f.path.join("a"), b"before").unwrap();
        assert!(f.execute(kind, &args).is_err());
        assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
        assert!(!f.path.join("b").exists());
    }
}

#[test]
fn all_five_tools_reject_same_inode_content_change() {
    for kind in KINDS {
        let f = Fixture::new();
        f.seed();
        let args = arguments(kind);
        f.admit(kind, &args, Policy::new(usize::MAX));
        fs::write(f.path.join("a"), b"tamper").unwrap();
        assert!(f.execute(kind, &args).is_err());
        assert_eq!(fs::read(f.path.join("a")).unwrap(), b"tamper");
        assert!(!f.path.join("b").exists());
    }
}

#[test]
fn approval_identity_preserves_missing_empty_and_exact_edit_postimage() {
    let f = Fixture::new();
    let args = arguments(NativeFileApprovalKind::Write);
    let absent = f
        .prepare(NativeFileApprovalKind::Write, &args, "absent")
        .unwrap();
    assert!(matches!(
        absent.preimage(),
        NativeFileApprovalPreimage::Missing
    ));
    let missing_key = absent.saved_rule_key().unwrap();
    drop(absent);
    fs::write(f.path.join("a"), b"").unwrap();
    let empty = f
        .prepare(NativeFileApprovalKind::Write, &args, "empty")
        .unwrap();
    assert!(matches!(
        empty.preimage(),
        NativeFileApprovalPreimage::File(b"")
    ));
    assert_ne!(missing_key, empty.saved_rule_key().unwrap());
    drop(empty);
    fs::write(f.path.join("a"), "α before ω").unwrap();
    let edited = f
        .prepare(
            NativeFileApprovalKind::Edit,
            &arguments(NativeFileApprovalKind::Edit),
            "edit",
        )
        .unwrap();
    assert_eq!(edited.postimage(), Some("α after ω".as_bytes()));
    drop(edited);
    fs::write(f.path.join("a"), b"before before").unwrap();
    assert!(
        f.prepare(
            NativeFileApprovalKind::Edit,
            &arguments(NativeFileApprovalKind::Edit),
            "ambiguous"
        )
        .is_err()
    );
    fs::write(f.path.join("a"), b"nomatch").unwrap();
    assert!(
        f.prepare(
            NativeFileApprovalKind::Edit,
            &arguments(NativeFileApprovalKind::Edit),
            "nomatch"
        )
        .is_err()
    );
}

#[test]
fn saved_content_rule_survives_inode_replacement_but_runtime_approval_does_not() {
    let f = Fixture::new();
    f.seed();
    let args = arguments(NativeFileApprovalKind::Write);
    let first = f
        .prepare(NativeFileApprovalKind::Write, &args, "first")
        .unwrap();
    let key = first.saved_rule_key().unwrap();
    Box::new(first.admit(Policy::new(usize::MAX)))
        .admit()
        .unwrap();
    fs::rename(f.path.join("a"), f.path.join("old")).unwrap();
    fs::write(f.path.join("a"), b"before").unwrap();
    assert!(f.execute(NativeFileApprovalKind::Write, &args).is_err());
    let second = f
        .prepare(NativeFileApprovalKind::Write, &args, "second")
        .unwrap();
    assert_eq!(key, second.saved_rule_key().unwrap());
}

#[test]
fn repeated_call_ids_are_allowed_sequentially_not_ambiguous_and_late_drop_is_exact() {
    let f = Fixture::new();
    f.seed();
    let args = arguments(NativeFileApprovalKind::Write);
    let old = f
        .prepare(NativeFileApprovalKind::Write, &args, "old")
        .unwrap();
    assert!(matches!(
        f.prepare(NativeFileApprovalKind::Write, &args, "overlap"),
        Err(Error::Busy)
    ));
    let c = Fixture::context();
    f.registry
        .close_turn(&c.session_id, &c.session_incarnation_id, &c.turn_id);
    let newer = f
        .prepare(NativeFileApprovalKind::Write, &args, "new")
        .unwrap();
    drop(old); // Must not erase the new generation occupying the same route.
    Box::new(newer.admit(Policy::new(usize::MAX)))
        .admit()
        .unwrap();
    f.execute(NativeFileApprovalKind::Write, &args).unwrap();
    let next_args = json!({"path":"a","content":"again"});
    f.admit(
        NativeFileApprovalKind::Write,
        &next_args,
        Policy::new(usize::MAX),
    );
    f.execute(NativeFileApprovalKind::Write, &next_args)
        .unwrap();
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"again");
}

#[test]
fn owned_unpolled_prepare_and_execution_are_inert_and_close_discards_ready_proof() {
    let f = Fixture::new();
    f.seed();
    let args = arguments(NativeFileApprovalKind::Write);
    let context = Fixture::context();
    let tool = f.tool(NativeFileApprovalKind::Write);
    let name = tool.spec().name;
    let request = PermissionRequest {
        id: PermissionRequestId::new("unpolled").unwrap(),
        session_id: context.session_id.clone(),
        session_incarnation_id: context.session_incarnation_id.clone(),
        turn_id: context.turn_id.clone(),
        capability: Capability::Filesystem {
            access: FilesystemAccess::Write,
            path: "a".into(),
        },
        risk: PermissionRisk::High,
        reason: String::new(),
    };
    let pending = f.registry.prepare(
        &f.authority,
        &request,
        PermissionInvocation {
            tool_name: &name,
            call_id: &context.call_id,
            arguments: &args,
        },
        CancellationToken::new(),
    );
    assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    f.registry.close_turn(
        &context.session_id,
        &context.session_incarnation_id,
        &context.turn_id,
    );
    assert!(matches!(block_on(pending), Err(Error::Denied)));
    f.admit(
        NativeFileApprovalKind::Write,
        &args,
        Policy::new(usize::MAX),
    );
    drop(tool.execute(context.clone(), args.clone(), CancellationToken::new()));
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
    f.registry.close_turn(
        &context.session_id,
        &context.session_incarnation_id,
        &context.turn_id,
    );
    assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    assert!(f.execute(NativeFileApprovalKind::Write, &args).is_err());
}

#[test]
fn denial_and_cancel_release_reservation_without_effects() {
    let f = Fixture::new();
    f.seed();
    let args = arguments(NativeFileApprovalKind::Write);
    let prepared = f
        .prepare(NativeFileApprovalKind::Write, &args, "denied")
        .unwrap();
    assert!(Box::new(prepared.admit(Policy::new(0))).admit().is_err());
    assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    let prepared = f
        .prepare(NativeFileApprovalKind::Write, &args, "dropped")
        .unwrap();
    drop(prepared);
    assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    f.admit(
        NativeFileApprovalKind::Write,
        &args,
        Policy::new(usize::MAX),
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(
        block_on(f.tool(NativeFileApprovalKind::Write).execute(
            Fixture::context(),
            args,
            cancellation
        ))
        .is_err()
    );
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
}

#[test]
fn wrong_arguments_context_tool_or_workspace_never_claims_authority() {
    let f = Fixture::new();
    f.seed();
    let args = arguments(NativeFileApprovalKind::Write);
    for change in 0..5 {
        f.admit(
            NativeFileApprovalKind::Write,
            &args,
            Policy::new(usize::MAX),
        );
        let mut context = Fixture::context();
        match change {
            0 => context.session_id = SessionId::new("other").unwrap(),
            1 => context.session_incarnation_id = SessionIncarnationId::new("other").unwrap(),
            2 => context.turn_id = TurnId::new("other").unwrap(),
            3 => context.call_id = ToolCallId::new("other").unwrap(),
            _ => {}
        }
        let changed = if change == 4 {
            json!({"path":"a","content":"unapproved"})
        } else {
            args.clone()
        };
        assert!(
            block_on(f.tool(NativeFileApprovalKind::Write).execute(
                context,
                changed,
                CancellationToken::new()
            ))
            .is_err()
        );
        let c = Fixture::context();
        f.registry
            .close_turn(&c.session_id, &c.session_incarnation_id, &c.turn_id);
    }
    f.admit(
        NativeFileApprovalKind::Write,
        &args,
        Policy::new(usize::MAX),
    );
    assert!(
        f.execute(
            NativeFileApprovalKind::Delete,
            &arguments(NativeFileApprovalKind::Delete)
        )
        .is_err()
    );
    let other = Fixture::new();
    other.seed();
    let tool = crate::WriteFileTool::open(&other.path)
        .unwrap()
        .with_file_approvals(f.registry.clone());
    assert!(block_on(tool.execute(Fixture::context(), args, CancellationToken::new())).is_err());
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
    assert_eq!(fs::read(other.path.join("a")).unwrap(), b"before");
}

#[test]
fn parent_replacement_and_destination_appearance_fail_closed() {
    for kind in KINDS {
        let f = Fixture::new();
        fs::create_dir(f.path.join("sub")).unwrap();
        fs::write(f.path.join("sub/a"), b"before").unwrap();
        let mut args = arguments(kind);
        for value in args.as_object_mut().unwrap().values_mut() {
            if value == "a" {
                *value = json!("sub/a");
            }
        }
        f.admit(kind, &args, Policy::new(usize::MAX));
        fs::rename(f.path.join("sub"), f.path.join("old-parent")).unwrap();
        fs::create_dir(f.path.join("sub")).unwrap();
        fs::write(f.path.join("sub/a"), b"before").unwrap();
        assert!(f.execute(kind, &args).is_err());
        assert_eq!(fs::read(f.path.join("sub/a")).unwrap(), b"before");
    }
    for kind in [NativeFileApprovalKind::Copy, NativeFileApprovalKind::Rename] {
        let f = Fixture::new();
        f.seed();
        let args = arguments(kind);
        f.admit(kind, &args, Policy::new(usize::MAX));
        fs::write(f.path.join("b"), b"unapproved").unwrap();
        assert!(f.execute(kind, &args).is_err());
        assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
        assert_eq!(fs::read(f.path.join("b")).unwrap(), b"unapproved");
    }
}

#[test]
fn missing_parent_symlink_nonempty_directory_and_special_targets_fail_before_prompt() {
    let f = Fixture::new();
    f.seed();
    assert!(
        f.prepare(
            NativeFileApprovalKind::Write,
            &json!({"path":"missing/a","content":"after"}),
            "missing"
        )
        .is_err()
    );
    std::os::unix::fs::symlink("a", f.path.join("link")).unwrap();
    assert!(
        f.prepare(
            NativeFileApprovalKind::Write,
            &json!({"path":"link","content":"after"}),
            "link"
        )
        .is_err()
    );
    fs::create_dir(f.path.join("directory")).unwrap();
    let prepared = f
        .prepare(
            NativeFileApprovalKind::Delete,
            &json!({"path":"directory"}),
            "directory",
        )
        .unwrap();
    assert!(matches!(
        prepared.preimage(),
        NativeFileApprovalPreimage::EmptyDirectory
    ));
    drop(prepared);
    fs::write(f.path.join("directory/entry"), b"keep").unwrap();
    assert!(
        f.prepare(
            NativeFileApprovalKind::Delete,
            &json!({"path":"directory"}),
            "nonempty"
        )
        .is_err()
    );
    assert!(
        std::process::Command::new("mkfifo")
            .arg(f.path.join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        f.prepare(
            NativeFileApprovalKind::Delete,
            &json!({"path":"fifo"}),
            "fifo"
        )
        .is_err()
    );
}

#[test]
fn copy_exact_sixteen_mib_preimage_and_postimage_are_retained_once() {
    let f = Fixture::new();
    let bytes = vec![0xab; MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES];
    fs::write(f.path.join("a"), &bytes).unwrap();
    let args = arguments(NativeFileApprovalKind::Copy);
    let prepared = f
        .prepare(NativeFileApprovalKind::Copy, &args, "limit")
        .unwrap();
    assert_eq!(prepared.source_preimage().unwrap().len(), bytes.len());
    assert!(std::ptr::eq(
        prepared.source_preimage().unwrap(),
        prepared.postimage().unwrap()
    ));
    Box::new(prepared.admit(Policy::new(usize::MAX)))
        .admit()
        .unwrap();
    f.execute(NativeFileApprovalKind::Copy, &args).unwrap();
    assert_eq!(fs::read(f.path.join("b")).unwrap(), bytes);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(f.path.join("a"))
        .unwrap();
    file.set_len((MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES + 1) as u64)
        .unwrap();
    assert!(matches!(
        f.prepare(NativeFileApprovalKind::Copy, &args, "overflow"),
        Err(Error::Limit)
    ));
}

#[test]
fn capacity_generation_exhaustion_and_registry_drop_are_bounded() {
    let f = Fixture::new();
    let mut retained = Vec::new();
    for index in 0..MAX_NATIVE_FILE_APPROVALS {
        let path = format!("file-{index}");
        let context = Fixture::context();
        let name = ToolName::new("write_file").unwrap();
        let call_id = ToolCallId::new(format!("call-{index}")).unwrap();
        let args = json!({"path":path,"content":"after"});
        let request = PermissionRequest {
            id: PermissionRequestId::new(format!("request-{index}")).unwrap(),
            session_id: context.session_id,
            session_incarnation_id: context.session_incarnation_id,
            turn_id: context.turn_id,
            capability: Capability::Filesystem {
                access: FilesystemAccess::Write,
                path,
            },
            risk: PermissionRisk::High,
            reason: String::new(),
        };
        retained.push(
            block_on(f.registry.prepare(
                &f.authority,
                &request,
                PermissionInvocation {
                    tool_name: &name,
                    call_id: &call_id,
                    arguments: &args,
                },
                CancellationToken::new(),
            ))
            .unwrap(),
        );
    }
    assert!(matches!(
        f.prepare(
            NativeFileApprovalKind::Write,
            &arguments(NativeFileApprovalKind::Write),
            "full"
        ),
        Err(Error::Busy)
    ));
    drop(retained);
    assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    f.registry.state.lock().unwrap().next_generation = u64::MAX;
    assert!(matches!(
        f.prepare(
            NativeFileApprovalKind::Write,
            &arguments(NativeFileApprovalKind::Write),
            "exhausted"
        ),
        Err(Error::GenerationExhausted)
    ));
    assert_eq!(f.registry.retained.load(Ordering::SeqCst), 0);
    let registry = Arc::new(NativeFileApprovalRegistry::new());
    let weak = Arc::downgrade(&registry);
    drop(registry);
    assert!(weak.upgrade().is_none());
    let owned = Fixture::new();
    owned.seed();
    owned.admit(
        NativeFileApprovalKind::Write,
        &arguments(NativeFileApprovalKind::Write),
        Policy::new(usize::MAX),
    );
    let weak = Arc::downgrade(&owned.registry);
    drop(owned);
    assert!(
        weak.upgrade().is_none(),
        "ready proofs must not create a registry cycle"
    );
}

struct UnlockedPolicy {
    registry: Weak<NativeFileApprovalRegistry>,
    calls: Arc<AtomicUsize>,
}
impl NativeFileApprovalPolicy for UnlockedPolicy {
    fn revalidate(&self) -> Result<(), PermissionError> {
        let registry = self.registry.upgrade().unwrap();
        assert!(registry.state.try_lock().is_ok());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
impl Drop for UnlockedPolicy {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            assert!(registry.state.try_lock().is_ok());
        }
    }
}

#[test]
fn live_policy_calls_and_owned_policy_drop_never_hold_registry_mutex() {
    let f = Fixture::new();
    f.seed();
    let args = arguments(NativeFileApprovalKind::Write);
    let calls = Arc::new(AtomicUsize::new(0));
    f.admit(
        NativeFileApprovalKind::Write,
        &args,
        Arc::new(UnlockedPolicy {
            registry: Arc::downgrade(&f.registry),
            calls: calls.clone(),
        }),
    );
    f.execute(NativeFileApprovalKind::Write, &args).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    f.admit(
        NativeFileApprovalKind::Write,
        &args,
        Arc::new(UnlockedPolicy {
            registry: Arc::downgrade(&f.registry),
            calls,
        }),
    );
    let c = Fixture::context();
    f.registry
        .close_turn(&c.session_id, &c.session_incarnation_id, &c.turn_id);
}

#[test]
fn overlong_saved_key_does_not_truncate_or_disable_one_shot_approval() {
    use rustix::fs::{AtFlags, Mode};
    let f = Fixture::new();
    let mut current = rustix::io::fcntl_dupfd_cloexec(f.authority.root.as_fd(), 3).unwrap();
    let mut ancestors = Vec::new();
    let mut components = Vec::new();
    for index in 0..16 {
        let component = format!("{index:02}{}", "x".repeat(242));
        rustix::fs::mkdirat(&current, component.as_str(), Mode::from_raw_mode(0o700)).unwrap();
        let next = rustix::fs::openat(
            &current,
            component.as_str(),
            snapshot::directory_flags(),
            Mode::empty(),
        )
        .unwrap();
        components.push(component.clone());
        ancestors.push((current, component));
        current = next;
    }
    let path = format!("{}/a", components.join("/"));
    let args = json!({"path":path,"content":"after"});
    let prepared = f
        .prepare(NativeFileApprovalKind::Write, &args, "long")
        .unwrap();
    assert!(matches!(prepared.saved_rule_key(), Err(Error::Limit)));
    assert_eq!(prepared.postimage(), Some(b"after".as_slice()));
    Box::new(prepared.admit(Policy::new(usize::MAX)))
        .admit()
        .unwrap();
    f.execute(NativeFileApprovalKind::Write, &args).unwrap();
    assert!(rustix::fs::statat(&current, "a", AtFlags::SYMLINK_NOFOLLOW).is_ok());
    rustix::fs::unlinkat(&current, "a", AtFlags::empty()).unwrap();
    drop(current);
    for (parent, name) in ancestors.into_iter().rev() {
        rustix::fs::unlinkat(&parent, name.as_str(), AtFlags::REMOVEDIR).unwrap();
    }
}

#[test]
fn old_unpolled_executions_cannot_claim_new_requests_with_reused_call_ids() {
    for kind in KINDS {
        let f = Fixture::new();
        f.seed();
        let args = arguments(kind);
        f.admit(kind, &args, Policy::new(usize::MAX));
        let tool = f.tool(kind);
        let old = tool.execute(Fixture::context(), args.clone(), CancellationToken::new());
        let c = Fixture::context();
        f.registry
            .close_turn(&c.session_id, &c.session_incarnation_id, &c.turn_id);
        Box::new(
            f.prepare(kind, &args, "next-request")
                .unwrap()
                .admit(Policy::new(usize::MAX)),
        )
        .admit()
        .unwrap();
        assert!(block_on(old).is_err());
        assert_eq!(fs::read(f.path.join("a")).unwrap(), b"before");
        f.execute(kind, &args).unwrap();
    }
}

#[test]
fn execution_constructed_without_approval_cannot_acquire_a_later_grant() {
    let f = Fixture::new();
    f.seed();
    let kind = NativeFileApprovalKind::Write;
    let args = arguments(kind);
    let tool = f.tool(kind);
    let old = tool.execute(Fixture::context(), args.clone(), CancellationToken::new());
    f.admit(kind, &args, Policy::new(usize::MAX));
    assert!(block_on(old).is_err());
    f.execute(kind, &args).unwrap();
}
