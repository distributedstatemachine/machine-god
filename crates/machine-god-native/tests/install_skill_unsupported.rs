#![cfg(not(any(target_os = "linux", target_os = "macos")))]

use std::error::Error;
use std::path::Path;

use machine_god_native::{InstallSkillTool, InstallSkillToolOpenErrorKind};

#[test]
fn unsupported_constructor_is_exact_redacted_and_effect_free() {
    let private_root = "/PRIVATE_UNSUPPORTED_INSTALL_SKILL_ROOT_DO_NOT_REFLECT";
    let error = InstallSkillTool::open(Path::new(private_root))
        .expect_err("unsupported targets cannot construct install_skill");
    assert_eq!(
        error.kind(),
        InstallSkillToolOpenErrorKind::UnsupportedPlatform
    );
    assert_eq!(
        error.to_string(),
        "native install_skill is unsupported on this platform"
    );
    assert_eq!(
        format!("{error:?}"),
        "InstallSkillToolOpenError { kind: UnsupportedPlatform }"
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private_root));
    assert!(!format!("{error:?}").contains(private_root));
}
