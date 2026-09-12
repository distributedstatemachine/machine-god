use super::{
    Arc, AtomicBool, McpCatalogKind, McpCatalogRefresh, McpDescriptorCatalog, McpRefreshDecision,
    McpRefreshError, McpRefreshGeneration, McpRefreshTicket, Ordering, Result, index, times,
};

impl McpCatalogRefresh {
    /// Decides one requested family using milliseconds from the catalog's
    /// original monotonic epoch. It performs no refresh or publication itself.
    /// # Errors
    /// Rejects foreign generations, closed policy state and clock regression.
    pub fn begin(
        &mut self,
        generation: &McpRefreshGeneration,
        kind: McpCatalogKind,
        now_ms: u64,
    ) -> Result<McpRefreshDecision> {
        self.check(generation)?;
        self.observe_time(now_ms)?;
        let family = &mut self.families[index(kind)];
        let may_serve_snapshot = family.times.is_some();
        if family
            .active
            .as_ref()
            .is_some_and(|active| active.load(Ordering::Acquire))
        {
            return Ok(McpRefreshDecision::AlreadyRefreshing { may_serve_snapshot });
        }
        // A dropped ticket permits a later caller-driven attempt, without
        // invoking a callback or retaining an abandoned permanent busy state.
        family.active = None;
        if family.retry_at.is_some_and(|retry| now_ms < retry) {
            return Ok(McpRefreshDecision::RetryLater { may_serve_snapshot });
        }
        if family.retry_at.is_none()
            && family.invalidation == family.handled
            && family.times.is_some_and(|times| now_ms < times.expires)
        {
            return Ok(McpRefreshDecision::Hit);
        }
        let active = Arc::new(AtomicBool::new(true));
        family.active = Some(active.clone());
        Ok(McpRefreshDecision::Refresh(McpRefreshTicket {
            generation: self.generation.clone(),
            kind,
            active,
            invalidation: family.invalidation,
            may_serve_snapshot,
        }))
    }

    /// Call only after the owner atomically publishes the admitted replacement
    /// against the original runtime checkpoint, while serializing this policy.
    /// # Errors
    /// Rejects foreign tickets, closed state, clock regression, mismatched
    /// families and replacement timestamps outside the original time partition.
    pub fn finish(
        &mut self,
        ticket: McpRefreshTicket,
        catalog: &McpDescriptorCatalog,
        now_ms: u64,
    ) -> Result<()> {
        if let Err(error) = self.validate_replacement(&ticket, catalog, now_ms) {
            if error == McpRefreshError::ClockRegression {
                self.closed = true;
            }
            return Err(error);
        }
        self.last_now = now_ms;
        let family = &mut self.families[index(ticket.kind)];
        family.times = Some(times(catalog));
        family.handled = ticket.invalidation;
        family.active = None;
        family.attempt = 0;
        family.retry_at = None;
        drop(ticket);
        Ok(())
    }

    /// Checks the same preconditions as `finish` without changing policy state.
    /// The owner must keep policy access serialized through conditional runtime
    /// publication and `finish` using these same inputs; validation grants no
    /// publication authority and is not a reservation against later changes.
    /// # Errors
    /// Rejects foreign tickets, closed state, clock regression, wrong families
    /// and invalid replacement timestamps. Errors have no policy side effects.
    pub fn validate_replacement(
        &self,
        ticket: &McpRefreshTicket,
        catalog: &McpDescriptorCatalog,
        now_ms: u64,
    ) -> Result<()> {
        self.check_ticket(ticket)?;
        if now_ms < self.last_now {
            return Err(McpRefreshError::ClockRegression);
        }
        if catalog.kind() != ticket.kind
            || catalog.fetched_at_ms() > now_ms
            || self.families[index(ticket.kind)]
                .times
                .is_some_and(|previous| catalog.fetched_at_ms() < previous.fetched)
        {
            return Err(McpRefreshError::Invalid);
        }
        Ok(())
    }

    /// Preserves existing data and schedules bounded, caller-driven backoff.
    /// # Errors
    /// Rejects foreign tickets, closed state, clock regression and retry overflow.
    pub fn fail(&mut self, ticket: McpRefreshTicket, now_ms: u64) -> Result<()> {
        self.check_ticket(&ticket)?;
        self.observe_time(now_ms)?;
        let family = &mut self.families[index(ticket.kind)];
        let delay = (100u64 << family.attempt.min(8)).min(5_000);
        let Some(retry_at) = now_ms.checked_add(delay) else {
            self.closed = true;
            return Err(McpRefreshError::Exhausted);
        };
        family.retry_at = Some(retry_at);
        family.attempt = family.attempt.saturating_add(1).min(8);
        family.active = None;
        drop(ticket);
        Ok(())
    }

    fn check_ticket(&self, ticket: &McpRefreshTicket) -> Result<()> {
        self.check(&ticket.generation)?;
        if !self.families[index(ticket.kind)]
            .active
            .as_ref()
            .is_some_and(|active| {
                Arc::ptr_eq(active, &ticket.active) && active.load(Ordering::Acquire)
            })
        {
            return Err(McpRefreshError::Foreign);
        }
        Ok(())
    }

    pub(super) fn invalidate(&mut self, families: &[McpCatalogKind], reads: bool) -> Result<()> {
        if families
            .iter()
            .any(|kind| self.families[index(*kind)].invalidation == u64::MAX)
            || (reads && self.resource_reads == u64::MAX)
        {
            self.closed = true;
            return Err(McpRefreshError::Exhausted);
        }
        for kind in families {
            self.families[index(*kind)].invalidation += 1;
        }
        if reads {
            self.resource_reads += 1;
        }
        Ok(())
    }
}
