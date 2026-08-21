use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use machine_god_native::{
    CONFIG_SCHEMA_VERSION, ConfigOrigin, MAX_CONFIG_BYTES, NativeConfigErrorKind,
    NativeEnvironment, PermissionMode, load_native_config,
};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mgcfg-{}-{identifier}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }

        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

fn environment(config_root: Option<&Path>, home: Option<&Path>) -> NativeEnvironment {
    NativeEnvironment::new(
        config_root.map(Path::as_os_str).map(OsString::from),
        None,
        home.map(Path::as_os_str).map(OsString::from),
    )
}

fn config_path(config_root: &Path) -> PathBuf {
    config_root.join("machine-god").join("config.json")
}

fn write_config(config_root: &Path, contents: &[u8]) -> PathBuf {
    let path = config_path(config_root);
    fs::create_dir_all(path.parent().expect("config file must have a parent")).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

fn valid_config_json() -> String {
    format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":"ask"}}"#)
}

fn load_error(environment: &NativeEnvironment) -> machine_god_native::NativeConfigError {
    match load_native_config(environment) {
        Ok(_) => panic!("configuration unexpectedly loaded successfully"),
        Err(error) => error,
    }
}

fn assert_contents_error(config_root: &Path, contents: &[u8], kind: NativeConfigErrorKind) {
    write_config(config_root, contents);
    let error = load_error(&environment(Some(config_root), None));
    assert_eq!(error.kind(), kind);
}

fn assert_diagnostics_omit(error: machine_god_native::NativeConfigError, forbidden: &[&str]) {
    let display = error.to_string();
    let debug = format!("{error:?}");

    for value in forbidden {
        assert!(
            !display.contains(value),
            "Display leaked forbidden text {value:?}: {display:?}"
        );
        assert!(
            !debug.contains(value),
            "Debug leaked forbidden text {value:?}: {debug:?}"
        );
    }
}

#[test]
fn missing_file_uses_built_in_ask_defaults_without_creating_paths() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("absent-xdg-root");
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();

    assert_eq!(loaded.origin(), ConfigOrigin::BuiltInDefaults);
    assert_eq!(loaded.config().permission_mode(), PermissionMode::Ask);
    assert!(!config_root.exists());
}

#[test]
fn unavailable_home_uses_built_in_ask_defaults() {
    let loaded = load_native_config(&NativeEnvironment::new(None, None, None)).unwrap();

    assert_eq!(loaded.origin(), ConfigOrigin::BuiltInDefaults);
    assert_eq!(loaded.config().permission_mode(), PermissionMode::Ask);
}

#[test]
fn invalid_selected_xdg_root_fails_instead_of_falling_back_to_home() {
    let temporary = TemporaryDirectory::new();
    let home = temporary.path().join("home");
    write_config(&home.join(".config"), valid_config_json().as_bytes());
    let selected_root = OsString::from("relative-secret-xdg-root");
    let environment =
        NativeEnvironment::new(Some(selected_root), None, Some(home.as_os_str().to_owned()));

    let error = load_error(&environment);
    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidEnvironment);
    assert_diagnostics_omit(error, &["relative-secret-xdg-root"]);
}

#[test]
fn valid_strict_file_is_loaded_without_modifying_it() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let contents = valid_config_json().into_bytes();
    let path = write_config(&config_root, &contents);

    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();

    assert_eq!(loaded.origin(), ConfigOrigin::File);
    assert_eq!(loaded.config().permission_mode(), PermissionMode::Ask);
    assert_eq!(fs::read(path).unwrap(), contents);
}

#[test]
fn strict_schema_rejects_unknown_duplicate_and_missing_fields() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        format!(
            r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":"ask","unexpected":true}}"#
        ),
        format!(
            r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":"ask"}}"#
        ),
        format!(
            r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":"ask","permission_mode":"ask"}}"#
        ),
        r#"{"permission_mode":"ask"}"#.to_owned(),
        format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION}}}"#),
        "{}".to_owned(),
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn strict_schema_rejects_wrong_field_types() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"schema_version":"1","permission_mode":"ask"}"#.to_owned(),
        r#"{"schema_version":true,"permission_mode":"ask"}"#.to_owned(),
        r#"{"schema_version":1.0,"permission_mode":"ask"}"#.to_owned(),
        format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":true}}"#),
        format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":1}}"#),
        format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":["ask"]}}"#),
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn unsupported_schema_version_has_a_distinct_error_kind() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let contents = format!(
        r#"{{"schema_version":{},"permission_mode":"ask"}}"#,
        CONFIG_SCHEMA_VERSION + 1
    );

    assert_contents_error(
        &config_root,
        contents.as_bytes(),
        NativeConfigErrorKind::UnsupportedSchemaVersion,
    );
}

