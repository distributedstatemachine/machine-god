use super::*;
use rustix::fs::{Mode, OFlags};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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
fn settings(file: &File) -> Termios {
    NativeIo.get(file).unwrap()
}
fn flags(file: &File) -> OFlags {
    rustix::fs::fcntl_getfl(file).unwrap()
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "terminal observation expired");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn ready<T>(mut future: BoxFuture<'_, T>) -> T {
    let mut output = None;
    until(|| {
        if let Poll::Ready(value) = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            output = Some(value);
        }
        output.is_some()
    });
    output.unwrap()
}
fn joined(guard: &NativeInteractiveTerminal) {
    until(|| guard.completion().is_complete());
    guard.completion().wait_on_worker().unwrap();
}

#[test]
fn construction_and_unpolled_activation_and_restore_do_not_touch_terminal() {
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let original_flags = flags(&alias);
    let mut guard = NativeInteractiveTerminal::new(slave);
    drop(guard.activate());
    drop(guard.restore());
    assert!(same_settings(&original, &settings(&alias)));
    assert_eq!(flags(&alias), original_flags);
    assert!(guard.source.is_some());
    assert!(!guard.shared.requested());
    assert!(!guard.completion().is_complete());
    assert_eq!(ready(guard.restore()), Ok(Receipt::NotActivated));
    joined(&guard);
    assert_eq!(ready(guard.activate()), Err(Error::Unavailable));
    assert_eq!(ready(guard.restore()), Ok(Receipt::NotActivated));
    assert!(same_settings(&original, &settings(&alias)));
}

#[test]
fn pinned_raw_configuration_changes_only_the_named_settings() {
    let (_master, slave) = pty();
    let mut original = settings(&slave);
    original.input_modes.insert(
        InputModes::BRKINT
            | InputModes::ICRNL
            | InputModes::INPCK
            | InputModes::ISTRIP
            | InputModes::IXON
            | InputModes::IXOFF
            | InputModes::IGNBRK
            | InputModes::PARMRK,
    );
    original
        .local_modes
        .insert(LocalModes::ECHO | LocalModes::ICANON | LocalModes::IEXTEN | LocalModes::ISIG);
    original.special_codes[SpecialCodeIndex::VMIN] = 7;
    original.special_codes[SpecialCodeIndex::VTIME] = 3;
    let raw = pinned_raw(&original);
    assert_eq!(
        raw.input_modes,
        original.input_modes
            & !(InputModes::BRKINT
                | InputModes::ICRNL
                | InputModes::INPCK
                | InputModes::ISTRIP
                | InputModes::IXON
                | InputModes::IXOFF)
    );
    assert_eq!(
        raw.local_modes,
        original.local_modes
            & !(LocalModes::ECHO | LocalModes::ICANON | LocalModes::IEXTEN | LocalModes::ISIG)
    );
    assert_eq!(raw.control_modes & ControlModes::CSIZE, ControlModes::CS8);
    assert_eq!(
        raw.control_modes & !ControlModes::CSIZE,
        original.control_modes & !ControlModes::CSIZE
    );
    assert_eq!(raw.output_modes, original.output_modes);
    assert_eq!(raw.special_codes[SpecialCodeIndex::VMIN], 1);
    assert_eq!(raw.special_codes[SpecialCodeIndex::VTIME], 0);
    assert_eq!(raw.input_speed(), original.input_speed());
    assert_eq!(raw.output_speed(), original.output_speed());
}

