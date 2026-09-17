use super::*;
use crate::managed::notices::{
    NoticeLimits, NoticePrincipal, NoticeRelationship, WorkNoticeIdentity,
};
use machine_god_core::{BoxFuture, ManagedNotifications};
use std::{future::Future, num::NonZeroU64, pin::Pin, sync::Mutex, task::Wake, time::Duration};

struct Clock {
    now: Mutex<Instant>,
    timer: Mutex<Option<(Instant, Waker)>>,
}
impl Clock {
    fn advance(&self, milliseconds: u64) {
        let now = {
            let mut now = self.now.lock().unwrap();
            *now += Duration::from_millis(milliseconds);
            *now
        };
        let wake = self
            .timer
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(deadline, _)| *deadline <= now)
            .map(|(_, wake)| wake.clone());
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
struct Sleep<'a>(&'a Clock, Instant);
impl Future for Sleep<'_> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0.now() >= self.1 {
            return Poll::Ready(());
        }
        *self.0.timer.lock().unwrap() = Some((self.1, cx.waker().clone()));
        Poll::Pending
    }
}
impl Drop for Sleep<'_> {
    fn drop(&mut self) {
        self.0.timer.lock().unwrap().take();
    }
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(Sleep(self, deadline))
    }
}
#[derive(Default)]
struct WakeCount(std::sync::atomic::AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn suppressed_duration_rearms_later_child_without_unrelated_activity() {
    let mut fixture = Fixture::new(vec![]);
    let start = Instant::now();
    let clock = Arc::new(Clock {
        now: Mutex::new(start),
        timer: Mutex::new(None),
    });
    fixture.manager.clock = clock.clone();
    fixture.manager.notices =
        Arc::new(ManagedNotices::new(NoticeLimits::default(), clock.clone()).unwrap());
    for name in ["duration", "interval"] {
        assert!(
            fixture
                .command(serde_json::json!({ "create": {"name": name, "mode": "persistent"} }))
                .ok
        );
    }
    fixture.drive(|f| f.manager.active.is_none());
    let nz = |n| NonZeroU64::new(n).unwrap();
    for (index, (interval, duration)) in [(10, Some(5)), (20, None)].into_iter().enumerate() {
        let child = &mut fixture.manager.children[index];
        let work = fixture
            .manager
            .notices
            .register_work(
                &WorkNoticeIdentity {
                    source: NoticePrincipal {
                        id: child.snapshot.head.id.clone(),
                        generation: nz(1),
                    },
                    work_id: format!("work-{index}"),
                    work_generation: nz(1),
                },
                ManagedNotifications {
                    report_interval_ms: Some(interval),
                    report_duration_ms: duration,
                    ..ManagedNotifications::default()
                },
                &NoticeRelationship {
                    generation: nz(1),
                    parent: Some(NoticePrincipal {
                        id: "parent".into(),
                        generation: nz(1),
                    }),
                },
                0,
            )
            .unwrap();
        assert!(matches!(
            fixture
                .manager
                .notices
                .prepare_start(&work, nz(1), None)
                .unwrap(),
            crate::managed::notices::PreparedNotice::Suppressed
        ));
        child.notice = Some(work);
    }
    let wake = Arc::new(WakeCount::default());
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(!fixture.manager.pump_notices(&mut cx).unwrap());
    clock.advance(5);
    assert!(!fixture.manager.pump_notices(&mut cx).unwrap());
    assert_eq!(
        clock
            .timer
            .lock()
            .unwrap()
            .as_ref()
            .map(|(deadline, _)| *deadline),
        Some(start + Duration::from_millis(20))
    );
    wake.0.store(0, Ordering::Relaxed);
    clock.advance(15);
    assert!(wake.0.load(Ordering::Relaxed) > 0);
    assert!(fixture.manager.pump_notices(&mut cx).unwrap());
    assert!(fixture.manager.children[0].pending.is_empty());
    assert_eq!(fixture.manager.children[1].pending.len(), 1);
}
