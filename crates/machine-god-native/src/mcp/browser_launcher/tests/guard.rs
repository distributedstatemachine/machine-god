//! Actual-process evidence: run with the coordinator's fresh release-helper gate.

use super::*;
use crate::background_url_opener::launcher::LauncherGuard;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicUsize},
    mpsc::{self, Receiver, SyncSender},
};

const FINAL_SPAWN_CHECK: usize = 4;
const FIRST_CHILD_CHECK: usize = 5;

#[derive(Default)]
struct Observations {
    checks: AtomicUsize,
    drops: AtomicUsize,
    revoked: AtomicBool,
    child: AtomicI32,
    child_reaped_at_drop: AtomicBool,
}

struct Guard {
    observed: Arc<Observations>,
    gate: Option<CheckGate>,
}

struct CheckGate {
    selected: usize,
    reached: SyncSender<()>,
    release: Mutex<Receiver<()>>,
}

impl LauncherGuard for Guard {
    fn check(&self) -> Result<(), NativeBackgroundOpenError> {
        let check = self.observed.checks.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(gate) = &self.gate
            && check == gate.selected
        {
            gate.reached
                .send(())
                .map_err(|_| NativeBackgroundOpenError::Unavailable)?;
            gate.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .map_err(|_| NativeBackgroundOpenError::Unavailable)?;
        }
        if self.observed.revoked.load(Ordering::Acquire) {
            Err(NativeBackgroundOpenError::Cancelled)
        } else {
            Ok(())
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(child) =
            rustix::process::Pid::from_raw(self.observed.child.load(Ordering::Acquire))
        {
            self.observed.child_reaped_at_drop.store(
                matches!(observe_child(child), Err(rustix::io::Errno::CHILD)),
                Ordering::Release,
            );
        }
        self.observed.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn observe_child(
    child: rustix::process::Pid,
) -> rustix::io::Result<Option<rustix::process::WaitIdStatus>> {
    // Never consume the exact launcher's wait status or compete for its reap.
    rustix::process::waitid(
        rustix::process::WaitId::Pid(child),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
}

struct Gate {
    reached: Receiver<()>,
    release: SyncSender<()>,
}

impl Gate {
    fn wait(&self) {
        self.reached.recv_timeout(Duration::from_secs(15)).unwrap();
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        // Declared after Fixture: unwind always releases its worker before
        // Fixture::drop joins. A closed receiver means work already completed.
        let _ = self.release.try_send(());
    }
}

fn gated(observed: &Arc<Observations>, check: usize) -> (Arc<Guard>, Gate) {
    let (reached_tx, reached) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::sync_channel(1);
    (
        Arc::new(Guard {
            observed: Arc::clone(observed),
            gate: Some(CheckGate {
                selected: check,
                reached: reached_tx,
                release: Mutex::new(release_rx),
            }),
        }),
        Gate { reached, release },
    )
}

fn launch(
    fixture: &Fixture,
    guard: Arc<Guard>,
) -> BoxFuture<'static, Result<NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLaunchError>> {
    fixture.launcher.launch_guarded(
        url(),
        CancellationToken::new(),
        CancellationToken::new(),
        deadline(),
        guard,
    )
}

#[test]
fn proof_is_lazy_and_first_poll_rejection_releases_without_child_effect() {
    let fixture = Fixture::new("printf effect > \"$MG_BROWSER_OUTPUT\"");
    let observed = Arc::new(Observations::default());
    observed.revoked.store(true, Ordering::Release);
    let guard = Arc::new(Guard {
        observed: Arc::clone(&observed),
        gate: None,
    });
    let weak = Arc::downgrade(&guard);
    let operation = launch(&fixture, guard);
    assert_eq!(observed.checks.load(Ordering::Acquire), 0);
    assert!(weak.upgrade().is_some());
    drop(operation);
    assert!(weak.upgrade().is_none());
    assert_eq!(observed.drops.load(Ordering::Acquire), 1);
    assert_eq!(observed.checks.load(Ordering::Acquire), 0);

    let guard = Arc::new(Guard {
        observed: Arc::clone(&observed),
        gate: None,
    });
    assert_eq!(
        block_on(launch(&fixture, guard)),
        Err(NativeMcpBrowserLaunchError::Cancelled)
    );
    fixture.settle();
    assert_eq!(observed.checks.load(Ordering::Acquire), 1);
    assert_eq!(observed.drops.load(Ordering::Acquire), 2);
    assert!(!fixture.directory.join("output").exists());
}

#[test]
fn revoked_proof_at_final_spawn_checkpoint_prevents_child_effect() {
    let fixture = Fixture::new("printf effect > \"$MG_BROWSER_OUTPUT\"");
    let observed = Arc::new(Observations::default());
    let (guard, gate) = gated(&observed, FINAL_SPAWN_CHECK);
    let weak = Arc::downgrade(&guard);
    let mut operation = launch(&fixture, guard);
    assert!(poll(&mut operation).is_pending());
    gate.wait();
    // First poll, worker entry and command preparation have all succeeded;
    // spawn_checked has reserved cleanup but has not performed OS spawn.
    assert_eq!(observed.checks.load(Ordering::Acquire), FINAL_SPAWN_CHECK);
    assert!(!fixture.directory.join("output").exists());
    assert!(weak.upgrade().is_some());
    observed.revoked.store(true, Ordering::Release);
    drop(gate);
    assert_eq!(
        block_on(operation),
        Err(NativeMcpBrowserLaunchError::Cancelled)
    );
    fixture.settle();
    assert_eq!(observed.checks.load(Ordering::Acquire), FINAL_SPAWN_CHECK);
    assert_eq!(observed.drops.load(Ordering::Acquire), 1);
    assert!(weak.upgrade().is_none());
    assert!(!fixture.directory.join("output").exists());
}

#[test]
fn dropped_observer_keeps_proof_until_spawned_child_is_reaped() {
    let fixture = Fixture::new(
        "printf '%s\n%s\n' \"$$\" \"$1\" > \"$MG_BROWSER_OUTPUT\"; exec /bin/sleep 30",
    );
    let observed = Arc::new(Observations::default());
    let (guard, gate) = gated(&observed, FIRST_CHILD_CHECK);
    let weak = Arc::downgrade(&guard);
    let mut operation = launch(&fixture, guard);
    assert!(poll(&mut operation).is_pending());
    gate.wait();
    assert_eq!(observed.checks.load(Ordering::Acquire), FIRST_CHILD_CHECK);
    let expected = format!("\n{}\n", url().as_str());
    let output = fixture.directory.join("output");
    until(|| std::fs::read_to_string(&output).is_ok_and(|text| text.ends_with(&expected)));
    let text = std::fs::read_to_string(output).unwrap();
    let child: i32 = text.strip_suffix(&expected).unwrap().parse().unwrap();
    observed.child.store(child, Ordering::Release);
    let child = rustix::process::Pid::from_raw(child).unwrap();
    assert!(observe_child(child).unwrap().is_none());
    drop(operation);
    // The first observation is still gated with a real unreaped child. Neither
    // dropping the public future nor losing its last caller owns this cleanup.
    assert!(weak.upgrade().is_some());
    assert_eq!(observed.drops.load(Ordering::Acquire), 0);
    assert!(observe_child(child).unwrap().is_none());
    drop(gate);
    fixture.settle();
    assert!(weak.upgrade().is_none());
    assert_eq!(observed.drops.load(Ordering::Acquire), 1);
    assert!(observed.child_reaped_at_drop.load(Ordering::Acquire));
    assert!(matches!(
        observe_child(child),
        Err(rustix::io::Errno::CHILD)
    ));
}
