//! Same-domain ACP replacement delegates custody to the native transition lane.
#[cfg(test)]
mod tests;
use super::{
    AcpSessionError, CancellationToken, Context, NativeAcpSession, NativeAcpSessionSelection, Poll,
};
use crate::interactive_session::NativeManagedStageFailure;
use crate::managed::manager::ManagedForegroundReservation;
use crate::reference_host::NativeManagedStagedParent;
use crate::{
    NativeInteractiveRequestReceipt, NativeInteractiveTransition, NativeManagedAgentsError,
    NativeResumeTarget,
};
use machine_god_core::BoxFuture;
#[cfg(feature = "mcp-http")]
use std::sync::Arc;

impl NativeAcpSession {
    pub(crate) fn reserve_parent_stage(
        &mut self,
    ) -> Result<ManagedForegroundReservation, AcpSessionError> {
        self.inner.reserve_parent_stage().map_err(Into::into)
    }
    pub(crate) fn poll_parent_stage_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
        cx: &Context<'_>,
    ) -> Poll<Result<(), AcpSessionError>> {
        self.inner
            .poll_parent_stage_reservation(reservation, cx)
            .map_err(Into::into)
    }
    pub(crate) fn start_parent_stage(
        &self,
        reservation: ManagedForegroundReservation,
        #[cfg(feature = "mcp-http")] network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
        configuration: crate::mcp::ephemeral::NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeManagedStagedParent, NativeManagedAgentsError>> {
        self.inner.start_parent_stage(
            reservation,
            #[cfg(feature = "mcp-http")]
            network,
            configuration,
            cancellation,
        )
    }
    pub(crate) fn request_staged_selection(
        &mut self,
        selection: NativeAcpSessionSelection,
        candidate: NativeManagedStagedParent,
        now_ms: i64,
        cancellation: CancellationToken,
    ) -> Result<NativeInteractiveRequestReceipt, NativeManagedStageFailure> {
        let kind = match selection {
            NativeAcpSessionSelection::New => NativeInteractiveTransition::New,
            NativeAcpSessionSelection::Load(id) | NativeAcpSessionSelection::Resume(id) => {
                NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(id))
            }
        };
        self.inner
            .request_staged_transition(kind, candidate, now_ms, cancellation)
    }
    pub(crate) fn refresh_selected_parent(&mut self, replay: bool) {
        self.command_services = super::super::commands::Services::from_session(&self.inner);
        self.history =
            replay.then(|| super::NativeAcpHistory::new(self.inner.runtime().record_snapshot()));
        self.cancelling = false;
    }
}