#[test]
fn real_tty_raw_ctrl_d_and_explicit_restore_keep_exact_descriptor_and_flags() {
    let (mut master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    rustix::fs::fcntl_setfl(&alias, flags(&alias) | OFlags::NONBLOCK).unwrap();
    let original = settings(&alias);
    let original_flags = flags(&alias);
    let mut guard = NativeInteractiveTerminal::new(slave);
    assert_eq!(ready(guard.activate()), Ok(()));
    assert_eq!(ready(guard.activate()), Ok(()));
    assert!(same_settings(&pinned_raw(&original), &settings(&alias)));
    assert_eq!(flags(&alias), original_flags);
    master.write_all(&[4]).unwrap();
    let mut byte = [0];
    until(|| match rustix::io::read(&alias, &mut byte[..]) {
        Ok(1) => true,
        Err(rustix::io::Errno::AGAIN) => false,
        other => panic!("raw byte read failed: {other:?}"),
    });
    assert_eq!(byte, [4]);
    assert_eq!(ready(guard.restore()), Ok(Receipt::Restored));
    joined(&guard);
    assert!(same_settings(&original, &settings(&alias)));
    assert_eq!(flags(&alias), original_flags);
    assert_eq!(ready(guard.restore()), Ok(Receipt::Restored));
    assert_eq!(ready(guard.activate()), Err(Error::Unavailable));
}

#[test]
fn dropping_active_guard_restores_without_response_consumer() {
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let original_flags = flags(&alias);
    let mut guard = NativeInteractiveTerminal::new(slave);
    ready(guard.activate()).unwrap();
    let completion = guard.completion();
    drop(guard);
    until(|| completion.is_complete());
    completion.wait_on_worker().unwrap();
    assert!(same_settings(&original, &settings(&alias)));
    assert_eq!(flags(&alias), original_flags);
}

#[derive(Default)]
struct FaultIo {
    sets: AtomicUsize,
    gets: AtomicUsize,
    failed_get_at: Option<usize>,
    block_set: usize,
    fail_activation: bool,
    fail_restore: bool,
    panic_activation: bool,
    blocked: AtomicBool,
    entered: AtomicBool,
}
impl TerminalIo for FaultIo {
    fn get(&self, file: &File) -> Result<Termios, Error> {
        if self.failed_get_at == Some(self.gets.fetch_add(1, Ordering::SeqCst)) {
            return Err(Error::Unavailable);
        }
        NativeIo.get(file)
    }
    fn set(&self, file: &File, action: OptionalActions, value: &Termios) -> Result<(), Error> {
        let index = self.sets.fetch_add(1, Ordering::SeqCst);
        if index == 0 {
            NativeIo.set(file, action, value)?;
            if self.block_set == index {
                self.wait_if_blocked();
            }
            assert!(!self.panic_activation, "injected activation failure");
            if self.fail_activation {
                return Err(Error::Unavailable);
            }
            Ok(())
        } else {
            if self.block_set == index {
                self.wait_if_blocked();
            }
            if self.fail_restore {
                Err(Error::Unavailable)
            } else {
                NativeIo.set(file, action, value)
            }
        }
    }
}
impl FaultIo {
    fn wait_if_blocked(&self) {
        self.entered.store(true, Ordering::Release);
        while self.blocked.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
struct ReleaseBlocked(Arc<FaultIo>);
impl Drop for ReleaseBlocked {
    fn drop(&mut self) {
        self.0.blocked.store(false, Ordering::Release);
    }
}

#[test]
fn dropped_started_activation_restores_even_after_unobserved_publication() {
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let io = Arc::new(FaultIo {
        blocked: AtomicBool::new(true),
        ..FaultIo::default()
    });
    let release = ReleaseBlocked(io.clone());
    let mut guard = NativeInteractiveTerminal::new(slave);
    guard.io = io.clone();
    let mut activation = guard.activate();
    assert!(
        activation
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    until(|| io.entered.load(Ordering::Acquire));
    drop(activation);
    assert!(!guard.completion().is_complete());
    drop(release);
    assert_eq!(ready(guard.restore()), Ok(Receipt::Restored));
    joined(&guard);
    assert!(same_settings(&original, &settings(&alias)));
}

#[test]
fn activation_failure_after_mutation_and_panic_both_restore_original_settings() {
    for panic_activation in [false, true] {
        let (_master, slave) = pty();
        let alias = slave.try_clone().unwrap();
        let original = settings(&alias);
        let mut guard = NativeInteractiveTerminal::new(slave);
        guard.io = Arc::new(FaultIo {
            fail_activation: !panic_activation,
            panic_activation,
            ..FaultIo::default()
        });
        assert_eq!(
            ready(guard.activate()),
            Err(if panic_activation {
                Error::Unavailable
            } else {
                Error::ActivationFailed
            })
        );
        joined(&guard);
        assert!(same_settings(&original, &settings(&alias)));
        assert_eq!(
            ready(guard.restore()),
            if panic_activation {
                Err(Error::Unavailable)
            } else {
                Ok(Receipt::Restored)
            }
        );
    }
}

#[test]
fn failed_restore_is_never_relabelled_success_by_repoll_or_completion() {
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let io = Arc::new(FaultIo {
        fail_restore: true,
        ..FaultIo::default()
    });
    let mut guard = NativeInteractiveTerminal::new(slave);
    guard.io = io.clone();
    ready(guard.activate()).unwrap();
    assert_eq!(ready(guard.restore()), Err(Error::RestoreFailed));
    joined(&guard);
    assert_eq!(ready(guard.restore()), Err(Error::RestoreFailed));
    assert_eq!(io.sets.load(Ordering::SeqCst), 2);
    assert!(!same_settings(&original, &settings(&alias)));
    NativeIo
        .set(&alias, OptionalActions::Now, &original)
        .unwrap();
}

#[test]
fn dropped_restore_future_settles_on_worker_and_wakes_without_busy_polling() {
    struct CountWake(AtomicUsize);
    impl std::task::Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let io = Arc::new(FaultIo {
        block_set: 1,
        blocked: AtomicBool::new(true),
        ..FaultIo::default()
    });
    let release = ReleaseBlocked(io.clone());
    let mut guard = NativeInteractiveTerminal::new(slave);
    guard.io = io.clone();
    ready(guard.activate()).unwrap();
    let notifications = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(notifications.clone());
    let mut restoration = guard.restore();
    assert!(
        restoration
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    until(|| io.entered.load(Ordering::Acquire));
    drop(restoration);
    assert!(!guard.completion().is_complete());
    assert_eq!(notifications.0.load(Ordering::SeqCst), 0);
    drop(release);
    joined(&guard);
    assert_eq!(notifications.0.load(Ordering::SeqCst), 1);
    assert!(same_settings(&original, &settings(&alias)));
    assert_eq!(ready(guard.restore()), Ok(Receipt::Restored));
}

#[test]
fn snapshot_and_readback_failures_never_publish_unverified_success() {
    for failed_get_at in 0..3 {
        let (_master, slave) = pty();
        let alias = slave.try_clone().unwrap();
        let original = settings(&alias);
        let mut guard = NativeInteractiveTerminal::new(slave);
        guard.io = Arc::new(FaultIo {
            failed_get_at: Some(failed_get_at),
            ..FaultIo::default()
        });
        let expected_activation = match failed_get_at {
            0 => Err(Error::Unavailable),
            1 => Err(Error::ActivationFailed),
            _ => Ok(()),
        };
        assert_eq!(ready(guard.activate()), expected_activation);
        let expected_restore = match failed_get_at {
            0 => Ok(Receipt::NotActivated),
            1 => Ok(Receipt::Restored),
            _ => Err(Error::RestoreFailed),
        };
        assert_eq!(ready(guard.restore()), expected_restore);
        joined(&guard);
        assert!(same_settings(&original, &settings(&alias)));
    }
}

#[test]
fn invalid_descriptors_and_closed_worker_admission_do_not_mutate() {
    let (pipe, _write) = std::io::pipe().unwrap();
    for file in [
        File::from(std::os::fd::OwnedFd::from(pipe)),
        File::open("/dev/null").unwrap(),
    ] {
        let mut guard = NativeInteractiveTerminal::new(file);
        assert_eq!(ready(guard.activate()), Err(Error::InvalidTerminal));
        joined(&guard);
        assert_eq!(ready(guard.restore()), Ok(Receipt::NotActivated));
    }
    let (_master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = settings(&alias);
    let mut guard = NativeInteractiveTerminal::new(slave);
    guard.scope.close();
    assert_eq!(ready(guard.activate()), Err(Error::Unavailable));
    joined(&guard);
    assert!(same_settings(&original, &settings(&alias)));
    assert_eq!(ready(guard.restore()), Ok(Receipt::NotActivated));
}
