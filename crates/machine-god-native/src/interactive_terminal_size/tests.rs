use super::*;
use rustix::fs::{Mode, OFlags};
use rustix::termios::Winsize;
use std::sync::{Condvar, Mutex};
use std::task::{Context, Poll, Waker};
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

fn dimensions(file: &File, columns: u16) {
    rustix::termios::tcsetwinsize(
        file,
        Winsize {
            ws_row: 0,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();
}

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "dimensions observation expired");
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

fn join(completion: &NativeOwnedWorkerCompletion) {
    until(|| completion.is_complete());
    completion.wait_on_worker().unwrap();
}

#[test]
fn construction_and_unpolled_read_are_inert_and_drop_closes_empty_scope() {
    let (_master, slave) = pty();
    let mut reader = NativeInteractiveTerminalSizeReader::new(slave);
    let completion = reader.completion();
    drop(reader.read_columns());
    assert!(!reader.active.load(Ordering::Acquire));
    assert!(!completion.is_complete());
    assert_eq!(Arc::strong_count(&reader.tty), 1);
    drop(reader);
    join(&completion);
}

#[test]
fn actual_tty_refresh_reads_columns_without_changing_modes_or_flags() {
    let (master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let flags = rustix::fs::fcntl_getfl(&alias).unwrap();
    let original = format!("{:?}", rustix::termios::tcgetattr(&alias).unwrap());
    let mut reader = NativeInteractiveTerminalSizeReader::new(slave);
    for columns in [1, 80, 132, u16::MAX] {
        dimensions(&master, columns);
        assert_eq!(
            ready(reader.read_columns()),
            NonZeroU16::new(columns).ok_or(Error::InvalidColumns)
        );
        assert_eq!(rustix::fs::fcntl_getfl(&alias).unwrap(), flags);
        assert_eq!(
            format!("{:?}", rustix::termios::tcgetattr(&alias).unwrap()),
            original
        );
    }
    reader.close();
    join(&reader.completion());
}

#[test]
fn zero_columns_are_an_error_and_later_resize_is_observed() {
    let (master, slave) = pty();
    dimensions(&master, 0);
    let mut reader = NativeInteractiveTerminalSizeReader::new(slave);
    assert_eq!(ready(reader.read_columns()), Err(Error::InvalidColumns));
    dimensions(&master, 44);
    assert_eq!(ready(reader.read_columns()).unwrap().get(), 44);
    reader.close();
    join(&reader.completion());
}

#[test]
fn pipes_and_non_tty_character_devices_are_rejected() {
    let (pipe, _writer) = std::io::pipe().unwrap();
    for source in [
        File::from(std::os::fd::OwnedFd::from(pipe)),
        File::open("/dev/null").unwrap(),
    ] {
        let mut reader = NativeInteractiveTerminalSizeReader::new(source);
        assert_eq!(ready(reader.read_columns()), Err(Error::InvalidTerminal));
        reader.close();
        join(&reader.completion());
    }
}

#[test]
fn closed_admission_is_fixed_error_and_releases_read_permit() {
    let (_master, slave) = pty();
    let mut reader = NativeInteractiveTerminalSizeReader::new(slave);
    reader.close();
    for _ in 0..2 {
        assert_eq!(ready(reader.read_columns()), Err(Error::Unavailable));
        assert!(!reader.active.load(Ordering::Acquire));
    }
    join(&reader.completion());
}

#[derive(Default)]
struct BlockedIo {
    entered: AtomicBool,
    released: Mutex<bool>,
    changed: Condvar,
}
impl SizeIo for BlockedIo {
    fn columns(&self, tty: &File) -> Result<NonZeroU16, Error> {
        self.entered.store(true, Ordering::Release);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
        NativeSizeIo.columns(tty)
    }
}
struct Release(Arc<BlockedIo>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.changed.notify_all();
    }
}

#[test]
fn dropped_started_read_stays_bounded_and_owned_until_worker_settles() {
    let (master, slave) = pty();
    dimensions(&master, 77);
    let mut reader = NativeInteractiveTerminalSizeReader::new(slave);
    let file = Arc::downgrade(&reader.tty);
    let io = Arc::new(BlockedIo::default());
    let release = Release(io.clone());
    reader.io = io.clone();
    let completion = reader.completion();
    let mut future = reader.read_columns();
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    until(|| io.entered.load(Ordering::Acquire));
    drop(future);
    assert_eq!(ready(reader.read_columns()), Err(Error::Busy));
    drop(reader);
    assert!(!completion.is_complete());
    assert!(file.upgrade().is_some());
    drop(release);
    join(&completion);
    assert!(file.upgrade().is_none());
}
