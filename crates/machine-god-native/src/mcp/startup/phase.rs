/// Exact pinned startup selection. Deferred Ask peers are an additive batch,
/// never a replacement for already active required peers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpStartupPhase {
    All,
    AskStartup,
    AskDeferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Selection {
    Connect,
    Disabled,
    Deferred,
}

impl NativeMcpStartupPhase {
    pub(super) const fn select(self, enabled: bool, required: bool) -> Selection {
        if !enabled {
            return Selection::Disabled;
        }
        match self {
            Self::All => Selection::Connect,
            Self::AskStartup if required => Selection::Connect,
            Self::AskDeferred if !required => Selection::Connect,
            Self::AskStartup | Self::AskDeferred => Selection::Deferred,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeMcpStartupPhase as Phase, Selection};

    #[test]
    fn pinned_required_optional_and_disabled_phase_selection() {
        for phase in [Phase::All, Phase::AskStartup, Phase::AskDeferred] {
            for required in [false, true] {
                assert_eq!(phase.select(false, required), Selection::Disabled);
                let expected = match (phase, required) {
                    (Phase::AskStartup, false) | (Phase::AskDeferred, true) => Selection::Deferred,
                    _ => Selection::Connect,
                };
                assert_eq!(phase.select(true, required), expected);
            }
        }
    }
}
