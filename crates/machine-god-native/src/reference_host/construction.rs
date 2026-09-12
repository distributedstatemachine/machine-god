//! Failure/unwind cleanup on the synchronous native constructor's caller worker.

use super::NativeTerminalHostResource;
use crate::NativeOwnedWorkerCompletion;

/// Declare before acquiring the terminal resource. Later locals must release
/// their actual owners before this observation joins the new scope on failure.
/// A successful constructor explicitly transfers ownership to its returned host.
#[derive(Default)]
pub(super) struct Construction(Option<NativeOwnedWorkerCompletion>);

impl Construction {
    pub(super) fn observe(&mut self, resource: Option<&NativeTerminalHostResource>) {
        self.0 = resource.map(NativeTerminalHostResource::completion);
    }

    pub(super) fn transfer(&mut self) {
        self.0 = None;
    }
}

impl Drop for Construction {
    fn drop(&mut self) {
        if let Some(completion) = self.0.take() {
            // The scope is newly created inside this constructor, not the
            // caller's worker scope. Its owner is dropped before this guard,
            // so neither a self-wait nor a still-open-scope wait is possible.
            completion
                .wait_on_worker()
                .expect("native construction never runs inside its newly created scope");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeOwnedWorkerScope;
    use std::sync::mpsc::{SyncSender, sync_channel};

    struct Resource {
        scope: NativeOwnedWorkerScope,
        release: Option<SyncSender<()>>,
    }
    impl Resource {
        fn new() -> Self {
            let scope = NativeOwnedWorkerScope::new();
            let (release, receive) = sync_channel(1);
            scope.spawn(move || receive.recv().unwrap()).unwrap();
            Self {
                scope,
                release: Some(release),
            }
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            self.scope.close();
            self.release.take().unwrap().send(()).unwrap();
        }
    }

    #[test]
    fn failed_and_unwound_construction_joins_after_releasing_actual_owner() {
        for unwind in [false, true] {
            let mut observed = None;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut construction = Construction::default();
                let resource = Resource::new();
                observed = Some(resource.scope.completion());
                construction.0 = observed.clone();
                assert!(!observed.as_ref().unwrap().is_complete());
                assert!(!unwind, "synthetic constructor unwind");
                Err::<(), ()>(())
            }));
            if unwind {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), Err(()));
            }
            assert!(observed.unwrap().is_complete());
        }
    }

    #[test]
    fn successful_transfer_keeps_the_returned_owner_live() {
        let resource = {
            let mut construction = Construction::default();
            let resource = Resource::new();
            construction.0 = Some(resource.scope.completion());
            construction.transfer();
            resource
        };
        let completion = resource.scope.completion();
        assert!(!completion.is_complete());
        drop(resource);
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
    }
}
