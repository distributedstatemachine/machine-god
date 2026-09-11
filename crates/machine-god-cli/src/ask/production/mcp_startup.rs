//! Inert selection from the already captured native configuration directory.

use machine_god_native::mcp::{
    management::NativeMcpManagementService, store::NativeMcpConfigStore,
};
use std::{path::Path, sync::Arc};

/// Does not load configuration, resolve environment values or activate servers.
/// Missing profile selection means unavailable authority, not an ambient fallback.
pub(super) fn prepare(
    profile_directory: Option<&Path>,
) -> Result<Option<Arc<NativeMcpManagementService>>, ()> {
    profile_directory
        .map(|directory| {
            let store = NativeMcpConfigStore::new(directory.to_owned()).map_err(|_| ())?;
            Ok(Arc::new(NativeMcpManagementService::new(Arc::new(store))))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_native::{NativeEnvironment, inspect_native_status};

    #[test]
    fn mcp_startup_uses_only_the_selected_native_profile() {
        let captured = super::super::skills_startup::environment(&[
            ("XDG_CONFIG_HOME".into(), "/selected/config".into()),
            ("XDG_STATE_HOME".into(), "/unselected/state".into()),
            ("HOME".into(), "/unselected/home".into()),
        ]);
        let status = inspect_native_status(&captured);
        let directory = status.config_file_path().and_then(Path::parent);
        assert_eq!(directory, Some(Path::new("/selected/config/machine-god")));
        assert!(prepare(directory).unwrap().is_some());
        let absent = inspect_native_status(&NativeEnvironment::new(None, None, None));
        assert!(
            prepare(absent.config_file_path().and_then(Path::parent))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn mcp_startup_rejects_invalid_explicit_authority_without_fallback() {
        for directory in ["relative", "/", "/selected/../other", "/bad\0profile"] {
            assert!(prepare(Some(Path::new(directory))).is_err());
        }
    }
}
