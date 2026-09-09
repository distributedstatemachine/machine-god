//! Startup authority capture and fixed presentation; native owns copy selection and effects.

use machine_god_native::{
    NativeClipboardExecutable, NativeInteractiveCopyOutcome, NativeInteractiveCopyReceipt,
};
use std::ffi::OsString;
use std::fmt::Write;
use std::path::Path;

pub(super) struct Authority {
    executable: NativeClipboardExecutable,
    environment: Vec<(OsString, OsString)>,
}

pub(super) fn configure(
    options: machine_god_native::NativeInteractiveSessionOptions,
    authority: Option<Authority>,
) -> machine_god_native::NativeInteractiveSessionOptions {
    match authority {
        Some(authority) => options.with_clipboard(authority.executable, authority.environment),
        None => options,
    }
}

/// Runs on the existing blocking startup owner, never the async poll thread.
/// Resolve only the platform clipboard program, once, from a frozen host PATH.
pub(super) fn capture(workspace: &Path) -> Option<Authority> {
    let environment: Vec<_> = std::env::vars_os()
        .take(machine_god_native::MAX_TERMINAL_ENVIRONMENT_ENTRIES + 1)
        .collect();
    let executable = resolve(workspace, &environment)?;
    Some(Authority {
        executable,
        environment,
    })
}

fn resolve(
    workspace: &Path,
    environment: &[(OsString, OsString)],
) -> Option<NativeClipboardExecutable> {
    use std::os::unix::fs::PermissionsExt;
    let program = if cfg!(target_os = "macos") {
        "pbcopy"
    } else {
        "xclip"
    };
    let (_, search) = environment.iter().find(|(key, _)| key == "PATH")?;
    if search.len() > machine_god_native::MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES {
        return None;
    }
    for directory in std::env::split_paths(search).take(512) {
        let candidate = workspace.join(directory).join(program);
        if candidate.as_os_str().len() > 4096 {
            continue;
        }
        let Ok(path) = candidate.canonicalize() else {
            continue;
        };
        if !path
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        if file
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            return NativeClipboardExecutable::new(&path, file).ok();
        }
    }
    None
}

pub(super) fn render(outcome: &NativeInteractiveCopyOutcome) -> Result<Vec<u8>, ()> {
    let mut text = crate::ask::production::interactive::bounded_output();
    let message = match outcome.result {
        Ok(NativeInteractiveCopyReceipt::Empty) => "No assistant reply to copy.",
        Ok(NativeInteractiveCopyReceipt::Copied) => "Copied to clipboard.",
        Err(_) => "Failed to copy to clipboard.",
    };
    write!(text, "\n[copy {}: {message}]\n> ", outcome.id.get()).map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn capture_search_uses_only_the_named_program_and_never_executes_it() {
        let fixture = super::super::support::Fixture::new();
        let first = fixture.workspace.join("first");
        let second = fixture.workspace.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let name = if cfg!(target_os = "macos") {
            "pbcopy"
        } else {
            "xclip"
        };
        let disabled = first.join(name);
        std::fs::write(&disabled, b"not executed").unwrap();
        std::fs::set_permissions(&disabled, std::fs::Permissions::from_mode(0o600)).unwrap();
        let executable = second.join(name);
        std::fs::write(&executable, b"not executed either").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let environment = vec![(
            "PATH".into(),
            std::env::join_paths([&first, &second]).unwrap(),
        )];
        let captured = resolve(&fixture.workspace, &environment).unwrap();
        assert_eq!(format!("{captured:?}"), "NativeClipboardExecutable { .. }");
        drop(captured);
        assert_eq!(std::fs::read(&executable).unwrap(), b"not executed either");
        fixture.finish();
    }

    #[test]
    fn missing_oversize_or_nonregular_clipboard_authority_is_optional() {
        let fixture = super::super::support::Fixture::new();
        assert!(resolve(&fixture.workspace, &[]).is_none());
        assert!(
            resolve(
                &fixture.workspace,
                &[(
                    "PATH".into(),
                    "x".repeat(machine_god_native::MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES + 1)
                        .into()
                )]
            )
            .is_none()
        );
        let name = if cfg!(target_os = "macos") {
            "pbcopy"
        } else {
            "xclip"
        };
        let fifo = fixture.workspace.join(name);
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let environment = vec![("PATH".into(), fixture.workspace.clone().into_os_string())];
        assert!(resolve(&fixture.workspace, &environment).is_none());
        assert!(fixture.transport.requests().is_empty());
        fixture.finish();
    }
}
