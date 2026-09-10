//! Internal composition seam; production retains the concrete generation-bound requester.

use crate::{
    NativeTerminalBackgroundError, NativeTerminalBackgroundInspection,
    NativeTerminalBackgroundPage, NativeTerminalBackgroundRequester,
    NativeTerminalBackgroundSnapshot, NativeTerminalBackgroundStopReceipt,
    NativeTerminalBackgroundTarget,
};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, TerminalCursor, TerminalSessionId,
};

type Reply<T> = BoxFuture<'static, Result<T, NativeTerminalBackgroundError>>;

pub(crate) trait BackgroundRequests: Send + Sync + 'static {
    fn snapshot(
        &self,
        owner: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundSnapshot>;
    fn select(
        &self,
        owner: BackgroundOutputOwner,
        id: Option<TerminalSessionId>,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundTarget>;
    fn inspect(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundInspection>;
    fn read(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cursor: TerminalCursor,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundPage>;
    fn tail_start(
        &self,
        target: &NativeTerminalBackgroundTarget,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> Reply<TerminalCursor>;
    fn stop(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundStopReceipt>;
}

impl BackgroundRequests for NativeTerminalBackgroundRequester {
    fn snapshot(
        &self,
        owner: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundSnapshot> {
        Self::snapshot(self, owner, cancellation)
    }
    fn select(
        &self,
        owner: BackgroundOutputOwner,
        id: Option<TerminalSessionId>,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundTarget> {
        Self::select(self, owner, id, cancellation)
    }
    fn inspect(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundInspection> {
        Self::inspect(self, target, cancellation)
    }
    fn read(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cursor: TerminalCursor,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundPage> {
        Self::read(self, target, cursor, maximum, cancellation)
    }
    fn tail_start(
        &self,
        target: &NativeTerminalBackgroundTarget,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> Reply<TerminalCursor> {
        Self::tail_start(self, target, maximum, cancellation)
    }
    fn stop(
        &self,
        target: &NativeTerminalBackgroundTarget,
        cancellation: CancellationToken,
    ) -> Reply<NativeTerminalBackgroundStopReceipt> {
        Self::stop(self, target, cancellation)
    }
}
