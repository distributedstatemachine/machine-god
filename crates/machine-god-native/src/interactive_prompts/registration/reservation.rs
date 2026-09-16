//! Staged host registrations; portable prompt bridges need no runtime staging.
use super::super::{
    NativeInteractivePromptError, NativeInteractivePromptPrincipal, PrincipalKey, Shared,
};
use std::{fmt, sync::Arc};

/// Inert, bounded custody for a future registration. It exposes no bridge and
/// cannot displace a live owner. Failed activation returns the original charge.
pub(crate) struct NativeInteractivePromptReservation {
    shared: Arc<Shared>,
    key: Option<PrincipalKey>,
}
impl NativeInteractivePromptReservation {
    pub(in crate::interactive_prompts) fn new(shared: Arc<Shared>, key: PrincipalKey) -> Self {
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
