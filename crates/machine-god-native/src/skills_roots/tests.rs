use super::*;

#[test]
fn cancelled_empty_composition_is_inert() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        compose_native_skill_catalog(None, None, None, &cancellation).unwrap_err(),
        NativeSkillRootsError::Cancelled
    );
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_composition_has_no_effects() {
    assert_eq!(
        compose_native_skill_catalog(None, None, None, &CancellationToken::new()).unwrap_err(),
        NativeSkillRootsError::UnsupportedPlatform
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "supported.rs"]
mod supported;
