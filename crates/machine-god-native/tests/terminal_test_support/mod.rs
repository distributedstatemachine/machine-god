#![allow(dead_code)]

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    SessionId, SessionIncarnationId, ToolCall, ToolCallId, ToolContext, ToolName, TurnId,
};
use serde_json::Value;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    pub fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-terminal-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a terminal test directory: {error}"),
            }
        }
        panic!("failed to allocate a unique terminal test directory");
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a terminal test directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

pub fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

pub fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("terminal execution unexpectedly remained pending"),
    }
}

pub fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("terminal-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

pub fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("terminal-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("terminal-incarnation").unwrap(),
        turn_id: TurnId::new("terminal-turn").unwrap(),
        call_id: ToolCallId::new("terminal-call").unwrap(),
    }
}
