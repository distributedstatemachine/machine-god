use super::*;

const SESSION: &str = "terminal-0123456789abcdef0123456789abcdef";

#[test]
fn empty_payload_lists_without_an_explicit_list_alias() {
    for payload in ["", " ", "\t", " \t \t "] {
        assert_eq!(payload.parse(), Ok(NativeBackgroundCommand::List));
    }
    for payload in ["list", "show", "last", SESSION, "/background"] {
        assert_eq!(
            payload.parse::<NativeBackgroundCommand>(),
            Err(NativeBackgroundCommandError)
        );
    }
}

#[test]
fn all_actions_accept_implicit_last_explicit_last_and_exact_session() {
    for (verb, constructor) in [
        ("stop", NativeBackgroundCommand::Stop as fn(_) -> _),
        ("open", NativeBackgroundCommand::Open),
        ("logs", NativeBackgroundCommand::Logs),
    ] {
        for target in ["", "last", SESSION] {
            let expected = if target == SESSION {
                NativeBackgroundTarget::Session(TerminalSessionId::new(SESSION).unwrap())
            } else {
                NativeBackgroundTarget::Last
            };
            for separator in [" ", "\t", " \t  \t "] {
                for (prefix, suffix) in [("", ""), (" \t", "\t ")] {
                    let payload = format!("{prefix}{verb}{separator}{target}{suffix}");
                    assert_eq!(payload.parse(), Ok(constructor(expected.clone())));
                }
            }
        }
        assert_eq!(verb.parse(), Ok(constructor(NativeBackgroundTarget::Last)));
    }
}

#[test]
fn generated_terminal_ids_require_exact_prefix_width_and_lowercase_hex() {
    for suffix in ["0".repeat(32), "f".repeat(32), "0123456789abcdef".repeat(2)] {
        let value = format!("terminal-{suffix}");
        assert_eq!(
            format!("logs {value}").parse(),
            Ok(NativeBackgroundCommand::Logs(
                NativeBackgroundTarget::Session(TerminalSessionId::new(value).unwrap())
            ))
        );
    }
    for value in [
        "terminal-".to_owned(),
        format!("terminal-{}", "0".repeat(31)),
        format!("terminal-{}", "0".repeat(33)),
        format!("terminal-{}", "F".repeat(32)),
        format!("terminal-{}", "g".repeat(32)),
        format!("Terminal-{}", "0".repeat(32)),
        format!("terminal_{}", "0".repeat(32)),
        "0".repeat(32),
        format!("{SESSION}.log"),
        format!("/{SESSION}"),
        format!("../{SESSION}"),
        format!("{SESSION}/"),
        "terminal-example".to_owned(),
    ] {
        for verb in ["stop", "open", "logs"] {
            assert!(
                format!("{verb} {value}")
                    .parse::<NativeBackgroundCommand>()
                    .is_err()
            );
        }
    }
}

#[test]
fn malformed_tokens_flags_aliases_and_legacy_numeric_ids_are_rejected() {
    for payload in [
        "Stop",
        "OPEN",
        "Logs",
        "stop LAST",
        "stop Last",
        "stop last extra",
        "logs last last",
        "open stop",
        "logs --json",
        "stop --force",
        "--json",
        "stop -- last",
        "logs last --json",
        "open https://localhost:3000",
        "logs /tmp/background.log",
        "logs 0",
        "stop 42",
        "open 00042",
        "stop 18446744073709551615",
        "stop -1",
        "stop +1",
        "logs #1",
        "logs 'last'",
        "logs \"last\"",
        "/background stop last",
    ] {
        assert_eq!(
            payload.parse::<NativeBackgroundCommand>(),
            Err(NativeBackgroundCommandError),
            "accepted {payload:?}"
        );
    }
    assert!(
        format!("logs {SESSION} extra")
            .parse::<NativeBackgroundCommand>()
            .is_err()
    );
}

#[test]
fn controls_and_non_ascii_characters_are_never_separators_or_targets() {
    for control in (0_u8..=31).chain(std::iter::once(127)) {
        if control == b'\t' {
            continue;
        }
        let control = char::from(control);
        for payload in [
            control.to_string(),
            format!("{control}logs last"),
            format!("logs{control}last"),
            format!("logs last{control}"),
        ] {
            assert!(payload.parse::<NativeBackgroundCommand>().is_err());
        }
    }
    for value in [
        '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2007}', '\u{200b}', '\u{2028}', '\u{2029}',
        '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}', 'é', '🦀',
    ] {
        for payload in [
            value.to_string(),
            format!("{value}logs last"),
            format!("logs{value}last"),
            format!("logs last{value}"),
        ] {
            assert!(payload.parse::<NativeBackgroundCommand>().is_err());
        }
    }
}

#[test]
fn utf8_byte_limit_is_inclusive_and_counts_padding() {
    assert_eq!(MAX_NATIVE_BACKGROUND_COMMAND_BYTES, 256);
    for payload in [String::new(), "logs".to_owned(), format!("stop {SESSION}")] {
        let expected = payload.parse::<NativeBackgroundCommand>().unwrap();
        let padding = " ".repeat(MAX_NATIVE_BACKGROUND_COMMAND_BYTES - payload.len());
        let bounded = format!("{payload}{padding}");
        assert_eq!(bounded.len(), MAX_NATIVE_BACKGROUND_COMMAND_BYTES);
        assert_eq!(bounded.parse(), Ok(expected));
        assert_eq!(
            format!("{bounded} ").parse::<NativeBackgroundCommand>(),
            Err(NativeBackgroundCommandError)
        );
    }
    for payload in ["é".repeat(129), " ".repeat(1_000_000)] {
        assert!(payload.parse::<NativeBackgroundCommand>().is_err());
    }
}

#[test]
fn invalid_and_default_targets_allocate_nothing() {
    let oversized = " ".repeat(1_000_000);
    let extra = format!("stop {SESSION} extra");
    for payload in [
        "",
        " \t",
        "logs",
        "stop last",
        "open --json",
        &extra,
        &oversized,
    ] {
        let allocation = allocation_counter::measure(|| {
            let _ = std::hint::black_box(payload.parse::<NativeBackgroundCommand>());
        });
        assert_eq!(allocation.count_total, 0);
    }
}

#[test]
fn debug_and_error_diagnostics_do_not_expose_target_or_payload() {
    let target = NativeBackgroundTarget::Session(TerminalSessionId::new(SESSION).unwrap());
    assert_eq!(target, target.clone());
    assert_eq!(format!("{target:?}"), "NativeBackgroundTarget { .. }");
    for command in [
        NativeBackgroundCommand::List,
        NativeBackgroundCommand::Stop(target.clone()),
        NativeBackgroundCommand::Open(target.clone()),
        NativeBackgroundCommand::Logs(target),
    ] {
        assert_eq!(command, command.clone());
        for debug in [format!("{command:?}"), format!("{command:#?}")] {
            assert!(!debug.contains(SESSION));
            assert!(!debug.contains("0123456789abcdef"));
        }
    }
    let error = "logs /private/secret"
        .parse::<NativeBackgroundCommand>()
        .unwrap_err();
    assert_eq!(format!("{error:?}"), "NativeBackgroundCommandError");
    assert_eq!(error.to_string(), "invalid native background command");
    assert!(std::error::Error::source(&error).is_none());
}
