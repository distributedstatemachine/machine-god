use super::{Instant, McpPeerError, McpStdioPeer, Result, RpcEnvelope, RpcId, routing};
use crate::mcp::catalog_refresh::{McpSubscriptionFilters, validate_subscription_response};
use crate::mcp::stdio::McpStdioControl;

const MAX_RETIRED: usize = 64;

#[derive(Default)]
pub(super) struct State {
    active: Option<i64>,
    retired: Vec<i64>,
    failure: Option<McpPeerError>,
}

impl State {
    pub(super) fn consume(&mut self, envelope: &RpcEnvelope) -> bool {
        let Some(RpcId::Integer(id)) = envelope.id() else {
            return false;
        };
        if let Some(index) = self.retired.iter().position(|retired| retired == id) {
            self.retired.swap_remove(index);
            return true;
        }
        if self.active != Some(*id) {
            return false;
        }
        self.active = None;
        if validate_subscription_response(envelope, &RpcId::Integer(*id)).is_err() {
            self.failure = Some(McpPeerError::InvalidResult);
        }
        true
    }

    pub(super) fn take_failure(&mut self) -> Option<McpPeerError> {
        self.failure.take()
    }
}

impl McpStdioPeer {
    /// Sends one typed listen request. The caller must validate its queued ACK
    /// with the catalog refresh policy before considering the listener ready.
    /// # Errors
    /// Rejects empty filters, a busy peer/listener, or a failed bounded write.
    pub async fn start_subscription(
        &mut self,
        filters: &McpSubscriptionFilters,
        deadline: Instant,
    ) -> Result<RpcId> {
        self.check_available()?;
        if self.subscription.active.is_some() {
            return Err(McpPeerError::Capacity);
        }
        if filters.is_empty() {
            return Err(McpPeerError::InvalidResult);
        }
        let id = self.allocate()?;
        let control = McpStdioControl::subscription(&id, filters, self.protocol.version)?;
        let RpcId::Integer(value) = id else {
            unreachable!("allocated integer ID")
        };
        self.subscription.active = Some(value);
        self.subscription.failure = None;
        self.send_subscription(control, deadline).await?;
        Ok(id)
    }

    /// The exact active listen ID; an idle observation timeout does not clear it.
    #[must_use]
    pub fn active_subscription(&self) -> Option<RpcId> {
        if self.closed {
            None
        } else {
            self.subscription.active.map(RpcId::Integer)
        }
    }

    /// Observes one queued notification or the listener ending. Dropping this
    /// read retains partial frames and pending replies on the healthy peer.
    /// # Errors
    /// Reports listener terminal failure, owner expiry, and transport failures.
    pub async fn poll_subscription(&mut self, deadline: Instant) -> Result<Option<RpcEnvelope>> {
        routing::poll_subscription(self, deadline).await
    }

    /// Cancels the exact listen ID without requiring a server final response.
    /// # Errors
    /// Rejects exhausted retired-ID storage or a failed/abandoned bounded write.
    pub async fn close_subscription(&mut self, deadline: Instant) -> Result<()> {
        self.check_available()?;
        let Some(id) = self.subscription.active else {
            return Ok(());
        };
        if self.subscription.retired.len() >= MAX_RETIRED {
            return Err(McpPeerError::Capacity);
        }
        let control = McpStdioControl::cancel_subscription(&RpcId::Integer(id))?;
        self.subscription.active = None;
        self.subscription.retired.push(id);
        self.send_subscription(control, deadline).await
    }

    async fn send_subscription(
        &mut self,
        control: McpStdioControl,
        deadline: Instant,
    ) -> Result<()> {
        let deadline = self.lifetime.constrain(deadline);
        self.closed = true;
        routing::send(
            routing::Exchange {
                connection: &self.connection,
                notifications: &mut self.notifications,
                notification_bytes: &mut self.notification_bytes,
                pending_replies: &mut self.pending_replies,
                subscription: &mut self.subscription,
                timer: &self.timer,
                cancellation: &self.cancellation,
            },
            self.connection.control(control, deadline),
            deadline,
        )
        .await?;
        self.closed = false;
        Ok(())
    }
}
