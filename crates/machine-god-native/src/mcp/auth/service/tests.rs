use super::*;
use crate::mcp::{
    config::{McpServerConfig, McpTransportConfig},
    http::McpHttpClock,
};
use state::lock;
use std::{
    path::PathBuf,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

mod workers;

type Hook = Mutex<Option<Arc<dyn Fn() + Send + Sync>>>;
#[derive(Default)]
pub(super) struct Hooks {
    before_load: Hook,
    before_commit: Hook,
    admitted_commit: Hook,
    after_commit: Hook,
    before_remove: Hook,
    after_remove: Hook,
}
macro_rules! hook_methods { ($($name:ident),+) => {$(
    pub fn $name(&self) {
        let hook = lock(&self.$name).take();
        if let Some(hook) = hook { hook(); }
    }
)+}; }
impl Hooks {
    hook_methods!(
        before_load,
        before_commit,
        admitted_commit,
        after_commit,
        before_remove,
        after_remove
    );
}

struct Gate {
    entered: CancellationToken,
    released: Mutex<bool>,
    wake: Condvar,
}
struct Pause(Arc<Gate>);
impl Pause {
    fn install(hook: &Hook) -> Self {
        let gate = Arc::new(Gate {
            entered: CancellationToken::new(),
            released: Mutex::new(false),
            wake: Condvar::new(),
        });
        let worker = gate.clone();
        *lock(hook) = Some(Arc::new(move || {
            worker.entered.cancel();
            let (released, _) = worker
                .wake
                .wait_timeout_while(lock(&worker.released), Duration::from_secs(10), |value| {
                    !*value
                })
                .unwrap();
            assert!(*released, "persistence barrier must be released");
        }));
        Self(gate)
    }
    async fn entered(&self) {
        self.0.entered.cancelled().await;
    }
    fn release(&self) {
        *lock(&self.0.released) = true;
        self.0.wake.notify_all();
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        self.release();
    }
}

struct Clock;
impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
impl McpAuthClock for Clock {
    fn unix_millis(&self) -> i64 {
        1_000_000
    }
}
struct NoNetwork;
impl McpAuthNetwork for NoNetwork {
    fn admit<'a>(
        &'a self,
        _: &'a str,
        _: &'a CancellationToken,
        _: Instant,
    ) -> BoxFuture<'a, Result<super::super::McpAuthDestination>> {
        Box::pin(async { panic!("persistence fixture never performs network effects") })
    }
}
struct NoEntropy;
impl McpAuthEntropy for NoEntropy {
    fn fill(&self, _: &mut [u8]) -> Result<()> {
        panic!("persistence fixture never performs authorization")
    }
}
struct Events;
impl McpAuthInvalidation for Events {
    fn invalidate(&self, _: super::super::McpAuthInvalidated) {}
}

struct Fixture {
    directory: PathBuf,
    workers: NativeOwnedWorkerScope,
    service: NativeMcpAuthService,
    config: McpAuthConfig,
}
impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mg-auth-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let directory = directory.canonicalize().unwrap();
        let workers = NativeOwnedWorkerScope::new();
        let service = NativeMcpAuthService::new(
            Arc::new(NativeMcpCredentialStore::new(directory.join("profile")).unwrap()),
            Arc::new(NoNetwork),
            Arc::new(Clock),
            Arc::new(NoEntropy),
            Arc::new(Events),
            workers.clone(),
        );
        Self {
            directory,
            workers,
            service,
            config: config("http://127.0.0.1:34567/mcp"),
        }
    }
    fn credentials(&self, access: &[u8]) -> Credentials {
        use super::super::codec::{Registration, Secret};
        Credentials {
            identity: self.config.identity().clone(),
            resource: "http://127.0.0.1:34567/mcp".into(),
            issuer: "http://127.0.0.1:34567".into(),
            registration: Registration {
                id: Secret::new(b"client").unwrap(),
                secret: None,
                method: "none".into(),
            },
            access: Secret::new(access).unwrap(),
            refresh: None,
            scope: "read".into(),
            expires_ms: i64::MAX,
            authorization_endpoint: "http://127.0.0.1:34567/authorize".into(),
            token_endpoint: "http://127.0.0.1:34567/token".into(),
            revocation_endpoint: None,
        }
    }
    fn seed(&self) {
        let store = &self.service.inner.store;
        store
            .publish(
                &store.load().unwrap(),
                self.config.identity(),
                Some(&self.credentials(b"old")),
            )
            .unwrap();
    }
    async fn commit(&self, cancellation: &CancellationToken) -> Result<McpAuthLease> {
        let operation =
            self.service
                .inner
                .begin(self.config.identity(), cancellation, deadline())?;
        let snapshot = operation.load(cancellation, deadline()).await?;
        operation
            .commit(
                snapshot,
                self.credentials(b"replacement"),
                cancellation,
                deadline(),
            )
            .await
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.close();
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn config(url: &str) -> McpAuthConfig {
    let server = McpServerConfig::decode(
        "srv",
        serde_json::json!({"type":"http","url":url})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = server.transport() else {
        panic!("remote fixture")
    };
    McpAuthConfig::new(remote, b"selected", |_| None).unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn run(future: impl std::future::Future<Output = ()>) {
    crate::mcp::http::tests::executor().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .expect("owned persistence fixture settles");
    });
}
