use super::*;
use machine_god_native::{TerminalTapeRecordingFrame, TokioWebSearchDeadline};
use std::{
    fs,
    os::unix::ffi::OsStringExt,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    store: Arc<FileSessionStore>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mg-cli-record-start-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let state = root.join("state");
        fs::create_dir(&state).unwrap();
        let store = Arc::new(FileSessionStore::open(&state).unwrap());
        Self { root, state, store }
    }
    fn start(
        &self,
        owner: &Settlement,
        required: bool,
        path: Option<&str>,
        input: Option<&str>,
    ) -> Result<Started, ()> {
        let runtime = TokioWebSearchDeadline::build_runtime_pair().unwrap().0;
        owner.start(
            &runtime,
            Selection::from_values(
                required,
                &self.root,
                path.map(Into::into),
                input.map(Into::into),
            ),
            self.store.clone(),
            self.state.clone(),
            TerminalTapeRecordingOptions::new(20, 3, 100, b"test".to_vec()),
            &mut Signals(None),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

struct Signals(Option<super::super::AskSignal>);
impl super::super::SignalSource for Signals {
    fn poll_signal(&mut self, _: &mut Context<'_>) -> Poll<super::super::AskSignal> {
        self.0.take().map_or(Poll::Pending, Poll::Ready)
    }
}

#[test]
fn selection_is_pure_preserves_raw_paths_and_exact_input_opt_in() {
    assert!(
        Selection::from_values(
            false,
            Path::new("/workspace"),
            Some(" \t\r\n".into()),
            Some("true".into())
        )
        .is_none()
    );
    for value in ["1", " TrUe\r\n", "ON", "\ton "] {
        assert!(
            Selection::from_values(true, Path::new("/workspace"), None, Some(value.into()))
                .unwrap()
                .record_stdin
        );
    }
    for value in ["yes", "2", "true\u{b}", "\u{c}on", "", "false"] {
        assert!(
            !Selection::from_values(true, Path::new("/workspace"), None, Some(value.into()))
                .unwrap()
                .record_stdin
        );
    }
    let selected = Selection::from_values(
        false,
        Path::new("/workspace"),
        Some(OsString::from_vec(b" \tname-\xff.fxtape\r\n".to_vec())),
        None,
    )
    .unwrap();
    let Destination::Explicit(path) = selected.destination else {
        panic!("explicit path");
    };
    assert_eq!(
        path.into_os_string().into_vec(),
        b"/workspace/name-\xff.fxtape"
    );
    assert!(!selected.record_stdin);
    let selected =
        Selection::from_values(true, Path::new("/workspace"), Some("existing".into()), None)
            .unwrap();
    assert!(
        matches!(selected.destination, Destination::Explicit(_)),
        "explicit environment destination wins over automatic flag"
    );
}

#[test]
fn explicit_start_precedes_admission_and_finish_proves_independent_worker_join() {
    let fixture = Fixture::new();
    let owner = Settlement::default();
    let mut started = fixture
        .start(&owner, true, Some("terminal.fxtape"), Some("true"))
        .unwrap();
    assert!(owner.is_live());
    let mut recorder = started.recorder.take().unwrap();
    let completion = recorder.completion();
    assert!(completion.status().active);
    assert!(!completion.workers().is_complete());
    assert!(
        String::from_utf8(started.notice.unwrap())
            .unwrap()
            .contains("stdin included")
    );
    let runtime = TokioWebSearchDeadline::build_runtime_pair().unwrap().0;
    runtime
        .block_on(recorder.record(
            TerminalTapeRecordingFrame::StdoutWritten(b"accepted"),
            101,
            CancellationToken::new(),
        ))
        .unwrap();
    runtime.block_on(recorder.finish()).unwrap();
    drop(recorder);
    owner.finish().unwrap();
    assert!(!owner.is_live());
    assert!(completion.workers().is_complete());
    assert!(completion.status().complete);
    assert!(
        fs::read(fixture.root.join("terminal.fxtape"))
            .unwrap()
            .ends_with(b"accepted")
    );
}

#[test]
fn automatic_destination_uses_retained_store_despite_path_replacement() {
    let fixture = Fixture::new();
    let moved = fixture.root.join("moved-state");
    fs::rename(&fixture.state, &moved).unwrap();
    fs::create_dir(&fixture.state).unwrap();
    let owner = Settlement::default();
    let started = fixture.start(&owner, true, None, None).unwrap();
    let mut recorder = started.recorder.unwrap();
    let name = recorder.path().file_name().unwrap().to_owned();
    assert!(moved.join("recordings").join(name).exists());
    assert!(!fixture.state.join("recordings").exists());
    let runtime = TokioWebSearchDeadline::build_runtime_pair().unwrap().0;
    runtime.block_on(recorder.finish()).unwrap();
    drop(recorder);
    owner.finish().unwrap();
}

#[test]
fn optional_failure_is_joined_and_fixed_while_required_failure_is_fatal() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("existing.fxtape"), b"preserved").unwrap();
    for required in [false, true] {
        let owner = Settlement::default();
        let result = fixture.start(&owner, required, Some("existing.fxtape"), None);
        assert!(
            !owner.is_live(),
            "failed setup collector has joined before return"
        );
        if required {
            assert!(result.is_err());
        } else {
            let started = result.unwrap();
            assert!(started.recorder.is_none());
            assert_eq!(started.notice.as_deref(), Some(UNAVAILABLE));
        }
        assert_eq!(
            fs::read(fixture.root.join("existing.fxtape")).unwrap(),
            b"preserved"
        );
        owner.finish().unwrap();
    }
}

