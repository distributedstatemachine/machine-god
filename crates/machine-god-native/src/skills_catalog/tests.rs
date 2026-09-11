#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "tests/supported.rs"]
mod supported;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "tests/prefix.rs"]
mod prefix;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_catalog_construction_is_inert() {
    assert_eq!(
        super::NativeSkillCatalog::new(Vec::new()).unwrap_err(),
        super::NativeSkillCatalogError::UnsupportedPlatform
    );
}
