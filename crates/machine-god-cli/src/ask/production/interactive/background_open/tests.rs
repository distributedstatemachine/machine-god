use super::*;
use std::io::Read as _;
use std::os::unix::ffi::OsStringExt as _;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
const FIXTURE_PROGRAM: &str = "#!/bin/sh\nprintf invoked > \"$0.invoked\"\n";

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..1000 {
            let path = std::env::temp_dir().join(format!(
                "mg-bg-open-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path.canonicalize().unwrap()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("fixture directory: {error}"),
            }
        }
        panic!("fixture directory allocation exhausted");
    }

    fn program(&self) -> PathBuf {
        let program = self.0.join("launcher");
        // A regression that launches during capture would leave a witness.
        std::fs::write(&program, FIXTURE_PROGRAM).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        program
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let result = std::fs::remove_dir_all(&self.0);
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

#[test]
fn platform_selection_is_fixed_without_path_or_workspace_search() {
    let expected = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        "/usr/bin/xdg-open"
    };
    assert_eq!(platform_program(), Path::new(expected));
}

#[test]
fn fixture_capture_retains_the_exact_regular_file_without_running_it() {
    let fixture = Fixture::new();
    let program = fixture.program();
    let (canonical, mut file) = resolve(&program).unwrap();
    let authority = capture_with(&program, |_| None).unwrap();
    assert_eq!(canonical, program);
    assert_eq!(
        file.metadata().unwrap().ino(),
        program.metadata().unwrap().ino()
    );
    assert_eq!(
        authority.environment,
        vec![("PATH".into(), DESKTOP_PATH.into())]
    );
    assert_eq!(
        format!("{:?}", authority.executable),
        "NativeBackgroundUrlExecutable { .. }"
    );
    assert!(!fixture.0.join("launcher.invoked").exists());
    std::fs::remove_file(&program).unwrap();
    let mut content = String::new();
    file.read_to_string(&mut content).unwrap();
    assert_eq!(content, FIXTURE_PROGRAM);
}

#[test]
fn missing_nonregular_nonexecutable_and_symlink_programs_are_optional() {
    let fixture = Fixture::new();
    let program = fixture.program();
    assert!(resolve(&fixture.0.join("missing")).is_none());
    assert!(resolve(&fixture.0).is_none());
    let link = fixture.0.join("link");
    std::os::unix::fs::symlink(&program, &link).unwrap();
    assert!(resolve(&link).is_none());
    let parent_link = fixture.0.join("parent-link");
    std::os::unix::fs::symlink(&fixture.0, &parent_link).unwrap();
    assert!(resolve(&parent_link.join("launcher")).is_none());
    let socket = fixture.0.join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    assert!(resolve(&socket).is_none());
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(resolve(&program).is_none());
}

#[test]
fn malformed_and_oversized_paths_are_rejected_before_environment_capture() {
    let fixture = Fixture::new();
    let program = fixture.program();
    for path in [
        PathBuf::from("launcher"),
        fixture.0.join(".").join("launcher"),
        fixture.0.join("..").join("launcher"),
        PathBuf::from(format!("/{}", "x".repeat(MAX_PROGRAM_BYTES))),
        PathBuf::from(OsString::from_vec(b"/invalid\0program".to_vec())),
    ] {
        assert!(capture_with(&path, |_| panic!("unexpected environment lookup")).is_none());
    }
    assert!(capture_with(&program, |_| None).is_some());
}

#[test]
fn desktop_whitelist_never_queries_credentials_browser_loader_or_path() {
    let mut queried = Vec::new();
    let mut environment = desktop_environment(|key| {
        queried.push(key.to_owned());
        Some(OsString::from(format!("desktop-{key}")))
    })
    .unwrap();
    assert_eq!(queried, DESKTOP_KEYS[..DESKTOP_KEYS.len() - 1]);
    assert_eq!(environment.len(), DESKTOP_KEYS.len());
    assert_eq!(
        environment.pop(),
        Some(("PATH".into(), DESKTOP_PATH.into()))
    );
    for excluded in [
        "PATH",
        "BROWSER",
        "OPENAI_API_KEY",
        "AI_GATEWAY_API_KEY",
        "AWS_SECRET_ACCESS_KEY",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "PYTHONPATH",
        "NODE_OPTIONS",
    ] {
        assert!(!queried.iter().any(|key| key == excluded));
        assert!(!environment.iter().any(|(key, _)| key == excluded));
    }
}

#[test]
fn environment_value_and_aggregate_bounds_are_checked_before_retention() {
    let mut calls = 0;
    assert!(
        desktop_environment(|_| {
            calls += 1;
            Some("x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES + 1).into())
        })
        .is_none()
    );
    assert_eq!(calls, 1);
    let maximum = desktop_environment(|key| {
        (key == "HOME").then(|| "x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES).into())
    })
    .unwrap();
    assert_eq!(maximum[0].1.len(), MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES);
    assert!(
        desktop_environment(|_| Some("x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES).into()))
            .is_none()
    );
}

#[test]
fn unavailable_optional_capability_keeps_native_startup_options_usable() {
    let fixture = Fixture::new();
    let authority = capture_with(&fixture.0.join("absent"), |_| None);
    assert!(authority.is_none());
    let options = NativeInteractiveSessionOptions::new(
        fixture.0.clone(),
        machine_god_native::NativeModelPreferences::default(),
    )
    .unwrap();
    assert!(
        configure(options, authority)
            .with_process_model_override("fixture/optional")
            .is_ok()
    );
    let program = fixture.program();
    let oversized = capture_with(&program, |_| {
        Some("x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES + 1).into())
    });
    assert!(oversized.is_none());
}
