//! Fresh, live-helper-authenticated process authority for a private tmux pane.
//!
//! A pane is not our direct child. Neither its numeric PID nor SID is signal
//! authority. Every delivery uses a retained OS incarnation; a separately
//! authenticated supervisor or inert sentinel pins session discovery after the
//! shell exits, independently of the actual terminal job's completion status.

#[cfg(target_os = "macos")]
use super::macos_scope_members_with;
use super::{
    BackgroundProcessError, BackgroundProcessSignal, GROUP_SNAPSHOT_TIMEOUT, cancelled_error,
    cleanup_error, invariant_error, process_signal, spawn_error,
};
#[cfg(target_os = "linux")]
use super::{
    BudgetedLinuxSignalFd, GroupSnapshotAuthority, LinuxProcIoBudget, LinuxProcStat,
    LinuxSignalDescriptorBudget, MAX_LINUX_PROC_STAT_BYTES, budgeted_linux_signal_open,
    linux_proc_record_buffer, linux_scope_members_with, read_linux_proc_stat,
    signal_pinned_linux_process,
};
use machine_god_core::CancellationToken;
#[cfg(target_os = "linux")]
use rustix::fd::AsFd as _;
use std::collections::HashSet;
use std::io::{self, Read as _, Write as _};
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

const MAX_MEMBERS: usize = super::MAX_CAPTURED_GROUP_MEMBERS;

struct CaptureBudget {
    #[cfg(target_os = "linux")]
    descriptors: LinuxSignalDescriptorBudget,
}

impl CaptureBudget {
    fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            descriptors: LinuxSignalDescriptorBudget::production(),
        }
    }
}

struct PinnedProcess {
    pid: rustix::process::Pid,
    #[cfg(target_os = "linux")]
    handle: BudgetedLinuxSignalFd,
    #[cfg(target_os = "macos")]
    handle: machine_god_terminal_sys::ProcessIdentity,
}

impl PinnedProcess {
    fn capture(pid: NonZeroU32, budget: &CaptureBudget) -> Result<Self, BackgroundProcessError> {
        #[cfg(target_os = "macos")]
        let _ = budget;
        let native =
            rustix::process::Pid::from_raw(i32::try_from(pid.get()).map_err(|_| spawn_error())?)
                .ok_or_else(spawn_error)?;
        #[cfg(target_os = "linux")]
        let handle = budgeted_linux_signal_open(&budget.descriptors, || {
            rustix::process::pidfd_open(native, rustix::process::PidfdFlags::empty())
        })
        .map_err(|_| spawn_error())?
        .map_err(|_| spawn_error())?;
        #[cfg(target_os = "macos")]
        let handle =
            machine_god_terminal_sys::ProcessIdentity::capture(pid).map_err(|_| spawn_error())?;
        Ok(Self {
            pid: native,
            handle,
        })
    }

    fn exists(&self) -> Result<bool, BackgroundProcessError> {
        #[cfg(target_os = "linux")]
        {
            use rustix::event::{PollFd, PollFlags, Timespec, poll};
            let mut fds = [PollFd::new(&self.handle, PollFlags::IN)];
            let timeout = Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => Ok(true),
                Ok(_) if fds[0].revents().intersects(PollFlags::IN | PollFlags::HUP) => Ok(false),
                _ => Err(cleanup_error()),
            }
        }
        #[cfg(target_os = "macos")]
        self.handle.exists().map_err(|_| cleanup_error())
    }

    fn in_session(&self, session: rustix::process::Pid) -> Result<bool, BackgroundProcessError> {
        // If the exact retained incarnation is still live after getsid, its PID
        // could not have been reused during the query. Never signal by this PID.
        let same = match rustix::process::getsid(Some(self.pid)) {
            Ok(current) => current == session,
            Err(rustix::io::Errno::SRCH) => false,
            Err(_) => return Err(cleanup_error()),
        };
        Ok(self.exists()? && same)
    }

    fn capture_session_member(
        pid: rustix::process::Pid,
        budget: &CaptureBudget,
        anchor: &Self,
        session: rustix::process::Pid,
    ) -> Result<Option<Self>, BackgroundProcessError> {
        let raw = NonZeroU32::new(pid.as_raw_nonzero().get().cast_unsigned())
            .ok_or_else(cleanup_error)?;
        let process = match Self::capture(raw, budget) {
            Ok(process) => process,
            Err(error) => {
                // A disappearing snapshot row is ordinary; inaccessible or
                // uncertain rows are not silently reported as absent.
                if rustix::process::getsid(Some(pid)) == Err(rustix::io::Errno::SRCH) {
                    return Ok(None);
                }
                return Err(error);
            }
        };
        if !process.in_session(session)? {
            return Ok(None);
        }
        // The exact anchor was live before this membership observation and
        // must still be live before its new pin grants cleanup authority.
        if !anchor.in_session(session)? {
            return Err(cleanup_error());
        }
        Ok(Some(process))
    }

    /// Only retained descendants use this operation; root/anchor exit receipts
    /// belong to their separate owners. An exited nonchild may still become
    /// our adopted child, including after leaving the original session.
    #[cfg(target_os = "linux")]
    fn cleanup_pending(&self, deadline: Instant) -> Result<bool, BackgroundProcessError> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        if Instant::now() >= deadline {
            return Err(cleanup_error());
        }
        let mut fds = [PollFd::new(&self.handle, PollFlags::IN)];
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        poll(&mut fds, Some(&timeout)).map_err(|_| cleanup_error())?;
        let events = fds[0].revents();
        if events.intersects(PollFlags::ERR | PollFlags::NVAL) {
            return Err(cleanup_error());
        }
        if events.contains(PollFlags::HUP) {
            return Ok(false);
        }
        if !events.contains(PollFlags::IN) {
            return Ok(true);
        }
        self.exited_cleanup_pending(deadline)
    }

    /// Caller has positively observed complete thread-group exit on this fd.
    #[cfg(target_os = "linux")]
    fn exited_cleanup_pending(&self, deadline: Instant) -> Result<bool, BackgroundProcessError> {
        if Instant::now() >= deadline {
            return Err(cleanup_error());
        }
        match rustix::process::waitid(
            rustix::process::WaitId::PidFd(self.handle.as_fd()),
            rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG,
        ) {
            Ok(Some(_)) => Ok(false),
            Ok(None) => Ok(true),
            Err(rustix::io::Errno::CHILD) => {
                if Instant::now() >= deadline {
                    return Err(cleanup_error());
                }
                // Older Linux pidfds report IN, without HUP, even after reap.
                // IN proves the entire thread group has exited: this exact-fd
                // signal cannot affect executing code. ESRCH proves removal;
                // success means the zombie remains and ECHILD is not absence.
                match rustix::process::pidfd_send_signal(
                    &self.handle,
                    rustix::process::Signal::KILL,
                ) {
                    Err(rustix::io::Errno::SRCH) => Ok(false),
                    Ok(()) => Ok(true),
                    Err(_) => Err(cleanup_error()),
                }
            }
            Err(_) => Err(cleanup_error()),
        }
    }

    fn signal(&self, signal: rustix::process::Signal) -> Result<(), BackgroundProcessError> {
        #[cfg(target_os = "linux")]
        return signal_pinned_linux_process(self.handle.as_fd(), signal)
            .map_err(|_| cleanup_error());
        #[cfg(target_os = "macos")]
        {
            for _ in 0..3 {
                match self.handle.signal(signal) {
                    Ok(()) => return Ok(()),
                    Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                        if !self.exists()? {
                            return Ok(());
                        }
                    }
                    Err(_) => return Err(cleanup_error()),
                }
            }
            Err(cleanup_error())
        }
    }
}

