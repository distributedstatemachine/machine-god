//! Non-owning evidence of one actual inbox registration, including its epoch.
use super::{
    NativeInteractivePromptBridge, NativeInteractivePromptError, NativeInteractivePromptInbox,
    NativeInteractivePromptPrincipal, PrincipalKey, Shared,
};
use crate::mcp::interaction::McpElicitationPromptRequest;
use machine_god_core::BackgroundOutputOwner;
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

/// Inert, bounded custody for a future registration. It exposes no bridge and
/// cannot displace a live owner. Failed activation returns the original charge.
pub(crate) struct NativeInteractivePromptReservation {
    shared: Arc<Shared>,
    key: Option<PrincipalKey>,
}
impl NativeInteractivePromptReservation {
    pub(super) fn new(shared: Arc<Shared>, key: PrincipalKey) -> Self {
        Self {
            shared,
            key: Some(key),
        }
    }

    pub(crate) fn activate(
        mut self,
    ) -> Result<NativeInteractivePromptPrincipal, (NativeInteractivePromptError, Self)> {
        if let Err(error) = self
            .shared
            .activate_reserved(self.key.as_ref().expect("reserved registration"))
        {
            return Err((error, self));
        }
        Ok(NativeInteractivePromptPrincipal {
            shared: self.shared.clone(),
            key: self.key.take().expect("activated registration"),
        })
    }
}
impl Drop for NativeInteractivePromptReservation {
    fn drop(&mut self) {
        if let Some(key) = &self.key {
            self.shared.release_reserved(key);
        }
    }
}
impl fmt::Debug for NativeInteractivePromptReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptReservation { .. }")
    }
}

#[derive(Clone)]
pub(crate) struct NativeInteractivePromptRegistration {
    live: Weak<AtomicBool>,
    owner: BackgroundOutputOwner,
}
impl NativeInteractivePromptRegistration {
    pub(crate) fn owner(&self) -> &BackgroundOutputOwner {
        &self.owner
    }
    pub(crate) fn is_live(&self) -> bool {
        // Only metadata is upgraded; this check cannot acquire inbox locks or
        // drop the last inbox/runtime owner while a caller holds its own lock.
        self.live
            .upgrade()
            .is_some_and(|live| live.load(Ordering::Acquire))
    }
}
impl From<PrincipalKey> for NativeInteractivePromptRegistration {
    fn from(key: PrincipalKey) -> Self {
        Self {
            live: Arc::downgrade(&key.live),
            owner: key.owner,
        }
    }
}
impl fmt::Debug for NativeInteractivePromptRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptRegistration { .. }")
    }
}
impl NativeInteractivePromptBridge {
    pub(crate) fn elicitation_registration(
        &self,
        request: &McpElicitationPromptRequest,
    ) -> Option<NativeInteractivePromptRegistration> {
        let key = self.shared.capture(
            self.principal.as_ref(),
            &super::payload::Payload::Elicitation {
                request: request.clone(),
            },
        )?;
        Some(key.into())
    }
}
impl NativeInteractivePromptInbox {
    pub(crate) fn registration_for_owner(
        &self,
        owner: &BackgroundOutputOwner,
    ) -> Option<NativeInteractivePromptRegistration> {
        let key = self.shared.capture_owner(owner)?;
        Some(key.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeInteractivePromptLimits;
    use machine_god_core::{SessionId, SessionIncarnationId};
    fn owner() -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        )
    }

    #[test]
    fn witness_does_not_pin_registration_or_inbox_and_never_rebinds_reused_labels() {
        let mut inbox =
            NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
        let weak = Arc::downgrade(&inbox.shared);
        let principal = inbox.register(owner()).unwrap();
        let original = inbox.registration_for_owner(&owner()).unwrap();
        assert!(original.is_live());
        drop(principal);
        assert!(!original.is_live());
        let replacement = inbox.register(owner()).unwrap();
        let current = inbox.registration_for_owner(&owner()).unwrap();
        assert!(!original.is_live());
        assert!(current.is_live());
        inbox.close();
        assert!(!current.is_live());
        drop(replacement);
        drop(inbox);
        assert!(weak.upgrade().is_none());
        assert!(!original.is_live());
        assert!(!current.is_live());
    }

    #[test]
    fn observing_liveness_does_not_acquire_the_inbox_mutex() {
        let mut inbox =
            NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
        let _principal = inbox.register(owner()).unwrap();
        let witness = inbox.registration_for_owner(&owner()).unwrap();
        let guard = inbox.shared.state.lock().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let observer = std::thread::spawn(move || sent.send(witness.is_live()));
        let result = received.recv_timeout(std::time::Duration::from_secs(2));
        drop(guard);
        observer.join().unwrap().unwrap();
        assert_eq!(result, Ok(true));
    }
}
