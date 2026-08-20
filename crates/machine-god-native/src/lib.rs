#![doc = "Explicit native capabilities for machine-god hosts."]

/// Returns the core API version supported by this native host.
#[must_use]
pub const fn supported_core_api_version() -> u32 {
    machine_god_core::API_VERSION
}