#[test]
fn future_schema_is_classified_before_version_specific_fields() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        br#"{"schema_version":2,"permission_mode":"future","new_field":true}"#.as_slice(),
        br#"{"schema_version":18446744073709551616}"#.as_slice(),
        br#"{"schema_version":-1,"future_shape":[]}"#.as_slice(),
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents,
            NativeConfigErrorKind::UnsupportedSchemaVersion,
        );
    }
}

#[test]
fn invalid_utf8_precedes_future_version_classification() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");

    assert_contents_error(
        &config_root,
        b"{\"schema_version\":2,\"future\":\"\xff\"}",
        NativeConfigErrorKind::InvalidFormat,
    );
}

#[test]
fn unsupported_permission_mode_is_an_invalid_format() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let contents =
        format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION},"permission_mode":"allow"}}"#);

    assert_contents_error(
        &config_root,
        contents.as_bytes(),
        NativeConfigErrorKind::InvalidFormat,
    );
}

#[test]
fn malformed_trailing_and_non_utf8_input_are_rejected() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let malformed = format!(r#"{{"schema_version":{CONFIG_SCHEMA_VERSION}"#);
    let trailing = format!("{}\n{{}}", valid_config_json());
    let mut non_utf8 = valid_config_json().into_bytes();
    let ask_start = non_utf8
        .windows(3)
        .position(|window| window == b"ask")
        .expect("valid fixture contains ask");
    non_utf8[ask_start] = 0xff;

    for contents in [malformed.into_bytes(), trailing.into_bytes(), non_utf8] {
        assert_contents_error(
            &config_root,
            &contents,
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn exact_size_limit_is_accepted_and_one_additional_byte_is_rejected() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let maximum = MAX_CONFIG_BYTES;
    assert_eq!(maximum, 64 * 1024);

    let mut contents = valid_config_json().into_bytes();
    assert!(contents.len() < maximum);
    contents.resize(maximum, b' ');
    let path = write_config(&config_root, &contents);
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    assert_eq!(loaded.origin(), ConfigOrigin::File);
    assert_eq!(loaded.config().permission_mode(), PermissionMode::Ask);

    contents.push(b' ');
    fs::write(path, &contents).unwrap();
    let error = load_error(&environment(Some(&config_root), None));
    assert_eq!(error.kind(), NativeConfigErrorKind::TooLarge);
}

#[cfg(unix)]
#[test]
fn final_symlink_is_rejected_without_reading_its_target() {
    use std::os::unix::fs::symlink;

    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let target = temporary.path().join("target.json");
    fs::write(&target, valid_config_json()).unwrap();
    let path = config_path(&config_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(target, path).unwrap();

    let error = load_error(&environment(Some(&config_root), None));
    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFileType);
}

#[test]
fn directory_at_config_path_is_rejected() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    fs::create_dir_all(config_path(&config_root)).unwrap();

    let error = load_error(&environment(Some(&config_root), None));
    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFileType);
}

#[cfg(unix)]
#[test]
fn unix_socket_at_config_path_is_rejected_without_blocking() {
    use std::os::unix::net::UnixListener;

    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("x");
    let path = config_path(&config_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _listener = UnixListener::bind(path).unwrap();

    let error = load_error(&environment(Some(&config_root), None));
    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFileType);
}

#[test]
fn inaccessible_metadata_is_unreadable_and_diagnostics_hide_os_details() {
    let temporary = TemporaryDirectory::new();
    let oversized_component = format!("RAW_PATH_SECRET_{}", "x".repeat(512));
    let config_root = temporary.path().join(oversized_component);

    let error = load_error(&environment(Some(&config_root), None));

    assert_eq!(error.kind(), NativeConfigErrorKind::Unreadable);
    assert_diagnostics_omit(
        error,
        &[
            "RAW_PATH_SECRET_",
            "File name too long",
            "file name too long",
            "filename too long",
            "os error",
            "ENAMETOOLONG",
        ],
    );
}

#[test]
fn invalid_format_diagnostics_hide_path_and_content() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("PATH_SECRET_MARKER");
    let contents = br#"{"permission_mode":"CONTENT_SECRET_MARKER""#;
    write_config(&config_root, contents);

    let error = load_error(&environment(Some(&config_root), None));

    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
    assert_diagnostics_omit(error, &["PATH_SECRET_MARKER", "CONTENT_SECRET_MARKER"]);
}

#[test]
fn loading_missing_config_writes_nothing_to_an_existing_root() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("empty-xdg-root");
    fs::create_dir(&config_root).unwrap();

    let before = fs::read_dir(&config_root).unwrap().count();
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    let after = fs::read_dir(&config_root).unwrap().count();

    assert_eq!(loaded.origin(), ConfigOrigin::BuiltInDefaults);
    assert_eq!(loaded.config().permission_mode(), PermissionMode::Ask);
    assert_eq!(before, 0);
    assert_eq!(after, 0);
    assert!(!config_path(&config_root).exists());
}
