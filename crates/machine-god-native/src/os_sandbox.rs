//! Explicit immutable macOS Seatbelt launch authority.

mod launch;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use launch::NATIVE_TESTS;
pub use launch::*;
#[cfg(test)]
use launch::{build_profile, quote};

#[cfg(test)]
mod tests;
