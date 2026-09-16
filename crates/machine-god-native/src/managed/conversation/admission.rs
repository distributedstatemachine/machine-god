//! Attribute readiness/checkpoint I/O before a core turn exists.

use super::{ManagedConversationBinding, NativeConversationError, NativeOwnedWorkerRun, Result};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub(crate) struct ManagedAdmission {
    cohort: Option<Arc<NativeOwnedWorkerRun>>,
    transferred: bool,
}

impl ManagedConversationBinding {
    pub(crate) fn prepare_admission(&self) -> Result<ManagedAdmission> {
        self.validate()?;
        let owner = self
            .0
            .upgrade()
            .ok_or(NativeConversationError::ManagedAdmission)?;
        let mut active = owner
            .active
            .lock()
            .map_err(|_| NativeConversationError::ManagedAdmission)?;
        if active
            .admission
            .as_ref()
            .is_some_and(|cohort| !cohort.completion().is_complete())
        {
            return Err(NativeConversationError::ManagedAdmission);
        }
        let cohort = owner
            .workers
            .get()
            .map(|workers| {
                workers
                    .scope
                    .begin_run_with_keepalive(workers.keepalive.clone())
                    .map(Arc::new)
                    .map_err(|_| NativeConversationError::ManagedAdmission)
            })
            .transpose()?;
        active.admission.clone_from(&cohort);
        active.preparation_pending = true;
        Ok(ManagedAdmission {
            cohort,
            transferred: false,
        })
    }

    /// Actual most-recent admission, including one which never minted a `RunRef`.
    /// This is completion metadata only and cannot keep the manager alive.
    pub(crate) fn admission_completion(&self) -> Option<crate::NativeOwnedWorkerCompletion> {
        self.0
            .upgrade()?
            .active
            .lock()
            .ok()?
            .admission
            .as_ref()
            .map(|run| run.completion())
    }

    /// Called only after the runtime's admission or actual turn has ended. An idle
    /// foreground is polled repeatedly, including during unrelated MCP controls;
    /// consume preparation custody once, even if it has no outstanding workers.
    /// Idle polling or an earlier turn's cleanup cannot authorize cancelling a
    /// later unrelated controller job.
    pub(crate) fn take_preparation_settlement(&self) -> bool {
        let Some(owner) = self.0.upgrade() else {
            return false;
        };
        let Ok(mut active) = owner.active.lock() else {
            return false;
        };
        std::mem::take(&mut active.preparation_pending)
    }
}

impl ManagedAdmission {
    pub(crate) fn cohort(&self) -> Option<Arc<NativeOwnedWorkerRun>> {
        self.cohort.clone()
    }

    pub(crate) fn transfer(&mut self) {
        self.transferred = true;
    }

    pub(crate) fn wrap<F: Future>(&self, future: F) -> AttributedAdmission<F> {
        AttributedAdmission {
            inner: Some(Box::pin(future)),
            cohort: self.cohort.clone(),
        }
    }
}

impl Drop for ManagedAdmission {
    fn drop(&mut self) {
        if !self.transferred
            && let Some(cohort) = &self.cohort
        {
            cohort.close();
        }
    }
}

pub(crate) struct AttributedAdmission<F: Future> {
    inner: Option<Pin<Box<F>>>,
    cohort: Option<Arc<NativeOwnedWorkerRun>>,
}

impl<F: Future> Future for AttributedAdmission<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let cohort = self.cohort.clone();
        let future = self
            .inner
            .as_mut()
            .expect("admission future retained until drop");
        match cohort {
            Some(cohort) => cohort.with_poll(|| future.as_mut().poll(cx)),
            None => future.as_mut().poll(cx),
        }
    }
}

impl<F: Future> Drop for AttributedAdmission<F> {
    fn drop(&mut self) {
        match &self.cohort {
            Some(cohort) => cohort.with_poll(|| drop(self.inner.take())),
            None => drop(self.inner.take()),
        }
    }
}
