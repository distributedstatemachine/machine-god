use super::*;
use crate::{
    FileSessionStore, NativeModelPreferences, NativeReasoningEffort, NativeResumeTarget,
    NativeSessionOrigin, SessionIdSource, SessionIdSourceError, SessionIncarnationSource,
    SessionIncarnationSourceError,
};
use machine_god_core::{Engine, SessionId, SessionStore};
use machine_god_testkit::{ScriptedModelProvider, ScriptedPermissionHandler};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::Duration,
};

struct FixedId;
impl SessionIncarnationSource for FixedId {
    fn next_incarnation_id(
        &self,
    ) -> Result<machine_god_core::SessionIncarnationId, SessionIncarnationSourceError> {
        Ok(machine_god_core::SessionIncarnationId::new("owned-incarnation").unwrap())
    }
}
impl SessionIdSource for FixedId {
    fn next_session_id(&self) -> Result<SessionId, SessionIdSourceError> {
        Ok(id())
    }
}
fn id() -> SessionId {
    SessionId::new("owned-preparation").unwrap()
}
struct Fixture {
    root: PathBuf,
    lifecycle: NativeSessionLifecycle,
    store: Arc<FileSessionStore>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "mg-owned-preparation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let store = Arc::new(FileSessionStore::open(&root).unwrap());
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("preparation", []))
            .shared_session_store(store.clone())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let lifecycle =
            NativeSessionLifecycle::with_identity_sources(engine, store.clone(), FixedId, FixedId)
                .unwrap();
        Self {
            root,
            lifecycle,
            store,
        }
    }
    fn prepare(&self, kind: NativeInteractiveTransition) -> Preparation {
        let options = NativeInteractiveSessionOptions::new(
            "/workspace".into(),
            NativeModelPreferences::new("fixture/model", NativeReasoningEffort::default(), false)
                .unwrap(),
        )
        .unwrap();
        Preparation::new(self.lifecycle.clone(), options, kind, 2)
    }
    fn seed(&self) {
        drop(
            futures_executor::block_on(
                self.lifecycle.create_generated_with_metadata(
                    NativeSessionMetadata::new(
                        std::path::Path::new("/workspace"),
                        1,
                        NativeSessionOrigin::Acp,
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn blocked(
    preparation: &mut Preparation,
) -> (mpsc::Receiver<std::thread::ThreadId>, mpsc::Sender<()>) {
    let (started, observe) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    preparation.before_io = Some(Box::new(move || {
        started.send(std::thread::current().id()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
    }));
    (observe, release)
}

#[test]
fn fresh_and_exact_prepare_never_poll_filesystem_work_on_the_calling_thread() {
    for resume in [false, true] {
        let fixture = Fixture::new();
        if resume {
            fixture.seed();
        }
        let kind = if resume {
            NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(id()))
        } else {
            NativeInteractiveTransition::New
        };
        let mut preparation = fixture.prepare(kind);
        let (started, release) = blocked(&mut preparation);
        let workers = NativeOwnedWorkerScope::new();
        let mut future = preparation.run(workers.clone());
        assert!(matches!(
            future.as_mut().poll(&mut Context::from_waker(
                futures_util::task::noop_waker_ref()
            )),
            Poll::Pending
        ));
        assert_ne!(
            started.recv_timeout(Duration::from_secs(5)).unwrap(),
            std::thread::current().id()
        );
        // A blocked pre-I/O phase did not block first poll or invent a completion.
        release.send(()).unwrap();
        let conversation = futures_executor::block_on(future).unwrap();
        assert_eq!(conversation.id(), id());
        drop(conversation);
        workers.close();
        workers.completion().wait_on_worker().unwrap();
    }
}

#[test]
fn dropped_accepted_preparation_retains_its_worker_and_persistence_until_settlement() {
    let fixture = Fixture::new();
    let mut preparation = fixture.prepare(NativeInteractiveTransition::New);
    let (started, release) = blocked(&mut preparation);
    let workers = NativeOwnedWorkerScope::new();
    let completion = workers.completion();
    let mut future = preparation.run(workers.clone());
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(
                futures_util::task::noop_waker_ref()
            ))
            .is_pending()
    );
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(future);
    workers.close();
    assert!(!completion.is_complete());
    release.send(()).unwrap();
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
    assert!(
        futures_executor::block_on(fixture.store.load(id()))
            .unwrap()
            .is_some()
    );
}

#[test]
fn unpolled_preparation_is_inert() {
    let fixture = Fixture::new();
    let mut preparation = fixture.prepare(NativeInteractiveTransition::New);
    preparation.before_io = Some(Box::new(|| panic!("unpolled work executed")));
    let workers = NativeOwnedWorkerScope::new();
    drop(preparation.run(workers.clone()));
    workers.close();
    assert!(workers.completion().is_complete());
    assert!(
        futures_executor::block_on(fixture.store.load(id()))
            .unwrap()
            .is_none()
    );
}