#[test]
fn relative_parent_destination_retains_native_walk_checks() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.root.join("nested")).unwrap();
    let owner = Settlement::default();
    let mut recorder = fixture
        .start(&owner, true, Some("nested/../relative.fxtape"), None)
        .unwrap()
        .recorder
        .unwrap();
    assert!(fixture.root.join("relative.fxtape").exists());
    let runtime = TokioWebSearchDeadline::build_runtime_pair().unwrap().0;
    runtime.block_on(recorder.finish()).unwrap();
    drop(recorder);
    owner.finish().unwrap();
    assert!(
        fixture
            .start(&owner, true, Some("missing/../absent.fxtape"), None)
            .is_err()
    );
    assert!(!fixture.root.join("absent.fxtape").exists());
}

#[test]
fn abandoned_tape_is_joined_but_never_reported_complete() {
    let fixture = Fixture::new();
    let owner = Settlement::default();
    let started = fixture
        .start(&owner, true, Some("abandoned.fxtape"), None)
        .unwrap();
    let completion = started.recorder.as_ref().unwrap().completion();
    drop(started);
    assert!(owner.finish().is_err());
    assert!(completion.workers().is_complete());
    assert!(!completion.status().complete);
}

#[test]
fn signal_before_recording_poll_blocks_open_even_for_optional_environment_request() {
    let fixture = Fixture::new();
    let runtime = TokioWebSearchDeadline::build_runtime_pair().unwrap().0;
    let owner = Settlement::default();
    let result = owner.start(
        &runtime,
        Selection::from_values(false, &fixture.root, Some("cancelled.fxtape".into()), None),
        fixture.store.clone(),
        fixture.state.clone(),
        TerminalTapeRecordingOptions::new(20, 3, 100, b"test".to_vec()),
        &mut Signals(Some(super::super::AskSignal::Interrupt)),
    );
    assert!(result.is_err());
    assert!(!owner.is_live());
    assert!(!fixture.root.join("cancelled.fxtape").exists());
}

#[test]
fn invalid_bounds_are_rejected_without_admission_or_output_path_leaks() {
    let fixture = Fixture::new();
    let owner = Settlement::default();
    for path in ["x".repeat(MAX_PATH_BYTES + 1), "bad\0path".into()] {
        let started = fixture.start(&owner, false, Some(&path), None).unwrap();
        assert!(started.recorder.is_none());
        assert_eq!(started.notice.as_deref(), Some(UNAVAILABLE));
        assert!(!owner.is_live());
    }
    let notice = active_notice(Path::new("/record/line\n\x1b.fxtape"), false).unwrap();
    assert!(!notice.strip_suffix(b"\n").unwrap().contains(&b'\n'));
    assert!(!notice.contains(&0x1b));
}
