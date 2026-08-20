#![doc = "Explicit native capabilities for machine-god hosts."]

/// Core API version intentionally supported by this native host.
pub const SUPPORTED_CORE_API_VERSION: u32 = 1;

/// Returns the core API version supported by this native host.
#[must_use]
pub const fn supported_core_api_version() -> u32 {
    SUPPORTED_CORE_API_VERSION
}

#[cfg(test)]
mod tests {
    use super::SUPPORTED_CORE_API_VERSION;

    #[test]
    fn compatibility_version_is_deliberately_current() {
        assert_eq!(SUPPORTED_CORE_API_VERSION, machine_god_core::API_VERSION);
    }
}
