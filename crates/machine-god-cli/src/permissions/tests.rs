use super::*;
use std::{cell::Cell, ffi::OsString};

struct Host {
    calls: Cell<usize>,
    fail: bool,
}
impl PermissionsCommandHost for Host {
    fn inspect(&self) -> Result<NativePermissionInspection, ()> {
        self.calls.set(self.calls.get() + 1);
        if self.fail {
            return Err(());
        }
        let loaded = machine_god_native::load_native_config(
            &machine_god_native::NativeEnvironment::new(None, None, None),
        )
        .unwrap();
        machine_god_native::inspect_native_permissions(&loaded, std::path::Path::new("/workspace"))
            .map_err(|_| ())
    }
}

#[test]
fn permissions_dispatch_calls_only_selected_host_once_after_argument_validation() {
    let host = Host {
        calls: Cell::new(0),
        fail: false,
    };
    for arguments in [
        vec!["help"],
        vec!["--version"],
        vec!["permissions", "extra"],
        vec!["permissions", "--json", "--json"],
        vec!["--json", "permissions"],
    ] {
        let mut output = Vec::new();
        let mut error = Vec::new();
        crate::run_with_hosts(
            arguments.into_iter().map(OsString::from),
            &mut output,
            &mut error,
            crate::CommandHosts {
                permissions: &host,
                ..Default::default()
            },
        );
        assert_eq!(host.calls.get(), 0);
    }
    let mut output = Vec::new();
    let mut error = Vec::new();
    assert_eq!(
        crate::run_with_hosts(
            [OsString::from("permissions"), OsString::from("--json")],
            &mut output,
            &mut error,
            crate::CommandHosts {
                permissions: &host,
                ..Default::default()
            }
        ),
        0
    );
    assert_eq!(host.calls.get(), 1);
    assert!(error.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["runtime_grants_available"], false);
    assert!(value.get("grants").is_none());
    assert_eq!(value["saved_exact_rules_available"], false);
}

#[test]
fn permissions_failures_do_not_emit_false_success_or_retry_the_host() {
    let host = Host {
        calls: Cell::new(0),
        fail: true,
    };
    let mut output = Vec::new();
    let mut error = Vec::new();
    assert_eq!(run_permissions(&host, true, &mut output, &mut error), 1);
    assert!(output.is_empty());
    assert_eq!(error, CONFIGURATION_FAILURE.as_bytes());
    let host = Host {
        calls: Cell::new(0),
        fail: false,
    };
    error.clear();
    assert_eq!(
        run_permissions(
            &host,
            false,
            &mut crate::test_support::BrokenWriter,
            &mut error
        ),
        1
    );
    assert_eq!(host.calls.get(), 1);
    assert_eq!(error, OUTPUT_FAILURE.as_bytes());
}
