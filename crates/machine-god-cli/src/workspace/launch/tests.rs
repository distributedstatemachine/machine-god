use super::*;

fn parse(args: &[&str]) -> Result<(LaunchWorkspaceOptions, Vec<OsString>), ()> {
    let mut args = args.iter().map(OsString::from).peekable();
    let options = LaunchWorkspaceOptions::parse(&mut args)?;
    Ok((options, args.collect()))
}

#[test]
fn launch_workspace_flags_are_leading_repeatable_and_preserve_path_operands() {
    let (options, rest) = parse(&[
        "--add-dir",
        "directory with spaces",
        "--add-dir=-literal",
        "--no-additional-dirs",
        "ask",
        "--",
        "--add-dir=prompt",
    ])
    .unwrap();
    assert_eq!(
        options.directories,
        vec![PathBuf::from("directory with spaces"), "-literal".into()]
    );
    assert!(options.suppress_saved);
    assert_eq!(rest, ["ask", "--", "--add-dir=prompt"].map(OsString::from));
    assert_eq!(
        parse(&["ask", "--add-dir=not-global"]).unwrap().0,
        LaunchWorkspaceOptions::EMPTY
    );
    assert_eq!(
        parse(&["--add-dir", "--no-additional-dirs"])
            .unwrap()
            .0
            .directories,
        vec![PathBuf::from("--no-additional-dirs")]
    );
}

#[test]
fn launch_workspace_flags_fail_closed_at_raw_input_bounds() {
    for args in [
        vec!["--add-dir"],
        vec!["--add-dir="],
        vec!["--add-dir", ""],
        vec!["--no-additional-dirs", "--no-additional-dirs"],
        vec!["--add-dir=a\0b"],
    ] {
        assert!(parse(&args).is_err());
    }
    assert!(parse(&["--add-dir", &"a".repeat(4096)]).is_ok());
    assert!(parse(&["--add-dir", &"a".repeat(4097)]).is_err());
    assert!(parse(&[&format!("--add-dir={}", "a".repeat(4097))]).is_err());
    assert_eq!(
        parse(&["--add-dir=one"; 64]).unwrap().0.directories.len(),
        64
    );
    assert!(parse(&["--add-dir=one"; 65]).is_err());
}

#[cfg(unix)]
#[test]
fn launch_inline_paths_preserve_non_unicode_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let mut arguments = [OsString::from_vec(b"--add-dir=/root/\xff".to_vec())]
        .into_iter()
        .peekable();
    let options = LaunchWorkspaceOptions::parse(&mut arguments).unwrap();
    assert_eq!(
        options.directories[0].as_os_str().as_encoded_bytes(),
        b"/root/\xff"
    );
}
