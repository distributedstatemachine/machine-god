use super::*;
use std::fmt::Write as _;

fn detected(input: &[u8]) -> Option<String> {
    detect_server_url(input, BackgroundUrlCaptureEnd::Snapshot)
        .expect("bounded snapshot")
        .map(|url| url.as_str().to_owned())
}

#[test]
fn pinned_local_dev_server_example() {
    assert_eq!(
        detected(b"ready - started server on 0.0.0.0:3000, url: http://localhost:3000\n"),
        Some("http://localhost:3000/".to_owned())
    );
}

#[test]
fn pinned_latest_local_url_example() {
    assert_eq!(
        detected(
            b"Network: http://192.168.1.20:3000\nLocal: http://localhost:3001\nwarn  - restarting dev server\nLocal: http://localhost:3000\n"
        ),
        Some("http://localhost:3000/".to_owned())
    );
}

#[test]
fn host_ranks_and_equal_scores() {
    let ranks = [
        "http://localhost:3000/",
        "http://0.0.0.0:3000/",
        "http://192.168.1.20:3000/",
        "http://example.test:3000/",
    ];
    for (index, higher) in ranks.iter().enumerate() {
        for lower in &ranks[index + 1..] {
            // Same-line hints apply equally to both URLs.
            assert_eq!(
                detected(format!("{higher} {lower}").as_bytes()).as_deref(),
                Some(*higher)
            );
            assert_eq!(
                detected(format!("{lower} {higher}").as_bytes()).as_deref(),
                Some(*higher)
            );
        }
    }
    for host in ["127.0.0.1", "[::1]", "localhost"] {
        assert_eq!(
            detected(format!("http://{host}:1 http://{host}:2").as_bytes()),
            Some(format!("http://{host}:2/"))
        );
    }
    for host in ["10.1.2.3", "172.99.1.2", "192.168.1.2"] {
        assert_eq!(
            detected(format!("http://{host}/ http://example.test/").as_bytes()),
            Some(format!("http://{host}/"))
        );
    }
}

#[test]
fn hints_preserve_case_insensitive_pinned_weights() {
    for hint in ["LOCAL", "URL", "READY", "STARTED", "LISTENING", "SERVER"] {
        assert_eq!(
            detected(format!("{hint}: https://first.test/\nhttps://last.test/").as_bytes()),
            Some("https://first.test/".to_owned())
        );
    }
    for hint in ["NETWORK", "ERROR", "WARN"] {
        assert_eq!(
            detected(format!("https://first.test/\n{hint}: https://last.test/").as_bytes()),
            Some("https://first.test/".to_owned())
        );
    }
    assert_eq!(
        line_hint_score(b"LOCAL URL READY STARTED LISTENING SERVER NETWORK ERROR WARN"),
        -4
    );
    assert_eq!(
        detected(b"https://first.test/\nhttps://last.test/"),
        Some("https://last.test/".to_owned())
    );
}

#[test]
fn binary_and_terminal_controls_are_never_serialized() {
    assert_eq!(
        detected(b"\xff\xfe\0\x1b[32mLocal: http://localhost:3000\x1b[0m\r\n"),
        Some("http://localhost:3000/".to_owned())
    );
    for delimiter in 0..=127_u8 {
        if is_delimiter(delimiter) {
            let mut input = b"http://localhost:3000".to_vec();
            input.push(delimiter);
            input.extend_from_slice(b"ignored");
            assert_eq!(detected(&input), Some("http://localhost:3000/".to_owned()));
        }
    }
    for invalid in [
        b"http://example.test/\xff".as_slice(),
        "http://example.test/\u{0085}".as_bytes(),
        "http://example.test/\u{00a0}".as_bytes(),
    ] {
        assert_eq!(detected(invalid), None);
    }
}

#[test]
fn malformed_authorities_credentials_and_other_schemes_are_rejected() {
    for invalid in [
        "http://",
        "https:///path",
        "http://?query",
        "https://#fragment",
        "http://:3000",
        "http://[::1",
        "http://example.test:99999",
        "http://user:secret@localhost:3000",
        "https://@localhost",
        "http://localhost@remote.test",
        "https://localhost\\@remote.test",
        "http://localhost\\remote.test",
        "file:///tmp/log",
        "javascript:alert(1)",
        "ftp://localhost/",
        "http://%00.test/",
        "http://exa%20mple.test/",
    ] {
        assert_eq!(detected(invalid.as_bytes()), None, "{invalid}");
    }
    assert_eq!(
        detected(b"https:///invalid http://user:pass@localhost/ https://valid.test/"),
        Some("https://valid.test/".to_owned())
    );
}

#[test]
fn parsed_hosts_prevent_authority_path_and_query_rank_spoofing() {
    for spoof in [
        "https://localhost.remote.test/",
        "https://remote.test/localhost",
        "https://remote.test/?next=127.0.0.1",
        "https://remote.test/#0.0.0.0",
        "https://remote.test/192.168.0.1",
        "https://10.remote.test/",
        "https://172.remote.test/",
        "https://remote.test/[::1]",
        "https://localhost@remote.test/",
    ] {
        assert_eq!(
            detected(format!("http://127.0.0.1:3000 {spoof}").as_bytes()),
            Some("http://127.0.0.1:3000/".to_owned())
        );
    }
}