/// Exclusive live authority; deliberately neither serializable nor clonable.
pub(crate) struct AuthenticatedTerminalProcess {
    budget: CaptureBudget,
    root: PinnedProcess,
    anchor: Option<PinnedProcess>,
    members: Vec<PinnedProcess>,
    anchor_retiring: bool,
    empty_inventory: bool,
    supervisor: bool,
    #[cfg(target_os = "linux")]
    authority: GroupSnapshotAuthority,
}

impl AuthenticatedTerminalProcess {
    /// The trusted launcher already checked this private connection's initial
    /// nonce/PID and exact pane identity. Acquire the OS incarnation FIRST, then
    /// issue a fresh challenge to the still-blocked helper over that connection.
    /// The helper responds with `challenge[i] ^ nonce[i]`, without exec/fork.
    pub(crate) fn authenticate(
        pid: NonZeroU32,
        gate: &mut UnixStream,
        nonce: &[u8; 32],
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self, BackgroundProcessError> {
        #[cfg(target_os = "macos")]
        machine_god_terminal_sys::ProcessIdentity::verify_signal_support()
            .map_err(|_| spawn_error())?;
        let budget = CaptureBudget::new();
        let root = authenticate_process(pid, gate, nonce, deadline, cancellation, &budget)?;
        if !root.in_session(root.pid)? {
            return Err(spawn_error());
        }
        #[cfg(target_os = "linux")]
        let authority = GroupSnapshotAuthority::open()?;
        require_time(deadline, cancellation)?;
        Ok(Self {
            budget,
            root,
            anchor: None,
            members: Vec::new(),
            anchor_retiring: false,
            empty_inventory: false,
            supervisor: false,
            #[cfg(target_os = "linux")]
            authority,
        })
    }

    /// Retain the authenticated pane helper as its own session anchor. The
    /// trusted helper supervises one gated child and never forks again after
    /// reporting that child's exact outcome to the host.
    pub(crate) fn retain_self_as_anchor(&mut self) -> Result<(), BackgroundProcessError> {
        if self.anchor.is_some() || self.supervisor || !self.root.in_session(self.root.pid)? {
            return Err(invariant_error());
        }
        self.supervisor = true;
        Ok(())
    }

    /// Attach a distinct, inert same-session sentinel while the root remains
    /// gated. The sentinel must never fork after its authentication and must
    /// remain alive until this owner retires it. Its group must differ from the
    /// interactive root group so terminal-generated signals cannot kill it.
    pub(crate) fn authenticate_anchor(
        &mut self,
        pid: NonZeroU32,
        gate: &mut UnixStream,
        nonce: &[u8; 32],
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(), BackgroundProcessError> {
        if self.supervisor
            || self.anchor.is_some()
            || pid.get() == self.root.pid.as_raw_nonzero().get().cast_unsigned()
        {
            return Err(invariant_error());
        }
        let anchor = authenticate_process(pid, gate, nonce, deadline, cancellation, &self.budget)?;
        if !anchor.in_session(self.root.pid)?
            || !self.root.in_session(self.root.pid)?
            || rustix::process::getpgid(Some(anchor.pid))
                == rustix::process::getpgid(Some(self.root.pid))
        {
            return Err(spawn_error());
        }
        require_time(deadline, cancellation)?;
        self.anchor = Some(anchor);
        Ok(())
    }

    pub(crate) fn validate_identity(
        &mut self,
        pid: NonZeroU32,
    ) -> Result<(), BackgroundProcessError> {
        if pid.get() != self.root.pid.as_raw_nonzero().get().cast_unsigned() {
            return Err(invariant_error());
        }
        self.refresh()
    }

    /// Discover only under a continuously live exact same-session anchor. New
    /// captures revalidate the anchor before entering retained ownership. A
    /// later failed observation cannot discard an already established pin, and
    /// a reused numeric SID can never expand this ownership set.
    fn refresh(&mut self) -> Result<(), BackgroundProcessError> {
        if self.anchor_retiring {
            return Ok(());
        }
        let anchor = self.anchor.as_ref().unwrap_or(&self.root);
        let deadline = Instant::now() + GROUP_SNAPSHOT_TIMEOUT;
        if !anchor.in_session(self.root.pid)? {
            return Err(cleanup_error());
        }
        #[cfg(target_os = "linux")]
        for index in (0..self.members.len()).rev() {
            // Release positively settled obligations before any new capture.
            // This both preserves escaped zombies and permits progress at the
            // descriptor cap without requiring a second fd for a known job.
            if !self.members[index].cleanup_pending(deadline)? {
                self.members.swap_remove(index);
            }
        }
        #[cfg(target_os = "linux")]
        let retained: HashSet<_> = self.members.iter().map(|member| member.pid).collect();
        if Instant::now() >= deadline {
            return Err(cleanup_error());
        }
        #[cfg(target_os = "linux")]
        let snapshot = {
            let mut record = linux_proc_record_buffer(MAX_LINUX_PROC_STAT_BYTES);
            linux_scope_members_with(
                &self.authority,
                self.root.pid,
                true,
                |pid, directory, stat, budget| {
                    if pid != self.root.pid && pid != anchor.pid && !retained.contains(&pid) {
                        reap_adopted_zombie(
                            pid,
                            directory,
                            stat,
                            &self.budget,
                            &mut record,
                            budget,
                            anchor,
                        )?;
                    }
                    Ok(())
                },
            )?
        };
        #[cfg(target_os = "macos")]
        let snapshot =
            capture_macos_scope_members(&mut self.members, anchor, self.root.pid, deadline)?;
        let empty_inventory = snapshot.iter().all(|member| member.pid == anchor.pid);
        let mut known = retain_pending_members(&mut self.members, anchor, self.root.pid, deadline)?;
        for member in snapshot {
            if Instant::now() >= deadline {
                return Err(cleanup_error());
            }
            if member.pid == self.root.pid || member.pid == anchor.pid {
                continue;
            }
            // Only live or unresolved retained identities enter this set; a
            // positively settled dead identity cannot hide a reused PID.
            if known.contains(&member.pid) {
                continue;
            }
            if known.len() >= MAX_MEMBERS {
                return Err(cleanup_error());
            }
            if let Some(process) = PinnedProcess::capture_session_member(
                member.pid,
                &self.budget,
                anchor,
                self.root.pid,
            )? {
                // Transfer established authority before any later fallible
                // work, including deadlines, captures and anchor observations.
                known.insert(process.pid);
                retain_captured_member(&mut self.members, process, deadline)?;
                if Instant::now() >= deadline {
                    return Err(cleanup_error());
                }
            }
        }
        if !anchor.in_session(self.root.pid)? {
            return Err(cleanup_error());
        }
        self.empty_inventory = empty_inventory;
        Ok(())
    }

    pub(crate) fn signal(
        &mut self,
        signal: BackgroundProcessSignal,
    ) -> Result<(), BackgroundProcessError> {
        let observation = self.refresh();
        // A failed snapshot cannot revoke already captured cleanup authority.
        let signal = process_signal(signal);
        let mut failed = observation.is_err();
        let deadline = Instant::now() + GROUP_SNAPSHOT_TIMEOUT;
        for member in self.members.iter().rev() {
            if Instant::now() >= deadline {
                failed = true;
                break;
            }
            failed |= member.signal(signal).is_err();
        }
        if !self.supervisor {
            failed |= self.root.signal(signal).is_err();
        }
        if failed { Err(cleanup_error()) } else { Ok(()) }
    }

    /// Retire the inert sentinel only after a complete anchored snapshot proves
    /// every real job absent. Until then, even root exit is not scope absence.
    pub(crate) fn is_absent(&mut self) -> Result<bool, BackgroundProcessError> {
        if self.supervisor {
            return if self.anchor_retiring {
                Ok(!self.root.exists()?)
            } else {
                self.refresh()?;
                Ok(false)
            };
        }
        self.refresh()?;
        // A parent observed in the snapshot may fork and exit before the live
        // checks below. Require a snapshot with NO real jobs, not merely that
        // every previously observed process has subsequently died.
        #[cfg(target_os = "linux")]
        let retained_pending = !self.members.is_empty();
        #[cfg(target_os = "macos")]
        let retained_pending = self.retained_jobs_pending()?;
        if !self.empty_inventory || self.root.exists()? || retained_pending {
            return Ok(false);
        }
        let anchor = self.anchor.as_ref().ok_or_else(cleanup_error)?;
        if !self.anchor_retiring {
            anchor.signal(rustix::process::Signal::KILL)?;
            self.anchor_retiring = true;
        }
        Ok(!anchor.exists()?)
    }

    /// Complete anchored inventory excluding the supervisor. Merely observing
    /// that previously seen parents have died cannot exclude a last child fork.
    pub(crate) fn jobs_absent(&mut self) -> Result<bool, BackgroundProcessError> {
        if !self.supervisor || self.anchor_retiring {
            return Err(invariant_error());
        }
        self.refresh()?;
        if !self.empty_inventory {
            return Ok(false);
        }
        #[cfg(target_os = "linux")]
        return Ok(self.members.is_empty());
        #[cfg(target_os = "macos")]
        return Ok(!self.retained_jobs_pending()?);
    }

    #[cfg(target_os = "macos")]
    fn retained_jobs_pending(&self) -> Result<bool, BackgroundProcessError> {
        let deadline = Instant::now() + GROUP_SNAPSHOT_TIMEOUT;
        for member in &self.members {
            if Instant::now() >= deadline {
                return Err(cleanup_error());
            }
            if member.exists()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The host must consume the exact child outcome before this call. Keep the
    /// live supervisor until a complete anchored inventory proves no real jobs.
    pub(crate) fn retire_anchor(&mut self) -> Result<(), BackgroundProcessError> {
        if !self.jobs_absent()? {
            return Err(cleanup_error());
        }
        self.root.signal(rustix::process::Signal::KILL)?;
        self.anchor_retiring = true;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn capture_macos_scope_members(
    members: &mut Vec<PinnedProcess>,
    anchor: &PinnedProcess,
    session: rustix::process::Pid,
    deadline: Instant,
) -> Result<Vec<super::CapturedGroupMember>, BackgroundProcessError> {
    let mut known = retain_pending_members(members, anchor, session, deadline)?;
    macos_scope_members_with(session, true, |member, identity| {
        if member.pid == session || member.pid == anchor.pid || known.contains(&member.pid) {
            return Ok(());
        }
        if known.len() >= MAX_MEMBERS || Instant::now() >= deadline {
            return Err(cleanup_error());
        }
        // The scanner authenticated this unique ID on both sides of its SID
        // observation. Later SID escape cannot revoke established ownership.
        // Carry the scanner's authenticated opaque identity directly. A second
        // fallible query here would itself reopen a prefix-loss boundary.
        let handle = identity.ok_or_else(cleanup_error)?;
        if !anchor.in_session(session)? {
            return Err(cleanup_error());
        }
        known.insert(member.pid);
        retain_captured_member(
            members,
            PinnedProcess {
                pid: member.pid,
                handle,
            },
            deadline,
        )
    })
}

fn retain_captured_member(
    members: &mut Vec<PinnedProcess>,
    process: PinnedProcess,
    deadline: Instant,
) -> Result<(), BackgroundProcessError> {
    #[cfg(test)]
    let pid = process.pid;
    members.push(process);
    if Instant::now() >= deadline {
        return Err(cleanup_error());
    }
    #[cfg(test)]
    if tests::FAIL_AFTER_CAPTURE
        .compare_exchange(
            pid.as_raw_nonzero().get().cast_unsigned(),
            0,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_ok()
    {
        return Err(cleanup_error());
    }
    Ok(())
}

fn retain_pending_members(
    members: &mut Vec<PinnedProcess>,
    anchor: &PinnedProcess,
    session: rustix::process::Pid,
    deadline: Instant,
) -> Result<HashSet<rustix::process::Pid>, BackgroundProcessError> {
    let mut known = HashSet::new();
    let mut alive = Vec::with_capacity(members.len());
    for member in &*members {
        if Instant::now() >= deadline {
            return Err(cleanup_error());
        }
        #[cfg(target_os = "linux")]
        let exists = true; // Live or unresolved exact reap obligation.
        #[cfg(target_os = "macos")]
        let exists = member.exists()?;
        alive.push(exists);
        if exists {
            known.insert(member.pid);
        }
    }
    if !anchor.in_session(session)? {
        return Err(cleanup_error());
    }
    // Settle the old prefix before extending ownership. Keeping positively
    // dead macOS entries across repeated later failures would otherwise
    // accumulate stale slots outside the existing member bound.
    let mut alive = alive.into_iter();
    members.retain(|_| alive.next().unwrap_or(true));
    Ok(known)
}

/// A subreaper host can adopt an exited grandchild from the authenticated SID.
/// Reap only a zombie positively rebound through its retained proc directory to
/// an exact pidfd. Never reap the supervisor/direct shell or by numeric PID.
/// The visitor keeps this row in its snapshot even after successful reaping:
/// only a subsequent complete anchored scan may prove an empty job inventory.
#[cfg(target_os = "linux")]
fn reap_adopted_zombie(
    pid: rustix::process::Pid,
    directory: rustix::fd::BorrowedFd<'_>,
    expected: &LinuxProcStat,
    capture: &CaptureBudget,
    record: &mut Vec<u8>,
    budget: &mut LinuxProcIoBudget,
    anchor: &PinnedProcess,
) -> Result<(), BackgroundProcessError> {
    if !expected.zombie || expected.parent != Some(rustix::process::getpid()) {
        return Ok(());
    }
    budget.preflight()?;
    // One transient pidfd is charged to the existing per-scope/global quota;
    // the one stat fd/read uses the current scan's original time/byte budget.
    let process = match budgeted_linux_signal_open(&capture.descriptors, || {
        rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
    })
    .map_err(|_| cleanup_error())?
    {
        Ok(process) => process,
        Err(rustix::io::Errno::SRCH) => return Ok(()),
        Err(_) => return Err(cleanup_error()),
    };
    let stat = match rustix::fs::openat(
        directory,
        "stat",
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    ) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::SRCH) => return Ok(()),
        Err(_) => return Err(cleanup_error()),
    };
    let Some(actual) = read_linux_proc_stat(&mut std::fs::File::from(stat), record, pid, budget)?
    else {
        return Ok(());
    };
    if actual.start_time != expected.start_time
        || !actual.zombie
        || actual.parent != expected.parent
        || actual.session != expected.session
    {
        return Ok(());
    }
    // New-row ownership must be established before this irreversible effect,
    // not merely by the full scan's final anchor check. A zombie cannot fork
    // or change SID between this positive incarnation proof and exact waitid.
    if !anchor.in_session(expected.session.ok_or_else(cleanup_error)?)? {
        return Err(cleanup_error());
    }
    budget.require_live()?;
    match rustix::process::waitid(
        rustix::process::WaitId::PidFd(process.as_fd()),
        rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG,
    ) {
        Ok(_) | Err(rustix::io::Errno::CHILD | rustix::io::Errno::SRCH) => budget.require_live(),
        Err(_) => Err(cleanup_error()),
    }
}

impl Drop for AuthenticatedTerminalProcess {
    fn drop(&mut self) {
        // Best-effort crash/failed-launch fallback, never a successful cleanup
        // receipt. Normal owners retain this value until is_absent proves exit.
        let _ = self.signal(BackgroundProcessSignal::Kill);
        if self.supervisor {
            let _ = self.root.signal(rustix::process::Signal::KILL);
        }
        if let Some(anchor) = &self.anchor {
            let _ = anchor.signal(rustix::process::Signal::KILL);
        }
    }
}

fn authenticate_process(
    pid: NonZeroU32,
    gate: &mut UnixStream,
    nonce: &[u8; 32],
    deadline: Instant,
    cancellation: &CancellationToken,
    budget: &CaptureBudget,
) -> Result<PinnedProcess, BackgroundProcessError> {
    if !nonce.iter().all(u8::is_ascii_hexdigit) {
        return Err(spawn_error());
    }
    require_time(deadline, cancellation)?;
    let process = PinnedProcess::capture(pid, budget)?;
    let mut challenge = [0; 32];
    getrandom::fill(&mut challenge).map_err(|_| spawn_error())?;
    gate.set_nonblocking(true).map_err(|_| spawn_error())?;
    let mut offset = 0;
    while offset < challenge.len() {
        require_time(deadline, cancellation)?;
        match gate.write(&challenge[offset..]) {
            Ok(0) => return Err(spawn_error()),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return Err(spawn_error()),
        }
    }
    let mut response = [0; 32];
    offset = 0;
    while offset < response.len() {
        require_time(deadline, cancellation)?;
        match gate.read(&mut response[offset..]) {
            Ok(0) => return Err(spawn_error()),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return Err(spawn_error()),
        }
    }
    let mismatch = response
        .iter()
        .zip(challenge)
        .zip(nonce)
        .fold(0, |mismatch, ((response, challenge), nonce)| {
            mismatch | (response ^ challenge ^ nonce)
        });
    require_time(deadline, cancellation)?;
    if mismatch != 0 || !process.exists()? {
        return Err(spawn_error());
    }
    Ok(process)
}

fn require_time(
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), BackgroundProcessError> {
    if cancellation.is_cancelled() {
        Err(cancelled_error())
    } else if Instant::now() >= deadline {
        Err(spawn_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::BackgroundProcessErrorKind;
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::os::unix::process::CommandExt as _;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};

    const NONCE: &[u8; 32] = b"0123456789abcdef0123456789abcdef";
    pub(super) static FAIL_AFTER_CAPTURE: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);

    #[test]
    fn failed_refresh_retains_authenticated_live_member() {
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        authority.retain_self_as_anchor().unwrap();
        let pid = fixture.release(b's');
        FAIL_AFTER_CAPTURE.store(pid, std::sync::atomic::Ordering::SeqCst);
        let failed = authority.jobs_absent().is_err();
        let retained = authority
            .members
            .iter()
            .any(|member| member.pid.as_raw_nonzero().get().cast_unsigned() == pid);
        authority.signal(BackgroundProcessSignal::Kill).unwrap();
        fixture.outcome();
        eventually(|| authority.jobs_absent().unwrap());
        authority.retire_anchor().unwrap();
        fixture.child.wait().unwrap();
        assert!(failed, "injected post-capture failure must fire");
        assert!(retained, "failed observation must retain the acquired pin");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_refresh_retains_observed_descendant_after_session_escape() {
        const CHILD: &str = "MACHINE_GOD_TMUX_FAILED_REFRESH_REGRESSION";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "background_process::terminal_tmux_process::tests::failed_refresh_retains_observed_descendant_after_session_escape", "--nocapture", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap();
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        authority.retain_self_as_anchor().unwrap();
        let listener = UnixListener::bind(fixture.directory.join("escape-gate")).unwrap();
        listener.set_nonblocking(true).unwrap();
        fixture.release(b'q');
        let mut connected = None;
        eventually(|| match listener.accept() {
            Ok((gate, _)) => {
                connected = Some(gate);
                true
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("escape gate: {error}"),
        });
        let mut gate = connected.unwrap();
        gate.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes = [0; 4];
        gate.read_exact(&mut bytes).unwrap();
        let raw_pid = u32::from_be_bytes(bytes);
        let pid = rustix::process::Pid::from_raw(i32::try_from(raw_pid).unwrap()).unwrap();
        let observer =
            rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).unwrap();
        FAIL_AFTER_CAPTURE.store(raw_pid, std::sync::atomic::Ordering::SeqCst);
        let failed = authority.jobs_absent().is_err();
        let retained = authority.members.iter().any(|member| member.pid == pid);
        gate.write_all(b"E").unwrap();
        let mut escaped = [0];
        gate.read_exact(&mut escaped).unwrap();
        assert_eq!(escaped, [b'E']);
        fixture.gate.write_all(b"R").unwrap();
        fixture.outcome();
        let premature_absence = authority.jobs_absent().unwrap();
        let child_still_alive = rustix::process::getsid(Some(pid)) == Ok(pid);
        // Clean the independent observer's exact fixture obligation before reporting.
        rustix::process::pidfd_send_signal(&observer, rustix::process::Signal::KILL).unwrap();
        eventually(|| adopted_exit_observed(&observer));
        rustix::process::waitid(
            rustix::process::WaitId::PidFd(observer.as_fd()),
            rustix::process::WaitIdOptions::EXITED,
        )
        .unwrap();
        authority.retire_anchor().unwrap();
        fixture.child.wait().unwrap();
        assert!(failed, "injected post-capture failure must fire");
        assert!(
            retained,
            "later refresh failure must retain established child pins"
        );
        assert!(
            child_still_alive,
            "fixture must retain a live escaped child before cleanup"
        );
        assert!(
            !premature_absence,
            "successful cleanup must not forget an observed live escaped child"
        );
    }

    struct Fixture {
        child: Child,
        gate: UnixStream,
        directory: PathBuf,
    }

    impl Fixture {
        fn start(bad_response: bool) -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let directory =
                std::env::temp_dir().join(format!("mg-tmux-pin-{:x}", u128::from_ne_bytes(random)));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("gate");
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "background_process::terminal_tmux_process::tests::authenticated_process_helper", "--ignored", "--nocapture"])
                .env("MACHINE_GOD_PROCESS_TEST_GATE", &path)
                .env("MACHINE_GOD_PROCESS_TEST_BAD", if bad_response { "1" } else { "0" })
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let gate = loop {
                match listener.accept() {
                    Ok((gate, _)) => break gate,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "helper did not connect");
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("accept helper: {error}"),
                }
            };
            Self {
                child,
                gate,
                directory,
            }
        }

        fn authenticate(&mut self) -> Result<AuthenticatedTerminalProcess, BackgroundProcessError> {
            AuthenticatedTerminalProcess::authenticate(
                NonZeroU32::new(self.child.id()).unwrap(),
                &mut self.gate,
                NONCE,
                Instant::now() + Duration::from_secs(5),
                &CancellationToken::new(),
            )
        }

        fn release(&mut self, mode: u8) -> u32 {
            self.gate.set_nonblocking(false).unwrap();
            self.gate
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            self.gate.write_all(&[mode]).unwrap();
            let mut pid = [0; 4];
            self.gate.read_exact(&mut pid).unwrap();
            u32::from_be_bytes(pid)
        }

        fn outcome(&mut self) {
            let mut outcome = [0];
            self.gate.read_exact(&mut outcome).unwrap();
            assert_eq!(outcome, [b'X']);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    #[ignore = "private subprocess entrypoint; launched by the fixture"]
    fn authenticated_process_helper() {
        let Some(path) = std::env::var_os("MACHINE_GOD_PROCESS_TEST_GATE") else {
            return;
        };
        rustix::process::setsid().unwrap();
        let mut gate = UnixStream::connect(&path).unwrap();
        gate.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut challenge = [0; 32];
        gate.read_exact(&mut challenge).unwrap();
        let mut response = [0; 32];
        for index in 0..32 {
            response[index] = challenge[index] ^ NONCE[index];
        }
        if std::env::var("MACHINE_GOD_PROCESS_TEST_BAD").unwrap() == "1" {
            response[0] ^= 1;
        }
        gate.write_all(&response).unwrap();
        let mut mode = [0];
        if gate.read_exact(&mut mode).is_err() {
            return;
        }
        if mode == [b'e'] {
            // Same-process exec proves the retained incarnation survives exec.
            let error = Command::new("/bin/sleep").arg("100").exec();
            panic!("exec failed: {error}");
        }
        let marker = PathBuf::from(path).with_file_name("descendant");
        let script = if mode == [b'd'] {
            "sleep 100 & printf '%s' \"$!\" > \"$1\"; exit 0"
        } else {
            "exec sleep 100"
        };
        let mut command = if mode == [b'q'] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "background_process::terminal_tmux_process::tests::escaped_descendant_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MACHINE_GOD_ESCAPE_PARENT", "1")
                .env(
                    "MACHINE_GOD_ESCAPE_GATE",
                    marker.with_file_name("escape-gate"),
                );
            command
        } else {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script, "sh"]).arg(&marker);
            command
        };
        let mut child = command
            .process_group(0)
            .stdin(if mode == [b'q'] {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        gate.write_all(&child.id().to_be_bytes()).unwrap();
        if mode == [b'q'] {
            let mut release = [0];
            gate.read_exact(&mut release).unwrap();
            assert_eq!(release, [b'R']);
            child.stdin.take().unwrap().write_all(&release).unwrap();
        }
        child.wait().unwrap();
        gate.write_all(b"X").unwrap();
        // The genuine supervising helper remains inert after exact outcome.
        let _ = gate.read_exact(&mut [0]);
    }

    fn eventually(mut operation: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !operation() {
            assert!(Instant::now() < deadline, "process state did not converge");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(target_os = "linux")]
    fn adopted_exit_observed(observer: &impl rustix::fd::AsFd) -> bool {
        matches!(
            rustix::process::waitid(
                rustix::process::WaitId::PidFd(observer.as_fd()),
                rustix::process::WaitIdOptions::EXITED
                    | rustix::process::WaitIdOptions::NOHANG
                    | rustix::process::WaitIdOptions::NOWAIT
            ),
            Ok(Some(_))
        )
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "private subprocess entrypoint; launched by the escape fixture"]
    fn escaped_descendant_helper() {
        let Some(path) = std::env::var_os("MACHINE_GOD_ESCAPE_GATE") else {
            return;
        };
        let path = PathBuf::from(path);
        if std::env::var_os("MACHINE_GOD_ESCAPE_PARENT").is_some() {
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "background_process::terminal_tmux_process::tests::escaped_descendant_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env_remove("MACHINE_GOD_ESCAPE_PARENT")
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            let mut release = [0];
            std::io::stdin().read_exact(&mut release).unwrap();
            assert_eq!(release, [b'R']);
            // Deliberately leave the exact child waitable until host adoption.
            drop(child);
            return;
        }
        let mut gate = UnixStream::connect(&path).unwrap();
        gate.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        gate.write_all(&std::process::id().to_be_bytes()).unwrap();
        let mut instruction = [0];
        gate.read_exact(&mut instruction).unwrap();
        assert_eq!(instruction, [b'E']);
        rustix::process::setsid().unwrap();
        gate.write_all(b"E").unwrap();
        let _ = gate.read_exact(&mut instruction);
    }

    #[test]
    fn authentication_rejects_wrong_response_invalid_nonce_and_cancellation() {
        let mut fixture = Fixture::start(true);
        assert!(fixture.authenticate().is_err());
        assert!(fixture.child.try_wait().unwrap().is_none());
        let mut fixture = Fixture::start(false);
        assert!(
            AuthenticatedTerminalProcess::authenticate(
                NonZeroU32::new(fixture.child.id()).unwrap(),
                &mut fixture.gate,
                &[b'z'; 32],
                Instant::now() + Duration::from_secs(5),
                &CancellationToken::new(),
            )
            .is_err()
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            AuthenticatedTerminalProcess::authenticate(
                NonZeroU32::new(fixture.child.id()).unwrap(),
                &mut fixture.gate,
                NONCE,
                Instant::now() + Duration::from_secs(5),
                &cancellation,
            )
            .err()
            .unwrap()
            .kind(),
            BackgroundProcessErrorKind::Cancelled
        );
        assert!(fixture.child.try_wait().unwrap().is_none());
        assert!(
            AuthenticatedTerminalProcess::authenticate(
                NonZeroU32::new(fixture.child.id()).unwrap(),
                &mut fixture.gate,
                NONCE,
                Instant::now(),
                &CancellationToken::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn authenticated_incarnation_survives_exec_and_rejects_other_process_identity() {
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        fixture.gate.write_all(b"e").unwrap();
        fixture.gate.set_nonblocking(false).unwrap();
        fixture
            .gate
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert_eq!(
            fixture.gate.read(&mut [0]).unwrap(),
            0,
            "exec closes helper gate"
        );
        assert!(fixture.child.try_wait().unwrap().is_none());
        eventually(|| {
            authority
                .validate_identity(NonZeroU32::new(fixture.child.id()).unwrap())
                .is_ok()
        });
        assert!(
            authority
                .validate_identity(NonZeroU32::new(std::process::id()).unwrap())
                .is_err()
        );
        authority.signal(BackgroundProcessSignal::Kill).unwrap();
        fixture.child.wait().unwrap();
        // An unanchored vanished numeric SID is not a successful empty scope.
        assert!(authority.is_absent().is_err());
        let mut unrelated = Command::new("/bin/sleep")
            .arg("100")
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(authority.signal(BackgroundProcessSignal::Kill).is_err());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }

    #[test]
    fn supervisor_survives_signals_until_exact_outcome_and_explicit_retirement() {
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        authority.retain_self_as_anchor().unwrap();
        fixture.release(b's');
        assert!(!authority.jobs_absent().unwrap());
        assert!(authority.retire_anchor().is_err());
        authority.signal(BackgroundProcessSignal::Kill).unwrap();
        fixture.outcome();
        assert!(fixture.child.try_wait().unwrap().is_none());
        eventually(|| authority.jobs_absent().unwrap());
        assert!(!authority.is_absent().unwrap());
        authority.retire_anchor().unwrap();
        fixture.child.wait().unwrap();
        assert!(authority.is_absent().unwrap());
    }

    #[test]
    fn supervisor_discovers_unseen_descendant_after_shell_has_already_exited() {
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        authority.retain_self_as_anchor().unwrap();
        fixture.release(b'd');
        fixture.outcome();
        // No registry snapshot happened while the shell was alive. The exact
        // supervisor SID anchor authorizes its now-reparented remaining job.
        assert!(!authority.jobs_absent().unwrap());
        assert!(fixture.directory.join("descendant").is_file());
        authority.signal(BackgroundProcessSignal::Kill).unwrap();
        eventually(|| authority.jobs_absent().unwrap());
        authority.retire_anchor().unwrap();
        fixture.child.wait().unwrap();
        assert!(authority.is_absent().unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn supervisor_reaps_adopted_zombie_only_before_a_fresh_empty_snapshot() {
        const CHILD: &str = "MACHINE_GOD_TMUX_SUBREAPER_REGRESSION";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "background_process::terminal_tmux_process::tests::supervisor_reaps_adopted_zombie_only_before_a_fresh_empty_snapshot", "--nocapture", "--test-threads=1"])
                .env(CHILD, "1")
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap();
        let mut unrelated = Command::new("/bin/sh")
            .args(["-c", "exit 23"])
            .spawn()
            .unwrap();
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        authority.retain_self_as_anchor().unwrap();
        fixture.release(b'd');
        fixture.outcome();
        let pid: i32 = std::fs::read_to_string(fixture.directory.join("descendant"))
            .unwrap()
            .parse()
            .unwrap();
        let observer = rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(pid).unwrap(),
            rustix::process::PidfdFlags::empty(),
        )
        .unwrap();
        // Keep this zombie unseen: the visitor, not retained-member cleanup,
        // must reap it without removing its row from the current snapshot.
        rustix::process::pidfd_send_signal(&observer, rustix::process::Signal::KILL).unwrap();
        // Prove this exact adopted child has exited without consuming its status.
        eventually(|| adopted_exit_observed(&observer));
        assert!(
            !authority.jobs_absent().unwrap(),
            "reaping cannot erase a row from the current snapshot"
        );
        assert_eq!(
            rustix::process::waitid(
                rustix::process::WaitId::PidFd(observer.as_fd()),
                rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG
            )
            .unwrap_err(),
            rustix::io::Errno::CHILD
        );
        assert!(
            authority.jobs_absent().unwrap(),
            "next complete anchored snapshot must converge"
        );
        assert!(fixture.child.try_wait().unwrap().is_none());
        assert_eq!(
            unrelated.wait().unwrap().code(),
            Some(23),
            "unrelated child wait authority is untouched"
        );
        authority.retire_anchor().unwrap();
        fixture.child.wait().unwrap();
        assert!(authority.is_absent().unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn supervisor_reaps_retained_descendant_after_session_escape_and_adoption() {
        const CHILD: &str = "MACHINE_GOD_TMUX_ESCAPED_SUBREAPER_REGRESSION";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "background_process::terminal_tmux_process::tests::supervisor_reaps_retained_descendant_after_session_escape_and_adoption", "--nocapture", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap();
        let mut unrelated = Command::new("/bin/sh")
            .args(["-c", "exit 23"])
            .spawn()
            .unwrap();
        let mut fixture = Fixture::start(false);
        let mut authority = fixture.authenticate().unwrap();
        authority.retain_self_as_anchor().unwrap();
        let listener = UnixListener::bind(fixture.directory.join("escape-gate")).unwrap();
        listener.set_nonblocking(true).unwrap();
        fixture.release(b'q');
        let mut connected = None;
        eventually(|| match listener.accept() {
            Ok((gate, _)) => {
                connected = Some(gate);
                true
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("escape gate: {error}"),
        });
        let mut gate = connected.unwrap();
        gate.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes = [0; 4];
        gate.read_exact(&mut bytes).unwrap();
        let pid = rustix::process::Pid::from_raw(i32::try_from(u32::from_be_bytes(bytes)).unwrap())
            .unwrap();
        assert!(!authority.jobs_absent().unwrap());
        assert!(authority.members.iter().any(|member| member.pid == pid));
        // No extra descriptor may be needed to settle already retained jobs.
        authority.budget.descriptors.operation_maximum = authority
            .budget
            .descriptors
            .operation_in_use
            .load(std::sync::atomic::Ordering::Acquire);
        let observer =
            rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).unwrap();
        gate.write_all(b"E").unwrap();
        let mut escaped = [0];
        gate.read_exact(&mut escaped).unwrap();
        assert_eq!(escaped, [b'E']);
        assert_eq!(rustix::process::getsid(Some(pid)).unwrap(), pid);
        assert_ne!(pid, authority.root.pid);
        assert!(
            !authority.jobs_absent().unwrap(),
            "live escaped job remains owned"
        );
        let member = authority
            .members
            .iter()
            .find(|member| member.pid == pid)
            .unwrap();
        member.signal(rustix::process::Signal::KILL).unwrap();
        eventually(|| !member.exists().unwrap());
        assert_eq!(
            rustix::process::waitid(
                rustix::process::WaitId::PidFd(observer.as_fd()),
                rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG
            )
            .unwrap_err(),
            rustix::io::Errno::CHILD
        );
        assert!(!authority.jobs_absent().unwrap());
        assert!(
            authority.members.iter().any(|member| member.pid == pid),
            "ECHILD must not discard the not-yet-adopted zombie"
        );
        fixture.gate.write_all(b"R").unwrap();
        fixture.outcome();
        eventually(|| adopted_exit_observed(&observer));
        assert!(authority.jobs_absent().unwrap());
        assert_eq!(
            rustix::process::waitid(
                rustix::process::WaitId::PidFd(observer.as_fd()),
                rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG
            )
            .unwrap_err(),
            rustix::io::Errno::CHILD
        );
        assert_eq!(unrelated.wait().unwrap().code(), Some(23));
        authority.retire_anchor().unwrap();
        fixture.child.wait().unwrap();
        assert!(authority.is_absent().unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_descendant_already_reaped_settles_without_poll_hup() {
        let mut child = Command::new("/bin/sleep").arg("100").spawn().unwrap();
        let pin =
            PinnedProcess::capture(NonZeroU32::new(child.id()).unwrap(), &CaptureBudget::new())
                .unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!pin.exists().unwrap());
        // Exercise the older-kernel IN-only path even on a kernel with HUP.
        assert!(
            !pin.exited_cleanup_pending(Instant::now() + GROUP_SNAPSHOT_TIMEOUT)
                .unwrap()
        );
        assert!(pin.exited_cleanup_pending(Instant::now()).is_err());
    }
}
