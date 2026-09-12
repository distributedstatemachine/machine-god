use super::*;

struct Workers(NativeOwnedWorkerScope);
impl Drop for Workers {
    fn drop(&mut self) {
        self.0.close();
        self.0.completion().wait_on_worker().unwrap();
    }
}

#[test]
fn successful_direct_child_handoff_continues_without_recovery_or_new_permission() {
    let archive = Archive::new();
    let workers = Workers(NativeOwnedWorkerScope::new());
    // This is an explicitly selected successful launcher fixture, not an
    // installed browser. The production direct-child path still owns its reap.
    let path = std::fs::canonicalize("/usr/bin/true").unwrap();
    let executable =
        NativeBackgroundUrlExecutable::new(path.clone(), File::open(path).unwrap()).unwrap();
    let launcher = NativeMcpBrowserLauncher::new(executable, vec![], workers.0.clone()).unwrap();
    let (fixture, mut inbox) = configured_with_launcher(
        &archive,
        &[json!({})],
        |id| {
            envelope(
                id,
                if id == 1 {
                    URL
                } else {
                    r#""result":{"resultType":"complete","content":[]}"#
                },
            )
        },
        None,
        Some(launcher),
    );
    let (events, prompts) = run_answered(&fixture, &mut inbox, r#"{"action":"accept"}"#);
    assert!(!result(&events).is_error);
    assert_eq!(prompts, 1);
    assert_eq!(wires(&fixture).len(), 2);
    assert_eq!(
        wires(&fixture)[1]["params"]["inputResponses"]["url"],
        json!({"action":"accept"})
    );
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
}