#[test]
fn canonical_serialization_is_the_only_exposed_form_and_debug_is_redacted() {
    let url = detect_server_url(
        b"HTTPS://LOCALHOST:443/private?token=secret",
        BackgroundUrlCaptureEnd::Snapshot,
    )
    .expect("bounded")
    .expect("URL");
    assert_eq!(url.as_str(), "https://localhost/private?token=secret");
    assert_eq!(format!("{url:?}"), "BackgroundServerUrl([REDACTED])");
    assert_eq!(url.clone(), url);
}

#[test]
fn input_bound_is_exact_and_never_truncates() {
    let mut input = vec![b' '; MAX_BACKGROUND_URL_INPUT_BYTES];
    let url = b"http://localhost:3000/";
    input[..url.len()].copy_from_slice(url);
    assert_eq!(detected(&input), Some("http://localhost:3000/".to_owned()));
    input.push(b' ');
    assert_eq!(
        detect_server_url(&input, BackgroundUrlCaptureEnd::Snapshot),
        Err(BackgroundUrlError::ResourceLimit)
    );
    assert_eq!(detected(&[]), None);
}

#[test]
fn candidate_bound_is_exact_for_raw_and_serialized_bytes() {
    let prefix = "http://localhost/";
    let mut candidate = format!(
        "{prefix}{}",
        "a".repeat(MAX_BACKGROUND_SERVER_URL_BYTES - prefix.len())
    );
    assert_eq!(
        detected(candidate.as_bytes()).as_deref(),
        Some(candidate.as_str())
    );
    candidate.push('a');
    assert_eq!(detected(candidate.as_bytes()), None);
    // Unicode path serialization expands two UTF-8 bytes into six ASCII bytes.
    let expanded = format!("{prefix}{}", "é".repeat(400));
    assert!(expanded.len() < MAX_BACKGROUND_SERVER_URL_BYTES);
    assert_eq!(detected(expanded.as_bytes()), None);
    assert_eq!(
        detected(format!("{candidate} http://valid.test/").as_bytes()),
        Some("http://valid.test/".to_owned())
    );
}

#[test]
fn bounded_flood_and_long_line_choose_latest_without_rescanning_lines() {
    let mut input = "x".repeat(32 * 1024);
    for port in 1..1000 {
        write!(input, " http://localhost:{port}").expect("string write");
    }
    assert!(input.len() <= MAX_BACKGROUND_URL_INPUT_BYTES);
    assert_eq!(
        detected(input.as_bytes()),
        Some("http://localhost:999/".to_owned())
    );
    let repeated_schemes = "http://".repeat(MAX_BACKGROUND_URL_INPUT_BYTES / 7);
    assert_eq!(detected(repeated_schemes.as_bytes()), None);
}

#[test]
fn arbitrary_byte_values_cannot_escape_validation() {
    for byte in 0..=255_u8 {
        let mut input = b"http://localhost/".to_vec();
        input.push(byte);
        if let Some(url) =
            detect_server_url(&input, BackgroundUrlCaptureEnd::Snapshot).expect("bounded")
        {
            assert!(url.as_str().len() <= MAX_BACKGROUND_SERVER_URL_BYTES);
            assert!(!url.as_str().chars().any(char::is_control));
            let parsed = Url::parse(url.as_str()).expect("validated URL");
            assert!(matches!(parsed.scheme(), "http" | "https"));
            assert!(parsed.host().is_some());
            assert!(parsed.username().is_empty());
            assert!(parsed.password().is_none());
        }
    }
}

#[test]
fn truncated_capture_requires_a_delimiter_for_its_last_candidate() {
    for suffix in [b"".as_slice(), b"/private", b"?key=partial", b"#fragment"] {
        let mut input = b"http://localhost:3000".to_vec();
        input.extend_from_slice(suffix);
        assert!(
            detect_server_url(&input, BackgroundUrlCaptureEnd::Truncated)
                .unwrap()
                .is_none()
        );
        assert!(
            detect_server_url(&input, BackgroundUrlCaptureEnd::Snapshot)
                .unwrap()
                .is_some()
        );
        for delimiter in 0..=127_u8 {
            if is_delimiter(delimiter) {
                let mut terminated = input.clone();
                terminated.push(delimiter);
                assert_eq!(
                    detect_server_url(&terminated, BackgroundUrlCaptureEnd::Truncated)
                        .unwrap()
                        .map(|url| url.as_str().to_owned()),
                    detected(&input)
                );
            }
        }
    }
    for separator in [" ", "\n"] {
        let input = format!("http://example.test/complete{separator}http://localhost:3000/partial");
        assert_eq!(
            detect_server_url(input.as_bytes(), BackgroundUrlCaptureEnd::Truncated)
                .unwrap()
                .map(|url| url.as_str().to_owned())
                .as_deref(),
            Some("http://example.test/complete")
        );
    }
}
