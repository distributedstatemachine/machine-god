use super::InputSettlement;
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeInteractiveInput, NativeInteractiveInputHelper, NativeInteractiveInputSource,
    NativeInteractiveTerminal,
};
use rustix::fs::{Mode, OFlags};
use std::{
    fs::File,
    future::poll_fn,
    io::Write as _,
    path::{Path, PathBuf},
    task::{Context, Waker},
    time::Duration,
};

fn runtime() -> machine_god_native::TokioWebSearchRuntime {
    machine_god_native::TokioWebSearchDeadline::build_runtime_pair()
        .unwrap()
        .0
}

fn helper() -> NativeInteractiveInputHelper {
    let selected = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY");
    #[cfg(target_os = "macos")]
    assert!(
        selected.is_some(),
        "provide the freshly built production CLI helper"
    );
    let path = selected.map_or_else(
        || {
            std::env::current_exe()
                .unwrap()
                .parent()
                .and_then(Path::parent)
                .unwrap()
                .join("machine-god")
        },
        PathBuf::from,
    );
    let path = path.canonicalize().unwrap();
    NativeInteractiveInputHelper::new(&path, File::open(&path).unwrap()).unwrap()
}

fn pty() -> (File, File) {
    let master = rustix::fs::open(
        "/dev/ptmx",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .unwrap();
    rustix::pty::grantpt(&master).unwrap();
    rustix::pty::unlockpt(&master).unwrap();
    #[cfg(target_os = "linux")]
    let slave = rustix::pty::ioctl_tiocgptpeer(
        &master,
        rustix::pty::OpenptFlags::RDWR
            | rustix::pty::OpenptFlags::NOCTTY
            | rustix::pty::OpenptFlags::CLOEXEC,
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let slave = {
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap()
    };
    (master.into(), slave.into())
}

fn settings(file: &File) -> String {
    // rustix does not expose equality for all termios fields. Its Debug includes
    // the special-code array as well as modes and input/output speeds.
    format!("{:?}", rustix::termios::tcgetattr(file).unwrap())
}

fn input(file: File) -> NativeInteractiveInput {
    NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveShared {
            input: file,
            helper: helper(),
        },
        CancellationToken::new(),
    )
}

async fn prove_live_raw_reader(master: &mut File, input: &mut NativeInteractiveInput) {
    // Ctrl-D, CR and NUL arrive as bytes, without a canonical line delimiter.
    let expected = [4, b'\r', 0, b'x'];
    master.write_all(&expected).unwrap();
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while received.len() < expected.len() {
            let chunk = poll_fn(|cx| input.poll_chunk(cx)).await.unwrap().unwrap();
            received.extend_from_slice(chunk.as_bytes());
        }
    })
    .await
    .expect("real helper raw-byte roundtrip");
    assert_eq!(received, expected);
    assert!(
        input
            .poll_chunk(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

fn assert_raw(alias: &File, original_flags: OFlags) {
    let local = rustix::termios::tcgetattr(alias).unwrap().local_modes;
    assert!(!local.intersects(
        rustix::termios::LocalModes::ICANON
            | rustix::termios::LocalModes::ECHO
            | rustix::termios::LocalModes::ISIG
    ));
    assert_eq!(rustix::fs::fcntl_getfl(alias).unwrap(), original_flags);
}

#[test]
fn settlement_joins_the_real_pending_reader_before_restoring_termios_and_flags() {
    for explicit_stop in [false, true] {
        let executor = runtime();
        let (mut master, slave) = pty();
        let alias = slave.try_clone().unwrap();
        if explicit_stop {
            let flags = rustix::fs::fcntl_getfl(&alias).unwrap();
            rustix::fs::fcntl_setfl(&alias, flags | OFlags::NONBLOCK).unwrap();
        }
        let original = settings(&alias);
        let original_flags = rustix::fs::fcntl_getfl(&alias).unwrap();
        let mut terminal = NativeInteractiveTerminal::new(slave.try_clone().unwrap());
        let terminal_completion = terminal.completion();
        let mut input = input(slave);
        let input_completion = input.completion();
        let completed_input = input.completion();
        executor.block_on(async {
            terminal.activate().await.unwrap();
            prove_live_raw_reader(&mut master, &mut input).await;
        });
        assert_raw(&alias, original_flags);
        assert!(!input_completion.is_complete());
        assert!(!terminal_completion.is_complete());
        if explicit_stop {
            input.request_stop();
        }
        drop(input);
        assert_raw(&alias, original_flags);
        InputSettlement {
            input_completion,
            terminal,
            runtime: &executor,
            size_completion: None,
        }
        .finish()
        .unwrap();
        assert!(completed_input.is_complete());
        assert!(terminal_completion.is_complete());
        completed_input.wait_on_worker().unwrap();
        terminal_completion.wait_on_worker().unwrap();
        assert_eq!(settings(&alias), original);
        assert_eq!(rustix::fs::fcntl_getfl(&alias).unwrap(), original_flags);
    }
}

#[test]
fn operation_unwind_drops_the_reader_but_retains_terminal_restoration_until_settlement() {
    let executor = runtime();
    let (mut master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let original_flags = rustix::fs::fcntl_getfl(&alias).unwrap();
    let terminal = NativeInteractiveTerminal::new(slave.try_clone().unwrap());
    let terminal_completion = terminal.completion();
    let input = input(slave);
    let input_completion = input.completion();
    let completed_input = input.completion();
    let mut settlement = InputSettlement {
        input_completion,
        terminal,
        runtime: &executor,
        size_completion: None,
    };
    let result: Result<(), _> = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut input = input;
        executor.block_on(async {
            settlement.terminal.activate().await.unwrap();
            prove_live_raw_reader(&mut master, &mut input).await;
        });
        panic!("simulated interactive operation failure");
    }));
    let failure = result.expect_err("operation must unwind after the real reader roundtrip");
    assert_eq!(
        failure.downcast_ref::<&str>(),
        Some(&"simulated interactive operation failure")
    );
    assert_raw(&alias, original_flags);
    assert!(!terminal_completion.is_complete());
    settlement.finish().unwrap();
    assert!(completed_input.is_complete());
    assert!(terminal_completion.is_complete());
    assert_eq!(settings(&alias), original);
    assert_eq!(rustix::fs::fcntl_getfl(&alias).unwrap(), original_flags);
}

#[test]
fn never_activated_terminal_and_unpolled_input_still_close_both_completion_scopes() {
    let executor = runtime();
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let original_flags = rustix::fs::fcntl_getfl(&alias).unwrap();
    let terminal = NativeInteractiveTerminal::new(slave.try_clone().unwrap());
    let terminal_completion = terminal.completion();
    let input = input(slave);
    let input_completion = input.completion();
    let completed_input = input.completion();
    assert_eq!(settings(&alias), original);
    assert!(!terminal_completion.is_complete());
    drop(input);
    InputSettlement {
        input_completion,
        terminal,
        runtime: &executor,
        size_completion: None,
    }
    .finish()
    .unwrap();
    assert!(completed_input.is_complete());
    assert!(terminal_completion.is_complete());
    assert_eq!(settings(&alias), original);
    assert_eq!(rustix::fs::fcntl_getfl(&alias).unwrap(), original_flags);
}
