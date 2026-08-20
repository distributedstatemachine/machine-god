#![doc = "Provider-neutral contracts and orchestration for machine-god."]

/// Current public API version.
pub const API_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::API_VERSION;

    #[test]
    fn api_version_starts_at_one() {
        assert_eq!(API_VERSION, 1);
    }
}
